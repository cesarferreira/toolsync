use std::fs;

use anyhow::{Context, Result, bail};

use crate::add;
use crate::apply;
use crate::git;
use crate::manifest::{self, Manifest, Mode};
use crate::paths::Paths;
use crate::state::State;

pub fn run(paths: &Paths, name: Option<String>, remote: &str, purge: bool) -> Result<()> {
    let mut manifest = Manifest::load(paths)?;

    let name = match name {
        Some(name) => {
            manifest::validate_name(&name)?;
            name
        }
        None => {
            let (_, url) = add::current_repo(remote)?;
            manifest
                .tools
                .iter()
                .find(|(_, tool)| tool.url == url)
                .map(|(n, _)| n.clone())
                .with_context(|| format!("{url} is not in the manifest"))?
        }
    };

    let in_manifest = manifest.tools.contains_key(&name);
    if !in_manifest && !purge {
        bail!("`{name}` is not in the manifest");
    }

    // Purge first: if the checkout holds local work, nothing changes.
    let mut had_install = manifest
        .tools
        .get(&name)
        .is_some_and(|t| t.mode == Mode::Install);
    if purge {
        let purged = purge_local(paths, &name)?;
        if !purged.found && !in_manifest {
            bail!("nothing to remove: `{name}` is not in the manifest or on this machine");
        }
        had_install |= purged.had_install;
    }

    if in_manifest {
        manifest.tools.remove(&name);
        manifest.save(paths)?;
        println!("Removed `{name}` from the manifest");
        println!("Run `toolsync publish` to share it with your other machines.");
    }
    if had_install {
        println!("toolsync does not uninstall binaries; the last installed build stays on PATH.");
    }
    Ok(())
}

struct Purged {
    found: bool,
    had_install: bool,
}

/// Delete this machine's managed checkout, install log, and state for `name`.
fn purge_local(paths: &Paths, name: &str) -> Result<Purged> {
    let _lock = apply::acquire_lock(paths)?;
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
        println!("Deleted {}", dir.display());
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
