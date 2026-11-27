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
cargo install toolsync   # installs the `tsync` binary
```

Verify:

```bash
tsync --help
```

<a id="quickstart"></a>
## Quickstart

Register a tool from any directory inside its repo, on any branch:

```bash
tsync add       # asks how to pull and install; Enter accepts the default
tsync add -y    # accept the defaults without asking
```

```text
✔ How should toolsync pull my-tool? · Built-in git fast-forward
✔ How should toolsync install my-tool? · Run `make install-release`
Added my-tool (tracks main)
```

For a new tool, the default install is `make install-release` when the repo's Makefile has that target, and pull only otherwise. For an existing tool, the defaults are its current settings.

Then pull and install everything you registered:

```bash
tsync                    # same as `tsync sync`
tsync -j 2               # same, at most 2 tools at a time
tsync status             # check every remote in parallel, then show the table
tsync status --offline   # last known state, no network
tsync config             # where the tool list and data live
```

All tools sync in parallel. Each tool gets a live spinner that shows what it is doing, including the latest line of install output, and turns into ✔ or ✖ when it finishes:

```text
Syncing 3 tool(s), 3 at a time
✖ broken-cli    1.1s  install failed; will retry
⠋ stax          1.5s  installing · Compiling anyhow v1.0.0
✔ toppy         0.0s  synced aed9b83 (pull only)
```

When output is not a terminal (a timer or a pipe), toolsync prints one plain line per tool instead.

`tsync status` checks every tool's remote at once the same way, and each row ends with the tool's state:

```text
Checking 3 tool(s)
✔ alpha    0.4s  installed a852d2d
● beta     0.5s  behind main a852d2d → 136b13b
✖ gamma    0.3s  couldn't refresh; showing last known a852d2d
```

Piped or with `--offline`, it prints a table instead:

```text
TOOL       MAIN     CHECKOUT  INSTALLED  CHECKED  STATUS
my-tool    a91c2d1  a91c2d1   71ef3b0    4m ago   install failed; will retry
other-cli  4a92f37  4a92f37   —          4m ago   synced (pull only)
```

`status` reads remote `main` for all tools at once, so `MAIN` is live and `behind main` means there is something to pull. If a remote can't be reached, it shows the last known value. `INSTALLED` is the last commit whose install command succeeded on this machine. It does not prove that the binary on `PATH` matches it.

Stop tracking a tool:

```bash
tsync remove            # from inside the repo, or `tsync remove <name>`
```

`remove` also deletes the managed checkout. It refuses if that checkout has uncommitted changes or unpushed commits. toolsync never uninstalls binaries.

## Custom pull and install commands

Each tool can have its own commands. They run with `sh -c` inside the managed checkout.

```bash
tsync add --install "cargo install --path ."   # custom install command
tsync add --pull "stax rs"                     # custom pull command
tsync add --pull ""                            # back to the built-in git sync
tsync add --pull-only                          # stop installing
```

Flags skip the matching question. `add` never asks when you pass `-y` or when it is not attached to a terminal, so it is safe in scripts. Running `add` again keeps the current settings unless you change them, so it never turns off installing by accident.

After any pull, the checkout must be clean, on `main`, and at the latest remote commit. If not, toolsync reports a sync error and skips the install. A zero exit code is not enough.

## How it works

- **Tool list.** Each machine keeps its own list in `tools.toml`. Each entry has a repo URL and optional `install` and `pull` commands. It holds no paths or branches, so you can edit it by hand.
- **Managed checkouts.** `sync` clones each tool into its own checkout, separate from your development clone. The built-in pull fast-forwards `main` only. A dirty or diverged checkout is an error and is never overwritten.
- **Installs.** The install command runs when remote `main` differs from the last successful install. A failed install keeps the previous SHA, and the next run retries it. Install output goes to a log in the data directory.
- **Parallel.** All tools sync at once by default. Use `-j N` to limit how many run together, for example to keep several `cargo install` builds from competing for CPU.
- **Failures.** One failing tool does not stop the others. `sync` exits nonzero with a summary if anything failed.
- **Locking.** Only one `sync` runs at a time on a machine.

```toml
# tools.toml
[tools.my-tool]
url = "git@github.com:you/my-tool.git"
install = "make install-release"   # absent = pull only
pull = "stax rs"                   # absent = built-in git fast-forward
```

| What | Where |
|---|---|
| Tool list | `~/.config/toolsync/tools.toml` (macOS: `~/Library/Application Support/toolsync/tools.toml`) |
| Managed checkouts, state, logs | `~/.local/share/toolsync/` (macOS: `~/Library/Application Support/toolsync/`) |

`tsync config` prints these paths for this machine. `tsync config --path` prints only the tool list path, for example `$EDITOR "$(tsync config --path)"`.

Set `TOOLSYNC_HOME` to keep everything, including `tools.toml`, in one directory.

## Run on a timer

`tsync sync` is safe to run on a schedule. The timer environment must include `git`, `cargo`, `make`, and any custom commands on `PATH`. Shell aliases and functions are not available to a timer, so use real binary names in custom commands.

**Linux (systemd user timer)**

```ini
# ~/.config/systemd/user/toolsync.service
[Unit]
Description=Sync personal Rust tools

[Service]
Type=oneshot
Environment=PATH=%h/.cargo/bin:/usr/local/bin:/usr/bin:/bin
ExecStart=%h/.cargo/bin/tsync sync
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
  <array><string>/Users/you/.cargo/bin/tsync</string><string>sync</string></array>
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

The install command runs code from the repo, and custom commands run as you. toolsync never escalates privileges. Only register repos you trust.

## License

MIT
