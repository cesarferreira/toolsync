<div align="center">
  <h1>toolsync</h1>

  <p><strong>Keep personal Rust tools synced across machines</strong></p>

  <p>
    <a href="https://github.com/cesarferreira/toolsync/actions/workflows/rust-tests.yml">
      <img alt="Tests" src="https://img.shields.io/github/actions/workflow/status/cesarferreira/toolsync/rust-tests.yml?branch=main&logo=github&color=007ACC&label=tests">
    </a>
    <a href="https://crates.io/crates/toolsync"><img alt="Crates.io" src="https://img.shields.io/crates/v/toolsync?color=blue"></a>
    <a href="https://crates.io/crates/toolsync"><img src="https://img.shields.io/crates/d/toolsync.svg?color=blue" alt="Downloads"></a>
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
  </p>

  <p>
    <a href="#install">Install</a>
    &nbsp;·&nbsp;
    <a href="#quickstart">Quickstart</a>
  </p>
</div>

---

## Install

Requires [Rust](https://rustup.rs) **1.100+** and `~/.cargo/bin` on your `PATH`.

```bash
cargo install toolsync
```

Verify:

```bash
toolsync --help
```

<a id="quickstart"></a>
## Quickstart

Create an empty Git repo for the shared manifest (for example `toolsync-manifest`), then enroll each machine once:

```bash
toolsync init git@github.com:you/toolsync-manifest.git
```

Register a tool from any directory inside its repo, on any branch:

```bash
toolsync add             # track remote main, pull only
toolsync add --install   # track remote main and run `make install-release`
toolsync publish         # commit and push the manifest change
```

On every enrolled machine:

```bash
toolsync apply    # refresh the manifest, sync, and install as needed
toolsync status   # this machine's last known state (no network)
```

```text
TOOL       MAIN     CHECKOUT  INSTALLED  CHECKED  STATUS
my-tool    a91c2d1  a91c2d1   71ef3b0    4m ago   install failed; will retry
other-cli  4a92f37  4a92f37   —          4m ago   synced (pull only)
```

Stop tracking a tool:

```bash
toolsync remove            # from inside the repo, or `toolsync remove <name>`
toolsync publish
toolsync remove <name> --purge   # on each machine: delete its managed checkout and state
```

`--purge` refuses to delete a checkout that has uncommitted changes or unpushed commits. toolsync never uninstalls binaries.

`INSTALLED` is the last commit that `make install-release` built successfully on this machine. It does not prove that the binary on `PATH` matches it.

## How it works

- **Manifest.** The manifest is `toolsync.toml` in a shared Git repo. Each entry holds only a repo URL and a mode (`pull` or `install`). It holds no paths, branches, or shell commands.
- **Managed checkouts.** `apply` clones each tool into its own checkout, separate from your development clone. The built-in sync fast-forwards `main` only. A dirty or diverged checkout is an error and is never overwritten.
- **Installs.** For `install` entries, `make install-release` runs when remote `main` differs from the last successful install. A failed install keeps the previous SHA, and the next run retries it. Build output goes to the install log in the data directory.
- **Failures.** One failing tool does not stop the others. `apply` exits nonzero with a summary if anything failed, including a manifest refresh.
- **Locking.** Only one `apply` runs at a time on a machine.

| What | Where |
|---|---|
| Manifest checkout, managed checkouts, state, logs | `~/.local/share/toolsync/` (macOS: `~/Library/Application Support/toolsync/`) |
| Machine-local config | `~/.config/toolsync/config.toml` (macOS: `~/Library/Application Support/toolsync/config.toml`) |

Set `TOOLSYNC_HOME` to keep everything, including `config.toml`, in one directory.

## Custom sync command

To sync with a command such as `stax rs` instead of the built-in Git sync, set it in the machine-local config:

```toml
sync_command = "stax rs"

[tools.some-tool]
sync_command = ""   # an empty string selects the built-in Git sync
```

The command runs with `sh -c` inside the managed checkout. A zero exit code is not enough: afterwards the checkout must be clean, on `main`, and at the expected commit, or toolsync reports a sync error and skips the install. Shell aliases and functions are not available to a timer, so use the real binary name.

## Run on a timer

`toolsync apply` is safe to run on a schedule. The timer environment must include `cargo`, `make`, `git`, and any sync command on `PATH`.

**Linux (systemd user timer)**

```ini
# ~/.config/systemd/user/toolsync.service
[Unit]
Description=Sync personal Rust tools

[Service]
Type=oneshot
Environment=PATH=%h/.cargo/bin:/usr/local/bin:/usr/bin:/bin
ExecStart=%h/.cargo/bin/toolsync apply
```

```ini
# ~/.config/systemd/user/toolsync.timer
[Timer]
OnCalendar=*:0/30
Persistent=true

[Install]
WantedBy=timers.target
```

```bash
systemctl --user enable --now toolsync.timer
journalctl --user -u toolsync.service   # logs
```

**macOS (launchd)**

```xml
<!-- ~/Library/LaunchAgents/com.example.toolsync.plist -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.example.toolsync</string>
  <key>ProgramArguments</key>
  <array><string>/Users/you/.cargo/bin/toolsync</string><string>apply</string></array>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>/Users/you/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin</string></dict>
  <key>StartInterval</key><integer>1800</integer>
  <key>StandardOutPath</key><string>/tmp/toolsync.log</string>
  <key>StandardErrorPath</key><string>/tmp/toolsync.log</string>
</dict>
</plist>
```

```bash
launchctl load ~/Library/LaunchAgents/com.example.toolsync.plist
```

## Trust

Enrolling a machine means you trust the manifest and every repo in it: `make install-release` runs code from those repos. toolsync runs as your user and never escalates privileges. The manifest selects a mode only; unknown fields such as commands are rejected.

## License

MIT
