use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::git;
use crate::paths::Paths;
use crate::state::{ErrorKind, State, ToolError, ToolState, now};
use crate::tools::{Tool, ToolList};

pub fn run(paths: &Paths) -> Result<()> {
    let _lock = acquire_lock(paths)?;
    let mut failures = Vec::new();

    let list = ToolList::load(paths)?;
    let mut state = State::load(paths)?;

    if list.tools.is_empty() {
        println!("No tools registered. Run `toolsync add` inside a repo.");
    }

    for (name, tool) in &list.tools {
        let entry = state.tools.entry(name.clone()).or_default();
        entry.last_run_at = Some(now());
        match sync_tool(paths, name, tool, entry) {
            Ok(outcome) => {
                entry.error = None;
                println!("{name}: {outcome}");
            }
            Err(err) => {
                eprintln!("{name}: {}: {}", err.summary(), err.message);
                entry.error = Some(err);
                failures.push(name.clone());
            }
        }
        state.save(paths)?;
    }

    if !failures.is_empty() {
        bail!("{} item(s) failed: {}", failures.len(), failures.join(", "));
    }
    Ok(())
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
            bail!("another `toolsync sync` is already running on this machine")
        }
        Err(fs::TryLockError::Error(err)) => Err(err).context("locking sync.lock"),
    }
}

fn sync_tool(
    paths: &Paths,
    name: &str,
    tool: &Tool,
    entry: &mut ToolState,
) -> Result<String, ToolError> {
    let refresh = |msg: String| ToolError::new(ErrorKind::Refresh, msg);
    let sync = |msg: String| ToolError::new(ErrorKind::Sync, msg);

    let mut desired = match git::remote_main(None, &tool.url) {
        Ok(Some(sha)) => sha,
        Ok(None) => return Err(refresh(format!("{} has no `main` branch", tool.url))),
        Err(err) => return Err(refresh(format!("{err:#}"))),
    };
    entry.remote_main = Some(desired.clone());
    entry.remote_checked_at = Some(now());

    let dir = paths.checkout(name);
    prepare_checkout(&dir, &tool.url).map_err(|e| sync(format!("{e:#}")))?;

    match tool.pull.as_deref() {
        None => {
            // Built-in sync fetches again; use that newer observation of main.
            desired = builtin_sync(&dir).map_err(|e| sync(format!("{e:#}")))?;
            entry.remote_main = Some(desired.clone());
        }
        Some(cmd) => run_sync_command(&dir, cmd).map_err(|e| sync(format!("{e:#}")))?,
    }

    let head = git::head(&dir).map_err(|e| sync(format!("{e:#}")))?;
    entry.checkout = Some(head.clone());
    verify_checkout(&dir, &head, &desired).map_err(|e| sync(format!("{e:#}")))?;

    let Some(install_cmd) = tool.install.as_deref() else {
        return Ok(format!("synced {} (pull only)", short(&head)));
    };
    if entry.installed.as_deref() == Some(head.as_str()) {
        return Ok(format!("up to date ({})", short(&head)));
    }

    println!("{name}: installing {} ...", short(&head));
    install(paths, name, &dir, install_cmd)
        .map_err(|e| ToolError::new(ErrorKind::Install, format!("{e:#}")))?;
    entry.installed = Some(head.clone());
    entry.installed_at = Some(now());
    Ok(format!("installed {}", short(&head)))
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

fn run_sync_command(dir: &Path, cmd: &str) -> Result<()> {
    let status = shell(dir, cmd)
        .status()
        .with_context(|| format!("running pull command `{cmd}`"))?;
    if !status.success() {
        bail!("pull command `{cmd}` failed ({status})");
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

fn install(paths: &Paths, name: &str, dir: &Path, cmd: &str) -> Result<()> {
    let log_path = paths.install_log(name);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let log =
        File::create(&log_path).with_context(|| format!("creating {}", log_path.display()))?;
    let status = shell(dir, cmd)
        .stdout(log.try_clone()?)
        .stderr(log)
        .status()
        .with_context(|| format!("running `{cmd}`"))?;
    if status.success() {
        return Ok(());
    }

    let text = fs::read_to_string(&log_path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(15)..];
    for line in tail {
        eprintln!("  | {line}");
    }
    bail!("`{cmd}` failed ({status}); log: {}", log_path.display())
}

pub fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}
