use std::path::PathBuf;

use anyhow::{Context, Result};

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
        let root = dirs::data_local_dir()
            .context("couldn't determine the local data directory")?
            .join("toolsync");
        let tools_file = dirs::config_dir()
            .context("couldn't determine the config directory")?
            .join("toolsync")
            .join("tools.toml");
        Ok(Self { root, tools_file })
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    /// The tool list lives in the config directory so it is easy to find and edit.
    pub fn tools_file(&self) -> &PathBuf {
        &self.tools_file
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.toml")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.root.join("sync.lock")
    }

    pub fn install_log(&self, name: &str) -> PathBuf {
        self.root.join("logs").join(format!("{name}-install.log"))
    }
}
