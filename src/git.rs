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

/// Name of the remote in `dir` whose fetch URL is `url`.
///
/// One `git remote -v` call. Process launches are slow on some machines, so
/// this avoids one `git remote get-url` per remote.
pub fn remote_for_url(dir: &Path, url: &str) -> Result<Option<String>> {
    Ok(parse_remote_for_url(
        &run(Some(dir), &["remote", "-v"])?,
        url,
    ))
}

fn parse_remote_for_url(remotes: &str, url: &str) -> Option<String> {
    remotes.lines().find_map(|line| {
        let (name, rest) = line.split_once('\t')?;
        let (fetch_url, annotation) = rest.rsplit_once(" (fetch)")?;
        // Git can append the partial clone filter, e.g. ` [blob:none]`.
        let has_filter = annotation.starts_with(" [") && annotation.ends_with(']');
        (fetch_url == url && (annotation.is_empty() || has_filter)).then(|| name.to_string())
    })
}

/// Branch, HEAD and cleanliness of a work tree, read in one `git status` call.
#[derive(Debug, PartialEq, Eq)]
pub struct RepoState {
    /// `None` on a detached HEAD.
    pub branch: Option<String>,
    /// `None` when the branch has no commits yet.
    pub head: Option<String>,
    /// True when tracked files have uncommitted changes.
    pub dirty: bool,
}

pub fn repo_state(dir: &Path) -> Result<RepoState> {
    let out = run(
        Some(dir),
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=no",
        ],
    )?;
    Ok(parse_repo_state(&out))
}

fn parse_repo_state(status: &str) -> RepoState {
    let mut state = RepoState {
        branch: None,
        head: None,
        dirty: false,
    };
    for line in status.lines() {
        if let Some(oid) = line.strip_prefix("# branch.oid ") {
            state.head = (oid != "(initial)").then(|| oid.to_string());
        } else if let Some(name) = line.strip_prefix("# branch.head ") {
            state.branch = (name != "(detached)").then(|| name.to_string());
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            state.dirty = true;
        }
    }
    state
}

pub fn head(dir: &Path) -> Result<String> {
    run(Some(dir), &["rev-parse", "HEAD"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_repo_on_a_branch() {
        let out = "# branch.oid 41bf7171804166213b30c62e1c6d2efa54512337\n\
                   # branch.head main\n\
                   # branch.upstream origin/main\n\
                   # branch.ab +0 -0\n";
        assert_eq!(
            parse_repo_state(out),
            RepoState {
                branch: Some("main".into()),
                head: Some("41bf7171804166213b30c62e1c6d2efa54512337".into()),
                dirty: false,
            }
        );
    }

    #[test]
    fn parses_detached_dirty_repo() {
        let out = "# branch.oid 41bf7171804166213b30c62e1c6d2efa54512337\n\
                   # branch.head (detached)\n\
                   1 .M N... 100644 100644 100644 aaa bbb src/main.rs\n";
        let state = parse_repo_state(out);
        assert_eq!(state.branch, None);
        assert!(state.dirty);
    }

    #[test]
    fn parses_repo_without_commits() {
        let state = parse_repo_state("# branch.oid (initial)\n# branch.head main\n");
        assert_eq!(state.head, None);
        assert_eq!(state.branch.as_deref(), Some("main"));
    }

    #[test]
    fn finds_remote_by_fetch_url() {
        let out = "fork\tgit@github.com:me/x.git (fetch)\n\
                   fork\tgit@github.com:me/x.git (push)\n\
                   origin\tgit@github.com:org/x.git (fetch)\n\
                   origin\tgit@github.com:org/push-only.git (push)\n";
        assert_eq!(
            parse_remote_for_url(out, "git@github.com:org/x.git").as_deref(),
            Some("origin")
        );
        assert_eq!(
            parse_remote_for_url(out, "git@github.com:org/push-only.git"),
            None
        );
    }

    #[test]
    fn finds_partial_clone_remote_by_fetch_url() {
        let out = "fork\t/local/other repo (fetch)\n\
                   fork\t/local/my repo (fetch) (push)\n\
                   upstream\t/local/my repo (fetch) (fetch) [blob:none]\n\
                   upstream\t/local/my repo (fetch) (push)\n";
        assert_eq!(
            parse_remote_for_url(out, "/local/my repo (fetch)").as_deref(),
            Some("upstream")
        );
    }
}
