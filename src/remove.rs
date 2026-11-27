use std::fs;

use anyhow::{Context, Result, bail};

use crate::add;
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
                "tsync does not uninstall binaries or touch the repo; the last installed build stays on PATH."
            )
        );
    }
    Ok(())
}

struct Purged {
    found: bool,
    had_install: bool,
}

/// Forget this machine's install log and state for `name`. The repo itself is never touched.
fn purge_local(paths: &Paths, name: &str) -> Result<Purged> {
    let _lock = sync::acquire_lock(paths)?;
    let mut purged = Purged {
        found: false,
        had_install: false,
    };

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
