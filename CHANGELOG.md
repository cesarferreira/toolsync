# Changelog

All notable changes to this project will be documented in this file.

## [0.2.1] - 2026-11-27

### 🚀 Features

- Use ~/.config and ~/.local/share on macOS too
## [0.2.0] - 2026-11-27

### 🐛 Bug Fixes

- Keep progress rows short of the terminal edge
- Status reads each repo's local main
## [0.1.0] - 2026-11-27

### 🚀 Features

- Initial toolsync MVP
- [**breaking**] Replace shared manifest with per-machine tool list
- Parallel sync with live progress, colors, add prompts; rename binary to tsync
- Status checks every remote in parallel
- Live per-tool status rows and tsync config
- [**breaking**] Sync in place in the registered repo; no more clones
- Tsync --pull pulls without installing

### 📚 Documentation

- Restyle README; fix release packaging for the tsync binary

### ⚙️ Miscellaneous Tasks

- Commit Cargo.lock for reproducible installs
