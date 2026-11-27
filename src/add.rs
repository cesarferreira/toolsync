use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Input, Select};

use crate::git;
use crate::paths::Paths;
use crate::tools::{self, DEFAULT_INSTALL, Tool, ToolList};
use crate::ui;

/// What `add` should change. Fields left as `None`/`false` keep an existing entry's value.
#[derive(Debug, Default)]
pub struct Changes {
    /// New install command. An empty string means pull only.
    pub install: Option<String>,
    /// Remove the install command.
    pub pull_only: bool,
    /// New pull command. An empty string selects the built-in Git sync.
    pub pull: Option<String>,
    /// Accept defaults instead of prompting for settings not given as flags.
    pub yes: bool,
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
    let mut tool = previous.clone().unwrap_or_else(|| Tool {
        url: url.clone(),
        path: None,
        install: detect_install(&root),
        pull: None,
    });
    // Sync works in this repo, so follow it if it moved.
    tool.path = Some(root.clone());
    let pull_given = changes.pull.is_some();
    let install_given = changes.install.is_some() || changes.pull_only;
    if let Some(cmd) = changes.install {
        tool.install = non_empty(cmd);
    } else if changes.pull_only {
        tool.install = None;
    }
    if let Some(cmd) = changes.pull {
        tool.pull = non_empty(cmd);
    }

    let interactive =
        !changes.yes && std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if interactive && !(pull_given && install_given) {
        println!("{} {}", ui::bold("Configuring"), ui::name(&name));
        let theme = ColorfulTheme::default();
        if !pull_given {
            tool.pull = prompt_pull(&theme, &name, tool.pull.take())?;
        }
        if !install_given {
            tool.install = prompt_install(&theme, &name, tool.install.take())?;
        }
    }

    match &previous {
        Some(prev) if *prev == tool => {
            println!("{} is already registered", ui::name(&name));
            print_tool(&tool);
            return Ok(());
        }
        Some(_) => println!("{} {}", ui::good("Updated"), ui::name(&name)),
        None => println!(
            "{} {} {}",
            ui::good("Added"),
            ui::name(&name),
            ui::dim("(syncs main in place)")
        ),
    }
    print_tool(&tool);

    list.tools.insert(name, tool);
    list.save(paths)?;
    println!("Run {} to pull it now.", ui::cmd("tsync"));
    println!(
        "{}",
        ui::dim("Sync skips this repo while it is on another branch or has uncommitted changes.")
    );
    Ok(())
}

fn print_tool(tool: &Tool) {
    println!("  {}     {}", ui::dim("url:"), tool.url);
    if let Some(path) = &tool.path {
        println!("  {}    {}", ui::dim("path:"), path.display());
    }
    println!(
        "  {}    {}",
        ui::dim("pull:"),
        describe(tool.pull.as_deref(), tool.pull_label())
    );
    println!(
        "  {} {}",
        ui::dim("install:"),
        describe(tool.install.as_deref(), tool.install_label())
    );
}

/// Commands in magenta, built-in behaviour dimmed.
fn describe(cmd: Option<&str>, label: &str) -> String {
    match cmd {
        Some(c) => ui::cmd(c).to_string(),
        None => ui::dim(label).to_string(),
    }
}

/// Default install command for a new entry: `make install-release` when the repo has that target.
fn detect_install(root: &Path) -> Option<String> {
    let makefile = fs::read_to_string(root.join("Makefile")).unwrap_or_default();
    makefile_has_target(&makefile, "install-release").then(|| DEFAULT_INSTALL.to_string())
}

fn makefile_has_target(makefile: &str, target: &str) -> bool {
    makefile.lines().any(|line| {
        line.split_once(':').is_some_and(|(names, rest)| {
            !rest.starts_with('=') && names.split_whitespace().any(|n| n == target)
        })
    })
}

fn prompt_pull(
    theme: &ColorfulTheme,
    name: &str,
    current: Option<String>,
) -> Result<Option<String>> {
    let choices = ["Built-in git fast-forward", "Custom command"];
    let choice = Select::with_theme(theme)
        .with_prompt(format!("How should toolsync pull {name}?"))
        .items(choices)
        .default(usize::from(current.is_some()))
        .interact()?;
    if choice == 0 {
        return Ok(None);
    }
    prompt_command(theme, "Pull command", current).map(Some)
}

fn prompt_install(
    theme: &ColorfulTheme,
    name: &str,
    current: Option<String>,
) -> Result<Option<String>> {
    let choices = [
        format!("Run `{DEFAULT_INSTALL}`"),
        "Custom command".to_string(),
        "Don't install (pull only)".to_string(),
    ];
    let default = match current.as_deref() {
        Some(DEFAULT_INSTALL) => 0,
        Some(_) => 1,
        None => 2,
    };
    let choice = Select::with_theme(theme)
        .with_prompt(format!("How should toolsync install {name}?"))
        .items(&choices)
        .default(default)
        .interact()?;
    match choice {
        0 => Ok(Some(DEFAULT_INSTALL.to_string())),
        1 => {
            let current = current.filter(|c| c != DEFAULT_INSTALL);
            prompt_command(theme, "Install command", current).map(Some)
        }
        _ => Ok(None),
    }
}

fn prompt_command(theme: &ColorfulTheme, prompt: &str, current: Option<String>) -> Result<String> {
    let mut input = Input::<String>::with_theme(theme).with_prompt(prompt);
    if let Some(current) = current {
        input = input.default(current);
    }
    let cmd = input
        .validate_with(|s: &String| {
            if s.trim().is_empty() {
                Err("enter a command")
            } else {
                Ok(())
            }
        })
        .interact_text()?;
    Ok(cmd.trim().to_string())
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
    fn finds_makefile_targets() {
        let mk = "VAR := x\n.PHONY: build install-release\nbuild:\n\tcargo build\ninstall-release: build\n";
        assert!(makefile_has_target(mk, "install-release"));
        assert!(makefile_has_target(mk, "build"));
        assert!(!makefile_has_target(mk, "install"));
        assert!(!makefile_has_target(
            "install-release := 1\n",
            "install-release"
        ));
        assert!(!makefile_has_target("", "install-release"));
    }

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
