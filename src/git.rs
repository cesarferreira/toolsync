use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};

fn command(dir: Option<&Path>, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args);
    // Timer runs have no terminal; fail instead of waiting for a credential prompt.
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    cmd
}

/// Run git and return the raw output, whatever the exit status.
pub fn output(dir: Option<&Path>, args: &[&str]) -> Result<Output> {
    command(dir, args)
        .output()
        .with_context(|| format!("failed to run `git {}`", args.join(" ")))
}

/// Run git and return trimmed stdout, or an error carrying stderr.
pub fn run(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let out = output(dir, args)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            bail!("`git {}` failed ({})", args.join(" "), out.status);
        }
        bail!("`git {}` failed: {stderr}", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Run git and report only whether it exited successfully.
pub fn succeeds(dir: Option<&Path>, args: &[&str]) -> Result<bool> {
    Ok(output(dir, args)?.status.success())
}

/// SHA of `refs/heads/main` on `remote` (a URL, or a remote name when `dir` is a repo).
/// Returns `None` when the remote is reachable but has no `main` branch.
pub fn remote_main(dir: Option<&Path>, remote: &str) -> Result<Option<String>> {
    let args = ["ls-remote", "--exit-code", remote, "refs/heads/main"];
    let out = output(dir, &args)?;
    match out.status.code() {
        Some(0) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let sha = stdout
                .split_whitespace()
                .next()
                .context("unexpected empty `git ls-remote` output")?;
            Ok(Some(sha.to_string()))
        }
        // ls-remote --exit-code exits 2 when no matching ref exists.
        Some(2) => Ok(None),
        _ => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            bail!("couldn't reach {remote}: {}", stderr.trim());
        }
    }
}

/// Name of the remote in `dir` whose URL is `url`.
pub fn remote_for_url(dir: &Path, url: &str) -> Result<Option<String>> {
    for remote in run(Some(dir), &["remote"])?.lines() {
        if run(Some(dir), &["remote", "get-url", remote])? == url {
            return Ok(Some(remote.to_string()));
        }
    }
    Ok(None)
}

/// Current branch name, or `None` on a detached HEAD.
pub fn current_branch(dir: &Path) -> Result<Option<String>> {
    let out = output(Some(dir), &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    if out.status.success() {
        Ok(Some(
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        ))
    } else {
        Ok(None)
    }
}

pub fn head(dir: &Path) -> Result<String> {
    run(Some(dir), &["rev-parse", "HEAD"])
}

/// True when tracked files have uncommitted changes.
pub fn is_dirty(dir: &Path) -> Result<bool> {
    let status = run(
        Some(dir),
        &["status", "--porcelain", "--untracked-files=no"],
    )?;
    Ok(!status.is_empty())
}
