use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::git;
use crate::paths::Paths;
use crate::tools::{self, Tool, ToolList};

/// What `add` should change. Fields left as `None`/`false` keep an existing entry's value.
#[derive(Debug, Default)]
pub struct Changes {
    /// New install command. An empty string means pull only.
    pub install: Option<String>,
    /// Remove the install command.
    pub pull_only: bool,
    /// New pull command. An empty string selects the built-in Git sync.
    pub pull: Option<String>,
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

pub fn run(paths: &Paths, remote: &str, name: Option<String>, changes: Changes) -> Result<()> {
    let (root, url) = current_repo(remote)?;

    match git::remote_main(Some(&root), remote)? {
        Some(_) => {}
        None => bail!("remote `{remote}` ({url}) has no `main` branch; toolsync only tracks main"),
    }

    let mut list = ToolList::load(paths)?;

    let existing = list
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
            tools::validate_name(&name)?;
            if let Some(other) = list.tools.get(&name) {
                bail!(
                    "`{name}` is already used by {}; pass --name to pick another",
                    other.url
                );
            }
            name
        }
    };

    let previous = list.tools.get(&name).cloned();
    let mut tool = previous.clone().unwrap_or(Tool {
        url: url.clone(),
        install: None,
        pull: None,
    });
    let keeps_install = changes.install.is_none() && !changes.pull_only;
    if let Some(cmd) = changes.install {
        tool.install = non_empty(cmd);
    } else if changes.pull_only {
        tool.install = None;
    }
    if let Some(cmd) = changes.pull {
        tool.pull = non_empty(cmd);
    }

    match &previous {
        Some(prev) if *prev == tool => {
            println!("`{name}` is already registered");
            print_tool(&tool);
            if keeps_install && tool.install.is_some() {
                println!("Use --pull-only to stop installing it.");
            }
            return Ok(());
        }
        Some(_) => println!("Updated `{name}`"),
        None => println!("Added `{name}` (tracks main)"),
    }
    print_tool(&tool);

    list.tools.insert(name, tool);
    list.save(paths)?;
    println!("Run `toolsync` to pull it now.");
    Ok(())
}

fn print_tool(tool: &Tool) {
    println!("  url:     {}", tool.url);
    println!("  pull:    {}", tool.pull_label());
    println!("  install: {}", tool.install_label());
}

fn non_empty(cmd: String) -> Option<String> {
    let cmd = cmd.trim();
    (!cmd.is_empty()).then(|| cmd.to_string())
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
