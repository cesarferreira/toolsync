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
use crate::project::IconStyle;
use crate::state::{ErrorKind, State, ToolError, ToolState, now};
use crate::tools::{Tool, ToolList};
use crate::ui;

/// Returns `false` when any tool failed; the failures are already reported.
/// With `install` false, tools are only pulled.
pub fn run(paths: &Paths, jobs: Option<usize>, install: bool, icons: IconStyle) -> Result<bool> {
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
    let (board, rows) = Board::new(&list.tools, icons);
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
    let mut summary = Summary::default();
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
                    let mut change = CheckoutChange::Unchanged;
                    let result =
                        sync_tool(paths, &name, &tool, &mut entry, &row, install, &mut change);
                    match &result {
                        Ok(outcome) => outcome.show(&row, change),
                        Err(err) => {
                            let error = ui::err_bad(err.summary());
                            let outcome = if change == CheckoutChange::Unchanged {
                                error.to_string()
                            } else {
                                format!("{} · {error}", change.label())
                            };
                            row.failure(&outcome, &err.message);
                        }
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
                    if tx.send((name, entry, change, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        // Only this thread touches the state file.
        for (name, entry, change, result) in rx {
            state.tools.insert(name.clone(), entry);
            state.save(paths)?;
            summary.record(change, &result);
            if let Err(err) = result {
                failures.push((name, err));
            }
        }
        Ok(())
    })?;

    if board.is_live() {
        print_failures(&board, paths, &mut failures);
    }
    eprintln!("\n{}", summary.render());
    Ok(failures.is_empty())
}

fn print_failures(board: &Board, paths: &Paths, failures: &mut [(String, ToolError)]) {
    if failures.is_empty() {
        return;
    }
    failures.sort_by(|a, b| a.0.cmp(&b.0));
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

fn print_install_tail(paths: &Paths, name: &str, err: &ToolError) {
    if err.kind != ErrorKind::Install {
        return;
    }
    for line in tail_lines(&paths.install_log(name), 15) {
        eprintln!("  {}", ui::err_dim(format!("| {line}")));
    }
}

#[derive(Default)]
struct Summary {
    pulled: usize,
    commits: usize,
    changed: usize,
    installed: usize,
    up_to_date: usize,
    skipped: usize,
    install_failed: usize,
    sync_failed: usize,
}

impl Summary {
    fn record(&mut self, change: CheckoutChange, result: &Result<Outcome, ToolError>) {
        match change {
            CheckoutChange::Pulled(count) => {
                self.pulled += 1;
                self.commits += count;
            }
            CheckoutChange::Changed => self.changed += 1,
            CheckoutChange::Unchanged => {}
        }
        match result {
            Ok(Outcome::Installed(_)) => self.installed += 1,
            Ok(Outcome::Skipped(_)) => self.skipped += 1,
            Ok(_) if change == CheckoutChange::Unchanged => self.up_to_date += 1,
            Err(err) if err.kind == ErrorKind::Install => self.install_failed += 1,
            Err(_) => self.sync_failed += 1,
            Ok(_) => {}
        }
    }

    fn render(&self) -> String {
        let mut parts = vec![
            ui::good(format!(
                "{} pulled ({})",
                self.pulled,
                commits(self.commits)
            ))
            .for_stderr()
            .to_string(),
        ];
        if self.changed > 0 {
            let noun = if self.changed == 1 {
                "change"
            } else {
                "changes"
            };
            parts.push(format!("{} checkout {noun}", self.changed));
        }
        if self.installed > 0 {
            parts.push(format!("{} installed", self.installed));
        }
        parts.push(ui::err_dim(format!("{} up to date", self.up_to_date)).to_string());
        if self.skipped > 0 {
            parts.push(
                ui::warn(format!("{} skipped", self.skipped))
                    .for_stderr()
                    .to_string(),
            );
        }
        if self.install_failed > 0 {
            parts.push(ui::err_bad(format!("{} install failed", self.install_failed)).to_string());
        }
        if self.sync_failed > 0 {
            parts.push(ui::err_bad(format!("{} sync failed", self.sync_failed)).to_string());
        }
        parts.join(" · ")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckoutChange {
    Unchanged,
    Pulled(usize),
    Changed,
}

impl CheckoutChange {
    fn between(dir: &Path, before: Option<&str>, after: &str) -> Self {
        let Some(before) = before else {
            return Self::Changed;
        };
        if before == after {
            return Self::Unchanged;
        }

        // Count the commits only when the old HEAD is part of the new history.
        // A custom command can replace history, and counting is only for display.
        let range = format!("{before}...{after}");
        let count = git::run(Some(dir), &["rev-list", "--left-right", "--count", &range])
            .ok()
            .and_then(|counts| counts.strip_prefix("0\t")?.parse::<usize>().ok());
        match count {
            Some(count) if count > 0 => Self::Pulled(count),
            _ => Self::Changed,
        }
    }

    fn label(self) -> String {
        match self {
            Self::Unchanged => "up to date".into(),
            Self::Pulled(count) => format!("pulled +{}", commits(count)),
            Self::Changed => "checkout changed".into(),
        }
    }
}

fn commits(count: usize) -> String {
    let noun = if count == 1 { "commit" } else { "commits" };
    format!("{count} {noun}")
}

enum Outcome {
    /// The repo is on another branch or has local changes; nothing was touched.
    Skipped(String),
    PullOnly(String),
    /// The checkout is current and `--pull` deferred a pending install.
    InstallSkipped(String),
    UpToDate(String),
    Installed(String),
}

impl Outcome {
    fn show(&self, row: &Row, change: CheckoutChange) {
        let (head, install) = match self {
            Self::Skipped(reason) => return row.skipped(reason),
            Self::PullOnly(head) => (head, " · pull only"),
            Self::InstallSkipped(head) => (head, " · install skipped"),
            Self::UpToDate(head) => (head, ""),
            Self::Installed(head) => (head, " · installed"),
        };
        let sha = ui::dim(format!("({})", short(head)));
        let label = change.label();
        match change {
            CheckoutChange::Unchanged if matches!(self, Self::Installed(_)) => row.success(
                ui::good("✔"),
                &format!("{} {sha}", ui::good("installed existing checkout")),
            ),
            CheckoutChange::Unchanged => {
                row.success(ui::dim("·"), &format!("{}{install} {sha}", ui::dim(label)));
            }
            _ => {
                let mark = if change == CheckoutChange::Changed {
                    "↔"
                } else {
                    "↑"
                };
                row.success(
                    ui::good(mark),
                    &format!("{}{install} {sha}", ui::good(label)),
                );
            }
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
    change: &mut CheckoutChange,
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
    let repo = git::repo_state(dir).map_err(|e| sync(format!("{e:#}")))?;
    if let Some(reason) = skip_reason(&repo) {
        return Ok(Outcome::Skipped(reason));
    }

    // Remote main was just read; if the repo already has it, there is nothing to
    // pull. The repo is then on a clean main at `desired`, so it needs no second check.
    let head = match tool.pull.as_deref() {
        _ if repo.head.as_deref() == Some(desired.as_str()) => desired.clone(),
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
            verify_after_pull(dir, entry, &desired, row).map_err(|e| sync(format!("{e:#}")))?
        }
        Some(cmd) => {
            // Local main already holds remote main plus unpushed commits; a pull
            // can't land on remote main, so leave it alone like the built-in sync.
            if git::succeeds(
                Some(dir),
                &["merge-base", "--is-ancestor", &desired, "HEAD"],
            )
            .map_err(|e| sync(format!("{e:#}")))?
            {
                return Ok(Outcome::Skipped(format!(
                    "main has commits that aren't on {remote}"
                )));
            }
            row.stage(format!("running {}", ui::cmd(cmd)));
            run_sync_command(dir, cmd).map_err(|e| sync(format!("{e:#}")))?;
            verify_after_pull(dir, entry, &desired, row).map_err(|e| sync(format!("{e:#}")))?
        }
    };
    // Keep the pull result even if the subsequent install fails.
    *change = CheckoutChange::between(dir, repo.head.as_deref(), &head);
    entry.checkout = Some(head.clone());

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
pub(crate) fn skip_reason(repo: &git::RepoState) -> Option<String> {
    match repo.branch.as_deref() {
        Some("main") => {}
        Some(branch) => return Some(format!("on branch {branch}")),
        None => return Some("detached HEAD".into()),
    }
    repo.dirty.then(|| "uncommitted changes".into())
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
/// Returns the new HEAD.
fn verify_after_pull(
    dir: &Path,
    entry: &mut ToolState,
    desired: &str,
    row: &Row,
) -> Result<String> {
    row.stage("verifying repo");
    let repo = git::repo_state(dir)?;
    let head = repo.head.context("repo has no commits after sync")?;
    entry.checkout = Some(head.clone());
    if repo.branch.as_deref() != Some("main") {
        bail!(
            "repo is on {} after sync, expected main",
            repo.branch.as_deref().unwrap_or("a detached HEAD")
        );
    }
    if head != desired {
        bail!(
            "repo is at {} after sync, expected main at {}",
            short(&head),
            short(desired)
        );
    }
    if repo.dirty {
        bail!("{} has uncommitted changes after sync", dir.display());
    }
    Ok(head)
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
