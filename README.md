<div align="center">
  <h1>toolsync</h1>
  <p>
    Keep personal Rust tools synced across machines
  </p>

  <p>
    <a href="https://github.com/cesarferreira/toolsync/actions/workflows/rust-tests.yml"><img alt="CI" src="https://github.com/cesarferreira/toolsync/actions/workflows/rust-tests.yml/badge.svg"></a>
    <a href="https://crates.io/crates/toolsync"><img alt="Crates.io" src="https://img.shields.io/crates/v/toolsync"></a>
    <img alt="Rust" src="https://img.shields.io/badge/rust-1.100%2B-orange?logo=rust">
    <img alt="Git" src="https://img.shields.io/badge/git-in--place%20sync-F05032?logo=git&logoColor=white">
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
  </p>
</div>

## Install

```bash
cargo install toolsync   # installs the `tsync` binary
tsync --help
```

Or the latest `main`:

```bash
cargo install --git https://github.com/cesarferreira/toolsync
```

## Quick start

Register a tool from anywhere inside its repo, on any branch. tsync remembers that repo and syncs it in place. It never clones anything.

```bash
tsync add       # asks how to pull and install; Enter accepts the default
tsync add -y    # accept the defaults without asking
```

```text
✔ How should toolsync pull my-tool? · Built-in git fast-forward
✔ How should toolsync install my-tool? · Run `make install-release`
Added my-tool (syncs main in place)
```

Then pull and install everything you registered:

```bash
tsync
```

```text
Syncing 5 tool(s), 5 at a time
✖ broken-cli    1.1s  pulled +1 commit · install failed; will retry
✔ markers       0.4s  installed existing checkout (a852d2d)
● space         0.2s  skipped: on branch feature-x
↑ stax          1.5s  pulled +4 commits · installed (bc5a317)
· toppy         0.0s  up to date · pull only (aed9b83)

2 pulled (5 commits) · 2 installed · 1 up to date · 1 skipped · 1 install failed
```

All tools sync in parallel. Each row shows what the tool is doing, including the latest line of install output. The final rows show which checkouts moved, how many commits they gained, and any install result. The totals count pulls and installs separately, so a tool can appear in both. A pipe or timer gets plain output with the same results and totals.

Stop tracking a tool:

```bash
tsync remove            # from inside the repo
tsync remove my-tool    # or by name
```

`remove` only forgets the tool. It never touches the repo and never uninstalls binaries.

## Options

```bash
tsync                    # sync: pull every tool, install what changed (same as `tsync sync`)
tsync -j 2               # at most 2 tools at a time
tsync --pull             # only pull; skip installs this run
tsync status             # check every remote in parallel, then show each tool's state
tsync status --offline   # last known state, no network
tsync config             # where the tool list and data live
tsync config --path      # only the tool list path
```

`tsync add` flags. Each one skips the matching question:

| Flag | Effect |
|---|---|
| `--install [CMD]` | Install after each new commit on `main` (default command: `make install-release`) |
| `--pull-only` | Stop installing |
| `--pull <CMD>` | Custom pull command; `""` goes back to the built-in git fast-forward |
| `--remote <NAME>` | Remote to register (default: `origin`) |
| `--name <NAME>` | Entry name (default: repo name from the URL) |
| `-y`, `--yes` | Accept defaults without asking |

For a new tool, the default install is `make install-release` when the repo's Makefile has that target, and pull only otherwise. Running `add` again keeps the current settings unless you change them, so it never turns off installing by accident. `add` never asks when you pass `-y` or when it isn't attached to a terminal, so it's safe in scripts.

## Configuration

Each machine keeps its own tool list. Each entry has the repo URL, the repo's path on this machine, and optional `install` and `pull` commands. You can edit it by hand:

```toml
# tools.toml
[tools.my-tool]
url = "git@github.com:you/my-tool.git"
path = "/home/you/code/my-tool"
install = "make install-release"   # absent = pull only
pull = "stax rs"                   # absent = built-in git fast-forward
```

| What | Linux and macOS |
|---|---|
| Tool list | `~/.config/toolsync/tools.toml` (or `$XDG_CONFIG_HOME/toolsync/`) |
| State, logs | `~/.local/share/toolsync/` (or `$XDG_DATA_HOME/toolsync/`) |

Earlier versions kept everything in `~/Library/Application Support/toolsync/` on macOS. tsync moves those files to the new locations the first time it runs.

```bash
tsync config                         # print these paths for this machine
$EDITOR "$(tsync config --path)"     # edit the tool list
TOOLSYNC_HOME=~/tsync tsync          # keep everything, including tools.toml, in one directory
```

## How sync works

- **In place.** Sync pulls and builds in the repo you registered. If you move a repo, run `tsync add` in its new location.
- **Only a clean `main`.** A repo on another branch, with uncommitted changes to tracked files, or with unpushed commits on `main` is **skipped** (●) and left exactly as it is. Untracked files don't block a sync. A `main` that has diverged from the remote is an error.
- **Built-in pull.** Fetches `main` from the remote whose URL matches the entry, then fast-forwards. It never merges, rebases, or switches branches.
- **Verified.** After any pull, the repo must be clean, on `main`, and at the latest remote commit. Otherwise it's a sync error and the install doesn't run. A zero exit code isn't enough.
- **Installs.** The install command runs when the pulled commit differs from the last successful install. A failed install keeps the previous SHA, and the next run retries it. Output goes to a log in the data directory.
- **Pull without installing.** `tsync --pull` runs no installs. `status` then shows `install pending`, and the next plain `tsync` installs it.
- **Parallel.** All tools sync at once by default. Use `-j N` to keep several `cargo install` builds from competing for CPU.
- **Failures.** One failing tool doesn't stop the others. Sync exits nonzero with a summary if anything failed. Skips aren't failures.
- **Locking.** Only one sync runs at a time on a machine.

## Status

`tsync status` reads remote `main` for every tool at once, so you see what there is to pull without pulling it:

```text
Checking 3 tool(s)
✔ alpha    0.4s  installed a852d2d
● beta     0.5s  behind main a852d2d → 136b13b
✖ gamma    0.3s  couldn't refresh; showing last known a852d2d
```

Piped or with `--offline`, it prints a table instead:

```text
TOOL       MAIN     LOCAL     INSTALLED  CHECKED  STATUS
my-tool    a91c2d1  a91c2d1   71ef3b0    4m ago   install failed; will retry
other-cli  4a92f37  4a92f37   —          4m ago   synced (pull only)
```

If a remote can't be reached, `MAIN` shows the last known value. `INSTALLED` is the last commit whose install command succeeded on this machine. It doesn't prove that the binary on `PATH` matches it.

## Custom pull and install commands

Each tool can have its own commands. They run with `sh -c` inside the tool's repo:

```bash
tsync add --install "cargo install --path ."   # custom install command
tsync add --pull "stax rs"                     # custom pull command
tsync add --pull ""                            # back to the built-in git fast-forward
tsync add --pull-only                          # stop installing
```

> [!NOTE]
> Custom commands run in your real repo, but only when it's on a clean `main`. A pull command like `stax rs` has its usual side effects there, such as deleting merged branches.

## Run on a timer

`tsync sync` is safe to run on a schedule. The timer environment must have `git`, `cargo`, `make`, and any custom commands on `PATH`.

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

## Requirements

- [Rust](https://rustup.rs) 1.100+ to build, with `~/.cargo/bin` on your `PATH`
- `git`
- `sh`, plus whatever your install commands use (`make`, `cargo`, ...)

> [!NOTE]
> Install commands run code from the repo, and custom commands run as you. tsync never escalates privileges. Only register repos you trust.

## Troubleshooting

| Message | Fix |
|---|---|
| `no local repo recorded; run tsync add inside it` | The entry predates path tracking. Run `tsync add -y` inside the repo. |
| `no Git repo at <path>` | The repo moved or was deleted. Run `tsync add` in its new location, or `tsync remove <name>`. |
| `no remote in <path> points to <url>` | The repo's remote URL changed. Update `url` in `tools.toml`, or run `tsync remove <name>` and then `tsync add` inside the repo. |
| `skipped: on branch …` / `uncommitted changes` / `main has commits that aren't on origin` | Expected. Switch to a clean, pushed `main` and sync again. |
| `local main has diverged from origin/main` | Reconcile `main` by hand (rebase or reset), then sync. |
| `install failed; will retry` | Read the log path in the message, or the tail printed under it. The next sync retries. |
| `couldn't refresh; showing last known` | The remote was unreachable. Check the network or SSH access. |
| `another tsync sync is already running` | Wait for the other run, which may be a timer, to finish. |
| A custom command works in your shell but not on a timer | Timers don't load shell aliases or functions. Use real binary names, such as `stax` instead of `st`. |

## License

MIT. See `LICENSE`.
