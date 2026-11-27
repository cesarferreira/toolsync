use std::path::PathBuf;

use anyhow::{Context, Result};

/// Machine-local locations. `TOOLSYNC_HOME` puts everything under one directory.
pub struct Paths {
    root: PathBuf,
    config_file: PathBuf,
}

impl Paths {
    pub fn resolve() -> Result<Self> {
        if let Some(home) = std::env::var_os("TOOLSYNC_HOME") {
            let root = PathBuf::from(home);
            let config_file = root.join("config.toml");
            return Ok(Self { root, config_file });
        }
        let root = dirs::data_local_dir()
            .context("couldn't determine the local data directory")?
            .join("toolsync");
        let config_file = dirs::config_dir()
            .context("couldn't determine the config directory")?
            .join("toolsync")
            .join("config.toml");
        Ok(Self { root, config_file })
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    pub fn config_file(&self) -> &PathBuf {
        &self.config_file
    }

    pub fn manifest_dir(&self) -> PathBuf {
        self.root.join("manifest")
    }

    pub fn manifest_file(&self) -> PathBuf {
        self.manifest_dir().join(crate::manifest::FILE_NAME)
    }

    pub fn checkout(&self, name: &str) -> PathBuf {
        self.root.join("repos").join(name)
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.toml")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.root.join("apply.lock")
    }

    pub fn install_log(&self, name: &str) -> PathBuf {
        self.root.join("logs").join(format!("{name}-install.log"))
    }
}
