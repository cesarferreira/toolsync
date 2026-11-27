use std::fs;

use anyhow::{Context, Result, bail};

use crate::add;
use crate::git;
use crate::paths::Paths;
use crate::state::State;
use crate::sync;
use crate::tools::{self, ToolList};
use crate::ui;

pub fn run(paths: &Paths, name: Option<String>, remote: &str) -> Result<()> {
    let mut list = ToolList::load(paths)?;

    let name = match name {
        Some(name) => {
            tools::validate_name(&name)?;
            name
        }
        None => {
            let (_, url) = add::current_repo(remote)?;
            list.tools
                .iter()
                .find(|(_, tool)| tool.url == url)
                .map(|(n, _)| n.clone())
                .with_context(|| format!("{url} is not registered"))?
        }
    };

    let registered = list.tools.contains_key(&name);
    let mut had_install = list.tools.get(&name).is_some_and(|t| t.install.is_some());

    // Purge first: if the checkout holds local work, nothing changes.
    let purged = purge_local(paths, &name)?;
    if !purged.found && !registered {
        bail!("`{name}` is not registered on this machine");
    }
    had_install |= purged.had_install;

    if registered {
        list.tools.remove(&name);
        list.save(paths)?;
    }
    println!("{} {}", ui::good("Removed"), ui::name(&name));
    if had_install {
        println!(
            "{}",
            ui::dim(
                "toolsync does not uninstall binaries; the last installed build stays on PATH."
            )
        );
    }
    Ok(())
}

struct Purged {
    found: bool,
    had_install: bool,
}

/// Delete this machine's managed checkout, install log, and state for `name`.
fn purge_local(paths: &Paths, name: &str) -> Result<Purged> {
    let _lock = sync::acquire_lock(paths)?;
    let mut purged = Purged {
        found: false,
        had_install: false,
    };

    let dir = paths.checkout(name);
    if dir.exists() {
        if dir.join(".git").exists() {
            if git::is_dirty(&dir)? {
                bail!(
                    "managed checkout {} has uncommitted changes; delete it yourself if they can go",
                    dir.display()
                );
            }
            let unpushed = git::run(
                Some(&dir),
                &["rev-list", "--count", "HEAD", "--not", "--remotes"],
            )?;
            if unpushed != "0" {
                bail!(
                    "managed checkout {} has commits that aren't on its remote; delete it yourself if they can go",
                    dir.display()
                );
            }
        }
        fs::remove_dir_all(&dir).with_context(|| format!("deleting {}", dir.display()))?;
        println!("{} {}", ui::dim("Deleted"), ui::dim(dir.display()));
        purged.found = true;
    }

    let log = paths.install_log(name);
    if log.exists() {
        fs::remove_file(&log).with_context(|| format!("deleting {}", log.display()))?;
    }

    let mut state = State::load(paths)?;
    if let Some(st) = state.tools.remove(name) {
        purged.had_install = st.installed.is_some();
        purged.found = true;
        state.save(paths)?;
    }
    Ok(purged)
}
