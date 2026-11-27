use std::collections::BTreeMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// Per-machine record of what was observed and installed. Never shared.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub tools: BTreeMap<String, ToolState>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ToolState {
    /// Last observed SHA of remote `main`.
    pub remote_main: Option<String>,
    /// Unix time of the last successful remote check.
    pub remote_checked_at: Option<u64>,
    /// HEAD of the local repo after the last sync that reached it.
    pub checkout: Option<String>,
    /// SHA of the last successful `make install-release`.
    pub installed: Option<String>,
    pub installed_at: Option<u64>,
    pub last_run_at: Option<u64>,
    pub error: Option<ToolError>,
    /// Why the last sync left the repo alone (another branch, local changes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorKind {
    Refresh,
    Sync,
    Install,
}

impl ToolError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn summary(&self) -> &'static str {
        match self.kind {
            ErrorKind::Refresh => "couldn't refresh; showing last known",
            ErrorKind::Sync => "sync error",
            ErrorKind::Install => "install failed; will retry",
        }
    }
}

impl State {
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.state_file();
        if !file.exists() {
            return Ok(Self::default());
        }
        let text =
            fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))
    }

    /// Write atomically so a crash mid-run never truncates the state file.
    pub fn save(&self, paths: &Paths) -> Result<()> {
        let file = paths.state_file();
        fs::create_dir_all(paths.root())?;
        let tmp = file.with_extension("toml.tmp");
        fs::write(&tmp, toml::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &file).with_context(|| format!("writing {}", file.display()))
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Human-friendly age of a Unix timestamp relative to `now`.
pub fn ago(ts: Option<u64>, now: u64) -> String {
    let Some(ts) = ts else {
        return "never".into();
    };
    let secs = now.saturating_sub(ts);
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_ages() {
        assert_eq!(ago(None, 100), "never");
        assert_eq!(ago(Some(100), 130), "just now");
        assert_eq!(ago(Some(0), 120), "2m ago");
        assert_eq!(ago(Some(0), 7200), "2h ago");
        assert_eq!(ago(Some(0), 3 * 86400), "3d ago");
        assert_eq!(ago(Some(200), 100), "just now");
    }
}
