use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::git;
use crate::manifest::{self, Manifest, Mode, Tool};
use crate::paths::Paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Plain `add`: new entries are pull only; existing entries keep their mode.
    Default,
    Install,
    PullOnly,
}

/// Root of the repo containing the working directory, and the URL of `remote`.
pub fn current_repo(remote: &str) -> Result<(PathBuf, String)> {
    let cwd = std::env::current_dir()?;
    let root = git::run(Some(&cwd), &["rev-parse", "--show-toplevel"])
        .context("not inside a Git repository")?;
    let root = PathBuf::from(root);
    let url = git::run(Some(&root), &["remote", "get-url", remote])
        .with_context(|| format!("remote `{remote}` is not configured in {}", root.display()))?;
    Ok((root, url))
}

pub fn run(paths: &Paths, remote: &str, request: Request, name: Option<String>) -> Result<()> {
    let (root, url) = current_repo(remote)?;

    match git::remote_main(Some(&root), remote)? {
        Some(_) => {}
        None => bail!("remote `{remote}` ({url}) has no `main` branch; toolsync only tracks main"),
    }

    let mut manifest = Manifest::load(paths)?;

    let existing = manifest
        .tools
        .iter()
        .find(|(_, tool)| tool.url == url)
        .map(|(n, _)| n.clone());

    let name = match (existing, name) {
        (Some(existing), Some(requested)) if existing != requested => {
            bail!("{url} is already registered as `{existing}`")
        }
        (Some(existing), _) => existing,
        (None, requested) => {
            let name = match requested {
                Some(n) => n,
                None => name_from_url(&url)?,
            };
            manifest::validate_name(&name)?;
            if let Some(other) = manifest.tools.get(&name) {
                bail!(
                    "`{name}` is already used by {}; pass --name to pick another",
                    other.url
                );
            }
            name
        }
    };

    let previous = manifest.tools.get(&name).cloned();
    let mode = match (request, previous.as_ref().map(|t| t.mode)) {
        (Request::Install, _) => Mode::Install,
        (Request::PullOnly, _) => Mode::Pull,
        (Request::Default, Some(mode)) => mode,
        (Request::Default, None) => Mode::Pull,
    };
    let tool = Tool { url, mode };

    match &previous {
        Some(prev) if *prev == tool => {
            println!("`{name}` is already registered ({})", mode.label());
            if request == Request::Default && mode == Mode::Install {
                println!("It keeps `make install-release`; use --pull-only to turn that off.");
            }
            return Ok(());
        }
        Some(prev) => {
            println!(
                "Updated `{name}`: {} -> {}",
                prev.mode.label(),
                tool.mode.label()
            );
        }
        None => {
            println!("Added `{name}`");
            println!("  url:  {}", tool.url);
            println!("  mode: {} (tracks main)", tool.mode.label());
        }
    }

    manifest.tools.insert(name, tool);
    manifest.save(paths)?;
    println!("Changed {}", paths.manifest_file().display());
    println!("Run `toolsync publish` to share it with your other machines.");
    Ok(())
}

/// `git@github.com:me/my-tool.git` -> `my-tool`
fn name_from_url(url: &str) -> Result<String> {
    let trimmed = url.trim_end_matches('/');
    let last = trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(trimmed)
        .trim_end_matches(".git");
    if last.is_empty() {
        bail!("couldn't derive a name from {url}; pass --name");
    }
    Ok(last.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_names_from_urls() {
        for (url, name) in [
            ("git@github.com:me/my-tool.git", "my-tool"),
            ("https://github.com/me/my-tool", "my-tool"),
            ("https://github.com/me/my-tool.git/", "my-tool"),
            ("/srv/git/other-cli.git", "other-cli"),
            ("host:repo.git", "repo"),
        ] {
            assert_eq!(name_from_url(url).unwrap(), name, "{url}");
        }
    }
}
