use std::collections::BTreeMap;
use std::fs;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::git;
use crate::paths::Paths;

pub const FILE_NAME: &str = "toolsync.toml";

/// The shared, Git-backed list of tools. It holds no machine-local paths or state.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub tools: BTreeMap<String, Tool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub url: String,
    #[serde(default)]
    pub mode: Mode,
}

/// The manifest picks a mode; it never carries shell commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Pull,
    Install,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Pull => "pull only",
            Mode::Install => "make install-release",
        }
    }
}

impl Manifest {
    pub fn load(paths: &Paths) -> Result<Self> {
        ensure_enrolled(paths)?;
        let file = paths.manifest_file();
        if !file.exists() {
            return Ok(Self::default());
        }
        let text =
            fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        let manifest: Manifest =
            toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
        for name in manifest.tools.keys() {
            validate_name(name).with_context(|| format!("in {}", file.display()))?;
        }
        Ok(manifest)
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let file = paths.manifest_file();
        let text = toml::to_string_pretty(self)?;
        fs::write(&file, text).with_context(|| format!("writing {}", file.display()))
    }
}

/// Names become directory names on every enrolled machine, so keep them boring.
pub fn validate_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !valid {
        bail!("invalid tool name `{name}`: use letters, digits, `-`, `_` or `.`");
    }
    Ok(())
}

fn ensure_enrolled(paths: &Paths) -> Result<()> {
    if !paths.manifest_dir().join(".git").exists() {
        bail!("this machine is not enrolled; run `toolsync init <manifest-git-url>` first");
    }
    Ok(())
}

pub fn init(paths: &Paths, url: &str) -> Result<()> {
    let dir = paths.manifest_dir();
    if dir.exists() {
        let existing = git::run(Some(&dir), &["remote", "get-url", "origin"]).unwrap_or_default();
        if existing == url {
            println!("Already enrolled with {url}");
            return Ok(());
        }
        bail!(
            "{} already exists and tracks `{existing}`; remove it to enroll with a different manifest",
            dir.display()
        );
    }
    fs::create_dir_all(paths.root())
        .with_context(|| format!("creating {}", paths.root().display()))?;
    let dir_str = dir.to_string_lossy();
    git::run(None, &["clone", "--quiet", url, &dir_str])
        .with_context(|| format!("cloning manifest {url}"))?;

    let count = Manifest::load(paths)?.tools.len();
    println!("Enrolled with {url} ({count} tool(s))");
    println!("Manifest: {}", dir.display());
    println!("Next: run `toolsync apply`, or schedule it with a user timer (see README).");
    Ok(())
}

/// Fast-forward the local manifest to its remote. Local edits are kept; a conflict is an error.
pub fn refresh(paths: &Paths) -> Result<()> {
    ensure_enrolled(paths)?;
    let dir = paths.manifest_dir();
    git::run(Some(&dir), &["fetch", "--quiet", "origin"])?;
    let branch = git::current_branch(&dir)?.context("manifest checkout is on a detached HEAD")?;
    let upstream = format!("refs/remotes/origin/{branch}");
    if !git::succeeds(Some(&dir), &["rev-parse", "--verify", "--quiet", &upstream])? {
        // Nothing has been published yet.
        return Ok(());
    }
    git::run(Some(&dir), &["merge", "--ff-only", "--quiet", &upstream]).context(
        "local manifest has unpublished changes that conflict with the remote; \
         run `toolsync publish` or resolve them in the manifest checkout",
    )?;
    Ok(())
}

pub fn publish(paths: &Paths, message: &str) -> Result<()> {
    ensure_enrolled(paths)?;
    let dir = paths.manifest_dir();
    let branch = git::current_branch(&dir)?.context("manifest checkout is on a detached HEAD")?;

    if paths.manifest_file().exists() {
        git::run(Some(&dir), &["add", FILE_NAME])?;
    }
    let staged = !git::succeeds(Some(&dir), &["diff", "--cached", "--quiet"])?;
    if staged {
        git::run(Some(&dir), &["commit", "--quiet", "-m", message])?;
    }

    git::run(Some(&dir), &["fetch", "--quiet", "origin"])?;
    let upstream = format!("refs/remotes/origin/{branch}");
    let has_upstream = git::succeeds(Some(&dir), &["rev-parse", "--verify", "--quiet", &upstream])?;
    let has_head = git::succeeds(Some(&dir), &["rev-parse", "--verify", "--quiet", "HEAD"])?;
    if !has_head {
        println!("Nothing to publish");
        return Ok(());
    }

    if has_upstream {
        let ahead = git::run(
            Some(&dir),
            &["rev-list", "--count", &format!("{upstream}..HEAD")],
        )?;
        if ahead == "0" {
            println!("Nothing to publish");
            return Ok(());
        }
        let up_to_date = git::succeeds(
            Some(&dir),
            &["merge-base", "--is-ancestor", &upstream, "HEAD"],
        )?;
        if !up_to_date && git::run(Some(&dir), &["rebase", "--quiet", &upstream]).is_err() {
            let _ = git::run(Some(&dir), &["rebase", "--abort"]);
            bail!(
                "the remote manifest changed and your edits conflict; resolve them in {}",
                dir.display()
            );
        }
    }

    git::run(Some(&dir), &["push", "--quiet", "-u", "origin", &branch])
        .context("pushing the manifest")?;
    println!("Published manifest to origin/{branch}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_typical_names() {
        for name in ["toolsync", "my-tool", "my_tool", "tool.rs", "Tool2"] {
            assert!(validate_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn rejects_path_like_names() {
        for name in ["", ".", "..", "../x", "a/b", ".hidden", "a b"] {
            assert!(validate_name(name).is_err(), "{name}");
        }
    }

    #[test]
    fn rejects_unknown_fields() {
        let text = "[tools.x]\nurl = \"u\"\ncommand = \"rm -rf ~\"\n";
        assert!(toml::from_str::<Manifest>(text).is_err());
    }

    #[test]
    fn mode_defaults_to_pull() {
        let m: Manifest = toml::from_str("[tools.x]\nurl = \"u\"\n").unwrap();
        assert_eq!(m.tools["x"].mode, Mode::Pull);
    }
}
