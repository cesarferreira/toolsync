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
pub fn run(paths: &Paths, jobs: Option<usize>) -> Result<bool> {
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
            &ui::err_dim(format!("Syncing {total} tool(s), {jobs} at a time")).to_string(),
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
                    let result = sync_tool(paths, &name, &tool, &mut entry, &row);
                    match &result {
                        Ok(outcome) => row.success(&outcome.render()),
                        Err(err) => row.failure(err.summary(), &err.message),
                    }
                    if let Err(err) = &result
                        && !row.is_live()
                    {
                        print_install_tail(paths, &name, err);
                    }
                    entry.error = result.as_ref().err().cloned();
                    if tx.send((name, entry, result.err())).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        // Only this thread touches the state file.
        for (name, entry, err) in rx {
            state.tools.insert(name.clone(), entry);
            state.save(paths)?;
            if let Some(err) = err {
                failures.push((name, err));
            }
        }
        Ok(())
    })?;

    if board.is_live() {
        print_summary(&board, paths, total, &mut failures);
    } else if !failures.is_empty() {
        let mut names: Vec<&str> = failures.iter().map(|(n, _)| n.as_str()).collect();
        names.sort();
        eprintln!("{} of {total} failed: {}", names.len(), names.join(", "));
    }
    Ok(failures.is_empty())
}

/// After the board finishes, explain each failure and give a one-line total.
fn print_summary(board: &Board, paths: &Paths, total: usize, failures: &mut [(String, ToolError)]) {
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
    let ok = total - failures.len();
    let mut line = ui::good(format!("✔ {ok} ok")).to_string();
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
    PullOnly(String),
    UpToDate(String),
    Installed(String),
}

impl Outcome {
    fn render(&self) -> String {
        match self {
            Outcome::PullOnly(sha) => format!(
                "{} {}",
                ui::good(format!("synced {}", short(sha))),
                ui::dim("(pull only)")
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

    let dir = paths.checkout(name);
    row.stage(if dir.exists() {
        "checking managed checkout"
    } else {
        "cloning"
    });
    prepare_checkout(&dir, &tool.url).map_err(|e| sync(format!("{e:#}")))?;

    match tool.pull.as_deref() {
        None => {
            row.stage("pulling main");
            // Built-in sync fetches again; use that newer observation of main.
            desired = builtin_sync(&dir).map_err(|e| sync(format!("{e:#}")))?;
            entry.remote_main = Some(desired.clone());
        }
        Some(cmd) => {
            row.stage(format!("running {}", ui::cmd(cmd)));
            run_sync_command(&dir, cmd).map_err(|e| sync(format!("{e:#}")))?
        }
    }

    row.stage("verifying checkout");
    let head = git::head(&dir).map_err(|e| sync(format!("{e:#}")))?;
    entry.checkout = Some(head.clone());
    verify_checkout(&dir, &head, &desired).map_err(|e| sync(format!("{e:#}")))?;

    let Some(install_cmd) = tool.install.as_deref() else {
        return Ok(Outcome::PullOnly(head));
    };
    if entry.installed.as_deref() == Some(head.as_str()) {
        return Ok(Outcome::UpToDate(head));
    }

    row.stage(format!(
        "{} {}",
        ui::warn("installing"),
        ui::dim(short(&head))
    ));
    install(paths, name, &dir, install_cmd, row)
        .map_err(|e| ToolError::new(ErrorKind::Install, format!("{e:#}")))?;
    entry.installed = Some(head.clone());
    entry.installed_at = Some(now());
    Ok(Outcome::Installed(head))
}

/// Clone the managed checkout on first use, and make sure an existing one follows `url`.
fn prepare_checkout(dir: &Path, url: &str) -> Result<()> {
    if !dir.exists() {
        if let Some(parent) = dir.parent() {
            fs::create_dir_all(parent)?;
        }
        let dir_str = dir.to_string_lossy();
        git::run(
            None,
            &["clone", "--quiet", "--branch", "main", url, &dir_str],
        )
        .with_context(|| format!("cloning {url}"))?;
        return Ok(());
    }
    let origin = git::run(Some(dir), &["remote", "get-url", "origin"])
        .with_context(|| format!("{} is not a usable Git checkout", dir.display()))?;
    if origin != url {
        bail!(
            "managed checkout {} follows {origin}, but the tool list says {url}; delete it to re-clone",
            dir.display()
        );
    }
    ensure_clean(dir)
}

fn ensure_clean(dir: &Path) -> Result<()> {
    if git::is_dirty(dir)? {
        bail!(
            "managed checkout {} has uncommitted changes; toolsync won't overwrite them",
            dir.display()
        );
    }
    Ok(())
}

/// Fast-forward `main` to `origin/main`. Returns the new HEAD.
fn builtin_sync(dir: &Path) -> Result<String> {
    git::run(Some(dir), &["fetch", "--quiet", "origin", "main"])?;
    let target = git::run(Some(dir), &["rev-parse", "refs/remotes/origin/main"])?;

    let branch = git::current_branch(dir)?;
    if branch.as_deref() != Some("main") {
        bail!(
            "managed checkout is on {}, expected main",
            branch.as_deref().unwrap_or("a detached HEAD")
        );
    }

    let head = git::head(dir)?;
    if head == target {
        return Ok(target);
    }
    if !git::succeeds(Some(dir), &["merge-base", "--is-ancestor", "HEAD", &target])? {
        bail!(
            "managed checkout {} has diverged from origin/main; fix or delete it",
            dir.display()
        );
    }
    git::run(Some(dir), &["merge", "--ff-only", "--quiet", &target])?;
    Ok(target)
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

/// A sync only counts when the checkout is clean, on main, and at the expected commit.
fn verify_checkout(dir: &Path, head: &str, desired: &str) -> Result<()> {
    let branch = git::current_branch(dir)?;
    if branch.as_deref() != Some("main") {
        bail!(
            "checkout is on {} after sync, expected main",
            branch.as_deref().unwrap_or("a detached HEAD")
        );
    }
    if head != desired {
        bail!(
            "checkout is at {} after sync, expected main at {}",
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
