use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::project::IconStyle;

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    icons: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { icons: true }
    }
}

impl Settings {
    fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        };
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

pub fn icon_style(path: &Path, override_style: Option<IconStyle>) -> Result<IconStyle> {
    if let Some(style) = override_style {
        return Ok(style);
    }
    Ok(if Settings::load(path)?.icons {
        IconStyle::default()
    } else {
        IconStyle::None
    })
}

pub fn set_icons(path: &Path, enabled: bool) -> Result<()> {
    let mut settings = Settings::load(path)?;
    settings.icons = enabled;

    let dir = path.parent().context("settings path has no directory")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    // A unique temporary file keeps concurrent toggles and readers independent.
    let mut tmp = NamedTempFile::new_in(dir)
        .with_context(|| format!("creating a temporary file in {}", dir.display()))?;
    tmp.write_all(toml::to_string_pretty(&settings)?.as_bytes())
        .with_context(|| format!("writing {}", tmp.path().display()))?;
    tmp.persist(path)
        .with_context(|| format!("saving {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_icon_style_takes_precedence_over_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        assert!(matches!(icon_style(&path, None).unwrap(), IconStyle::Emoji));

        fs::write(&path, "icons = false\n").unwrap();
        assert!(matches!(icon_style(&path, None).unwrap(), IconStyle::None));
        assert!(matches!(
            icon_style(&path, Some(IconStyle::Nerd)).unwrap(),
            IconStyle::Nerd
        ));
        assert!(matches!(icon_style(&path, None).unwrap(), IconStyle::None));
    }
}
