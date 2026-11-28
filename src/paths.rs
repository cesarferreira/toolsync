use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ui;

/// Machine-local locations. `TOOLSYNC_HOME` puts everything under one directory.
pub struct Paths {
    root: PathBuf,
    tools_file: PathBuf,
}

impl Paths {
    pub fn resolve() -> Result<Self> {
        if let Some(home) = std::env::var_os("TOOLSYNC_HOME") {
            let root = PathBuf::from(home);
            let tools_file = root.join("tools.toml");
            return Ok(Self { root, tools_file });
        }
        let config = xdg_dir("XDG_CONFIG_HOME", ".config", dirs::config_dir)?.join("toolsync");
        let root = xdg_dir("XDG_DATA_HOME", ".local/share", dirs::data_local_dir)?.join("toolsync");
        let paths = Self {
            root,
            tools_file: config.join("tools.toml"),
        };

        // Earlier versions used the platform directories, which on macOS is
        // ~/Library/Application Support. Move those files once.
        if let Some(legacy) = dirs::config_dir().map(|d| d.join("toolsync"))
            && legacy != config
            && legacy != paths.root
            && legacy.exists()
            && migrate(&legacy, &paths)?
        {
            eprintln!(
                "{}",
                ui::err_dim(format!(
                    "Moved toolsync files from {} to {} and {}",
                    legacy.display(),
                    config.display(),
                    paths.root.display()
                ))
            );
        }
        Ok(paths)
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    /// The tool list lives in the config directory so it is easy to find and edit.
    pub fn tools_file(&self) -> &PathBuf {
        &self.tools_file
    }

    pub fn settings_file(&self) -> PathBuf {
        self.tools_file.with_file_name("settings.toml")
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.toml")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.root.join("sync.lock")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn install_log(&self, name: &str) -> PathBuf {
        self.logs_dir().join(format!("{name}-install.log"))
    }
}

/// `~/.config` and `~/.local/share` style directories on every Unix, macOS
/// included, honoring the XDG variables. Windows keeps its platform directory.
fn xdg_dir(var: &str, fallback: &str, platform: fn() -> Option<PathBuf>) -> Result<PathBuf> {
    if cfg!(windows) {
        return platform().context("couldn't determine the user directories");
    }
    if let Some(dir) = std::env::var_os(var).map(PathBuf::from)
        && dir.is_absolute()
    {
        return Ok(dir);
    }
    Ok(dirs::home_dir()
        .context("couldn't determine the home directory")?
        .join(fallback))
}

/// Move the tool list, state, and logs out of `legacy` unless the new
/// locations are already in use. Returns whether anything moved.
fn migrate(legacy: &Path, paths: &Paths) -> Result<bool> {
    if paths.tools_file.exists() || paths.state_file().exists() {
        return Ok(false);
    }
    let moves = [
        (legacy.join("tools.toml"), paths.tools_file.clone()),
        (legacy.join("state.toml"), paths.state_file()),
        (legacy.join("logs"), paths.logs_dir()),
    ];
    let mut moved = false;
    for (from, to) in moves {
        if from.exists() {
            move_path(&from, &to)?;
            moved = true;
        }
    }
    // The lock file is recreated on demand. Remove the old directory only if
    // nothing else is left in it.
    let _ = fs::remove_file(legacy.join("sync.lock"));
    let _ = fs::remove_dir(legacy);
    Ok(moved)
}

fn move_path(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    // Different filesystems: copy, then delete the original.
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            move_path(&entry.path(), &to.join(entry.file_name()))?;
        }
        fs::remove_dir(from)?;
    } else {
        fs::copy(from, to)
            .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
        fs::remove_file(from)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths_in(dir: &Path) -> Paths {
        Paths {
            root: dir.join("share/toolsync"),
            tools_file: dir.join("config/toolsync/tools.toml"),
        }
    }

    #[test]
    fn migrates_legacy_files_once() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("Application Support/toolsync");
        fs::create_dir_all(legacy.join("logs")).unwrap();
        fs::write(legacy.join("tools.toml"), "[tools]\n").unwrap();
        fs::write(legacy.join("state.toml"), "state").unwrap();
        fs::write(legacy.join("logs/x-install.log"), "log").unwrap();
        fs::write(legacy.join("sync.lock"), "").unwrap();
        let paths = paths_in(tmp.path());

        assert!(migrate(&legacy, &paths).unwrap());
        assert_eq!(fs::read_to_string(paths.tools_file()).unwrap(), "[tools]\n");
        assert_eq!(fs::read_to_string(paths.state_file()).unwrap(), "state");
        assert_eq!(fs::read_to_string(paths.install_log("x")).unwrap(), "log");
        assert!(!legacy.exists());

        // A second run finds nothing to do.
        assert!(!migrate(&legacy, &paths).unwrap());
    }

    #[test]
    fn leaves_legacy_alone_when_new_files_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("tools.toml"), "old").unwrap();
        let paths = paths_in(tmp.path());
        fs::create_dir_all(paths.tools_file().parent().unwrap()).unwrap();
        fs::write(paths.tools_file(), "new").unwrap();

        assert!(!migrate(&legacy, &paths).unwrap());
        assert_eq!(fs::read_to_string(paths.tools_file()).unwrap(), "new");
        assert!(legacy.join("tools.toml").exists());
    }

    #[test]
    fn keeps_unknown_files_in_legacy_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        fs::create_dir_all(legacy.join("repos/x")).unwrap();
        fs::write(legacy.join("tools.toml"), "t").unwrap();
        let paths = paths_in(tmp.path());

        assert!(migrate(&legacy, &paths).unwrap());
        assert!(paths.tools_file().exists());
        assert!(legacy.join("repos/x").exists());
    }
}
