use std::collections::BTreeMap;
use std::fs;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// This machine's list of tools to keep in sync. Install state lives in `State`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolList {
    #[serde(default)]
    pub tools: BTreeMap<String, Tool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub url: String,
    /// Shell command that installs the tool. Absent means pull only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<String>,
    /// Shell command that updates the checkout. Absent means the built-in Git fast-forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull: Option<String>,
}

pub const DEFAULT_INSTALL: &str = "make install-release";

impl Tool {
    pub fn install_label(&self) -> &str {
        self.install.as_deref().unwrap_or("none (pull only)")
    }

    pub fn pull_label(&self) -> &str {
        self.pull.as_deref().unwrap_or("built-in git fast-forward")
    }
}

impl ToolList {
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.tools_file();
        if !file.exists() {
            return Ok(Self::default());
        }
        let text =
            fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let list: ToolList =
            toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
        for name in list.tools.keys() {
            validate_name(name).with_context(|| format!("in {}", file.display()))?;
        }
        Ok(list)
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let file = paths.tools_file();
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        fs::write(file, text).with_context(|| format!("writing {}", file.display()))
    }
}

/// Names become directory names, so keep them boring.
pub fn validate_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !valid {
        bail!("invalid tool name `{name}`: use letters, digits, `-`, `_` or `.`");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_typical_names() {
        for name in ["toolsync", "my-tool", "my_tool", "tool.rs", "Tool2"] {
            assert!(validate_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn rejects_path_like_names() {
        for name in ["", ".", "..", "../x", "a/b", ".hidden", "a b"] {
            assert!(validate_name(name).is_err(), "{name}");
        }
    }

    #[test]
    fn rejects_unknown_fields() {
        let text = "[tools.x]\nurl = \"u\"\ncommand = \"rm -rf ~\"\n";
        assert!(toml::from_str::<ToolList>(text).is_err());
    }

    #[test]
    fn commands_are_optional() {
        let m: ToolList = toml::from_str("[tools.x]\nurl = \"u\"\n").unwrap();
        assert_eq!(m.tools["x"].install, None);
        assert_eq!(m.tools["x"].pull, None);
        let text = toml::to_string_pretty(&m).unwrap();
        assert!(
            !text.contains("install") && !text.contains("pull"),
            "{text}"
        );
    }
}
