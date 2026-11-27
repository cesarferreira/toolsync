use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::git;
use crate::paths::Paths;
use crate::progress::{Board, Row};
use crate::state::{ErrorKind, State, ToolError, ToolState, now};
use crate::tools::{Tool, ToolList};
use crate::ui;

/// Returns `false` when any tool failed; the failures are already reported.
/// With `install` false, tools are only pulled.
pub fn run(paths: &Paths, jobs: Option<usize>, install: bool) -> Result<bool> {
    let _lock = acquire_lock(paths)?;

    let list = ToolList::load(paths)?;
    let mut state = State::load(paths)?;

    if list.tools.is_empty() {
        println!(
            "No tools registered. Run {} inside a repo.",
            ui::cmd("tsync add")
        );
        return Ok(true);
    }

    let total = list.tools.len();
    let jobs = jobs.unwrap_or(total).clamp(1, total);
    let (board, rows) = Board::new(list.tools.keys());
    if board.is_live() {
        board.println(
            &ui::err_dim(format!(
                "{} {total} tool(s), {jobs} at a time",
                if install { "Syncing" } else { "Pulling" }
            ))
            .to_string(),
        );
    }

    let queue: Mutex<Vec<_>> = Mutex::new(
        list.tools
            .iter()
            .zip(rows)
            .map(|((name, tool), row)| {
                let entry = state.tools.get(name).cloned().unwrap_or_default();
                (name.clone(), tool.clone(), entry, row)
            })
            .rev()
            .collect(),
    );

    let (tx, rx) = mpsc::channel();
    let mut failures: Vec<(String, ToolError)> = Vec::new();
    let mut skipped = 0;
    thread::scope(|scope| -> Result<()> {
        for _ in 0..jobs {
            let tx = tx.clone();
            let queue = &queue;
            scope.spawn(move || {
                loop {
                    let next = queue.lock().expect("queue lock").pop();
                    let Some((name, tool, mut entry, row)) = next else {
                        break;
                    };
                    entry.last_run_at = Some(now());
                    let result = sync_tool(paths, &name, &tool, &mut entry, &row, install);
                    let skipped = matches!(result, Ok(Outcome::Skipped(_)));
                    match &result {
                        Ok(Outcome::Skipped(reason)) => row.skipped(reason),
                        Ok(outcome) => row.success(&outcome.render()),
                        Err(err) => row.failure(err.summary(), &err.message),
                    }
                    entry.skipped = match &result {
                        Ok(Outcome::Skipped(reason)) => Some(reason.clone()),
                        _ => None,
                    };
                    if let Err(err) = &result
                        && !row.is_live()
                    {
                        print_install_tail(paths, &name, err);
                    }
                    entry.error = result.as_ref().err().cloned();
                    if tx.send((name, entry, result.err(), skipped)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        // Only this thread touches the state file.
        for (name, entry, err, was_skipped) in rx {
            state.tools.insert(name.clone(), entry);
            state.save(paths)?;
            skipped += usize::from(was_skipped);
            if let Some(err) = err {
                failures.push((name, err));
            }
        }
        Ok(())
    })?;

    if board.is_live() {
        print_summary(&board, paths, total, skipped, &mut failures);
    } else if !failures.is_empty() {
        let mut names: Vec<&str> = failures.iter().map(|(n, _)| n.as_str()).collect();
        names.sort();
        eprintln!("{} of {total} failed: {}", names.len(), names.join(", "));
    }
    Ok(failures.is_empty())
}

/// After the board finishes, explain each failure and give a one-line total.
fn print_summary(
    board: &Board,
    paths: &Paths,
    total: usize,
    skipped: usize,
    failures: &mut [(String, ToolError)],
) {
    failures.sort_by(|a, b| a.0.cmp(&b.0));
    if !failures.is_empty() {
        eprintln!();
        for (name, err) in failures.iter() {
            eprintln!(
                "{} {} {}",
                ui::err_name(format!("{name:<w$}", w = board.width())),
                ui::err_bad(format!("{}:", err.summary())),
                ui::err_dim(&err.message)
            );
            print_install_tail(paths, name, err);
        }
    }
    let ok = total - failures.len() - skipped;
    let mut line = ui::good(format!("✔ {ok} ok")).to_string();
    if skipped > 0 {
        line.push_str(&format!("  {}", ui::warn(format!("● {skipped} skipped"))));
    }
    if !failures.is_empty() {
        line.push_str(&format!(
            "  {}",
            ui::err_bad(format!("✖ {} failed", failures.len()))
        ));
    }
    eprintln!();
    eprintln!("{line}");
}

fn print_install_tail(paths: &Paths, name: &str, err: &ToolError) {
    if err.kind != ErrorKind::Install {
        return;
    }
    for line in tail_lines(&paths.install_log(name), 15) {
        eprintln!("  {}", ui::err_dim(format!("| {line}")));
    }
}

/// What a successful sync did.
enum Outcome {
    /// The repo is on another branch or has local changes; nothing was touched.
    Skipped(String),
    PullOnly(String),
    /// Pulled, but `--pull` asked not to install this run.
    InstallSkipped(String),
    UpToDate(String),
    Installed(String),
}

impl Outcome {
    fn render(&self) -> String {
        match self {
            Outcome::Skipped(reason) => format!("skipped: {reason}"),
            Outcome::PullOnly(sha) => format!(
                "{} {}",
                ui::good(format!("synced {}", short(sha))),
                ui::dim("(pull only)")
            ),
            Outcome::InstallSkipped(sha) => format!(
                "{} {}",
                ui::good(format!("pulled {}", short(sha))),
                ui::dim("(install skipped)")
            ),
            Outcome::UpToDate(sha) => format!(
                "{} {}",
                ui::good("up to date"),
                ui::dim(format!("({})", short(sha)))
            ),
            Outcome::Installed(sha) => ui::good(format!("installed {}", short(sha))).to_string(),
        }
    }
}

/// Holds an exclusive lock on the sync lock file until dropped.
pub struct SyncLock(#[allow(dead_code)] File);

pub fn acquire_lock(paths: &Paths) -> Result<SyncLock> {
    fs::create_dir_all(paths.root())
        .with_context(|| format!("creating {}", paths.root().display()))?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.lock_file())
        .with_context(|| format!("opening {}", paths.lock_file().display()))?;
    match file.try_lock() {
        Ok(()) => Ok(SyncLock(file)),
        Err(fs::TryLockError::WouldBlock) => {
            bail!("another `tsync sync` is already running on this machine")
        }
        Err(fs::TryLockError::Error(err)) => Err(err).context("locking sync.lock"),
    }
}

fn sync_tool(
    paths: &Paths,
    name: &str,
    tool: &Tool,
    entry: &mut ToolState,
    row: &Row,
    install_allowed: bool,
) -> Result<Outcome, ToolError> {
    let refresh = |msg: String| ToolError::new(ErrorKind::Refresh, msg);
    let sync = |msg: String| ToolError::new(ErrorKind::Sync, msg);

    row.stage("checking remote main");
    let mut desired = match git::remote_main(None, &tool.url) {
        Ok(Some(sha)) => sha,
        Ok(None) => return Err(refresh(format!("{} has no `main` branch", tool.url))),
        Err(err) => return Err(refresh(format!("{err:#}"))),
    };
    entry.remote_main = Some(desired.clone());
    entry.remote_checked_at = Some(now());

    let Some(dir) = tool.path.as_deref() else {
        return Err(sync(
            "no local repo recorded; run `tsync add` inside it".into(),
        ));
    };
    row.stage("checking local repo");
    let remote = local_remote(dir, &tool.url).map_err(|e| sync(format!("{e:#}")))?;
    if let Some(reason) = skip_reason(dir).map_err(|e| sync(format!("{e:#}")))? {
        return Ok(Outcome::Skipped(reason));
    }

    match tool.pull.as_deref() {
        None => {
            row.stage("pulling main");
            // Built-in sync fetches again; use that newer observation of main.
            match builtin_sync(dir, &remote).map_err(|e| sync(format!("{e:#}")))? {
                Some(target) => desired = target,
                None => {
                    return Ok(Outcome::Skipped(format!(
                        "main has commits that aren't on {remote}"
                    )));
                }
            }
            entry.remote_main = Some(desired.clone());
        }
        Some(cmd) => {
            row.stage(format!("running {}", ui::cmd(cmd)));
            run_sync_command(dir, cmd).map_err(|e| sync(format!("{e:#}")))?
        }
    }

    row.stage("verifying repo");
    let head = git::head(dir).map_err(|e| sync(format!("{e:#}")))?;
    entry.checkout = Some(head.clone());
    verify_checkout(dir, &head, &desired).map_err(|e| sync(format!("{e:#}")))?;

    let Some(install_cmd) = tool.install.as_deref() else {
        return Ok(Outcome::PullOnly(head));
    };
    if entry.installed.as_deref() == Some(head.as_str()) {
        return Ok(Outcome::UpToDate(head));
    }
    if !install_allowed {
        return Ok(Outcome::InstallSkipped(head));
    }

    row.stage(format!(
        "{} {}",
        ui::warn("installing"),
        ui::dim(short(&head))
    ));
    install(paths, name, dir, install_cmd, row)
        .map_err(|e| ToolError::new(ErrorKind::Install, format!("{e:#}")))?;
    entry.installed = Some(head.clone());
    entry.installed_at = Some(now());
    Ok(Outcome::Installed(head))
}

/// The remote in the local repo that points at the tool's URL.
fn local_remote(dir: &Path, url: &str) -> Result<String> {
    if !dir.join(".git").exists() {
        bail!(
            "no Git repo at {}; run `tsync add` from its new location",
            dir.display()
        );
    }
    git::remote_for_url(dir, url)?
        .with_context(|| format!("no remote in {} points to {url}", dir.display()))
}

/// Sync only touches a repo that is on `main` with no uncommitted changes.
fn skip_reason(dir: &Path) -> Result<Option<String>> {
    match git::current_branch(dir)?.as_deref() {
        Some("main") => {}
        Some(branch) => return Ok(Some(format!("on branch {branch}"))),
        None => return Ok(Some("detached HEAD".into())),
    }
    if git::is_dirty(dir)? {
        return Ok(Some("uncommitted changes".into()));
    }
    Ok(None)
}

fn ensure_clean(dir: &Path) -> Result<()> {
    if git::is_dirty(dir)? {
        bail!("{} has uncommitted changes after sync", dir.display());
    }
    Ok(())
}

/// Fast-forward `main` to `<remote>/main`. Returns the new HEAD, or `None` when
/// local `main` is ahead (unpushed commits), which is left alone.
fn builtin_sync(dir: &Path, remote: &str) -> Result<Option<String>> {
    git::run(Some(dir), &["fetch", "--quiet", remote, "main"])?;
    let target = git::run(
        Some(dir),
        &["rev-parse", &format!("refs/remotes/{remote}/main")],
    )?;

    let head = git::head(dir)?;
    if head == target {
        return Ok(Some(target));
    }
    if git::succeeds(Some(dir), &["merge-base", "--is-ancestor", "HEAD", &target])? {
        git::run(Some(dir), &["merge", "--ff-only", "--quiet", &target])?;
        return Ok(Some(target));
    }
    if git::succeeds(Some(dir), &["merge-base", "--is-ancestor", &target, "HEAD"])? {
        return Ok(None);
    }
    bail!("local main has diverged from {remote}/main; reconcile it by hand")
}

fn shell(dir: &Path, cmd: &str) -> Command {
    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    command.current_dir(dir).stdin(Stdio::null());
    command
}

/// Output is captured so it can't tear the progress display; it is shown on failure.
fn run_sync_command(dir: &Path, cmd: &str) -> Result<()> {
    let out = shell(dir, cmd)
        .output()
        .with_context(|| format!("running pull command `{cmd}`"))?;
    if !out.status.success() {
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(5)..].join("\n");
        let tail = if tail.is_empty() {
            String::new()
        } else {
            format!("\n{tail}")
        };
        bail!("pull command `{cmd}` {}{tail}", describe_status(out.status));
    }
    Ok(())
}

/// A sync only counts when the repo is clean, on main, and at the expected commit.
fn verify_checkout(dir: &Path, head: &str, desired: &str) -> Result<()> {
    let branch = git::current_branch(dir)?;
    if branch.as_deref() != Some("main") {
        bail!(
            "repo is on {} after sync, expected main",
            branch.as_deref().unwrap_or("a detached HEAD")
        );
    }
    if head != desired {
        bail!(
            "repo is at {} after sync, expected main at {}",
            short(head),
            short(desired)
        );
    }
    ensure_clean(dir)
}

fn install(paths: &Paths, name: &str, dir: &Path, cmd: &str, row: &Row) -> Result<()> {
    let log_path = paths.install_log(name);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let log =
        File::create(&log_path).with_context(|| format!("creating {}", log_path.display()))?;
    let mut child = shell(dir, cmd)
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
        .with_context(|| format!("running `{cmd}`"))?;

    // Mirror the latest log line on the tool's row while the command runs.
    let mut shown = String::new();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if let Some(line) = tail_lines(&log_path, 1).pop()
            && line != shown
        {
            row.activity("installing", &line);
            shown = line;
        }
        thread::sleep(Duration::from_millis(100));
    };
    if status.success() {
        return Ok(());
    }
    bail!(
        "install command {}; log: {}",
        describe_status(status),
        log_path.display()
    )
}

fn describe_status(status: std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exited with code {code}"),
        None => "was killed by a signal".into(),
    }
}

/// Last `n` non-empty lines of a file, reading at most its final 16 KiB.
fn tail_lines(path: &Path, n: usize) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(len.saturating_sub(16 * 1024)));
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    // Cargo redraws progress lines with carriage returns.
    let lines: Vec<String> = text
        .lines()
        .filter_map(|l| l.rsplit('\r').find(|p| !p.trim().is_empty()))
        .map(|l| l.trim_end().to_string())
        .collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

pub fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}
