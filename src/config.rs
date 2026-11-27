use std::collections::BTreeMap;
use std::fs;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::paths::Paths;

/// Machine-local settings. Never shared through the manifest.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Shell command that syncs a managed checkout, e.g. `stax rs`.
    pub sync_command: Option<String>,
    #[serde(default)]
    pub tools: BTreeMap<String, ToolConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    /// Per-tool override. An empty string selects the built-in Git sync.
    pub sync_command: Option<String>,
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.config_file();
        if !file.exists() {
            return Ok(Self::default());
        }
        let text =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))
    }

    /// The custom sync command for `name`, or `None` for the built-in Git sync.
    pub fn sync_command(&self, name: &str) -> Option<&str> {
        let cmd = self
            .tools
            .get(name)
            .and_then(|t| t.sync_command.as_deref())
            .or(self.sync_command.as_deref())?;
        let cmd = cmd.trim();
        (!cmd.is_empty()).then_some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_tool_override_wins_and_empty_means_builtin() {
        let c: Config = toml::from_str(
            "sync_command = \"stax rs\"\n[tools.a]\nsync_command = \"\"\n[tools.b]\nsync_command = \"other\"\n",
        )
        .unwrap();
        assert_eq!(c.sync_command("a"), None);
        assert_eq!(c.sync_command("b"), Some("other"));
        assert_eq!(c.sync_command("c"), Some("stax rs"));
        assert_eq!(Config::default().sync_command("c"), None);
    }
}
