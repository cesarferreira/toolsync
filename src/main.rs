mod add;
mod git;
mod paths;
mod progress;
mod project;
mod remove;
mod settings;
mod state;
mod status;
mod sync;
mod tools;
mod ui;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

use crate::paths::Paths;
use crate::project::IconStyle;

#[derive(Parser, Debug)]
#[command(
    name = "tsync",
    version,
    about = "Keep personal Rust tools synced across machines",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// With no command, runs `sync`
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    sync: SyncArgs,

    /// Save a feature preference, e.g. `--feature icons off`
    #[arg(
        long,
        num_args = 2,
        value_names = ["FEATURE", "ON|OFF"],
        action = clap::ArgAction::Set,
        conflicts_with_all = ["jobs", "pull", "icons"]
    )]
    feature: Option<Vec<String>>,
}

#[derive(clap::Args, Debug, Default)]
struct SyncArgs {
    /// How many tools to sync at once (default: all)
    #[arg(short, long, value_name = "N")]
    jobs: Option<usize>,
    /// Only pull; skip installs this run (the next plain sync installs what's pending)
    #[arg(long)]
    pull: bool,
    /// Override project icons for this run
    #[arg(long, value_enum)]
    icons: Option<IconStyle>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Register the current repo's remote `main` on this machine
    Add {
        /// Install after each new commit on main [default command: make install-release]
        #[arg(
            long,
            value_name = "CMD",
            num_args = 0..=1,
            default_missing_value = tools::DEFAULT_INSTALL,
            conflicts_with = "pull_only"
        )]
        install: Option<String>,
        /// Stop installing an existing entry
        #[arg(long)]
        pull_only: bool,
        /// Command that updates the repo (default: built-in git fast-forward; "" resets)
        #[arg(long, value_name = "CMD")]
        pull: Option<String>,
        /// Remote to register
        #[arg(long, default_value = "origin")]
        remote: String,
        /// Entry name (defaults to the repo name from the remote URL)
        #[arg(long)]
        name: Option<String>,
        /// Accept defaults instead of prompting
        #[arg(short, long)]
        yes: bool,
    },
    /// Stop tracking a tool on this machine (never touches the repo)
    Remove {
        /// Entry name (defaults to the current repo's entry)
        name: Option<String>,
        /// Remote used to find the current repo's entry
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Pull every registered tool from main and install as needed (the default)
    Sync(SyncArgs),
    /// Show each tool's state, checking remote main for all tools in parallel
    Status {
        /// Show the last known state without contacting remotes
        #[arg(long)]
        offline: bool,
        /// Override project icons for this run
        #[arg(long, value_enum)]
        icons: Option<IconStyle>,
    },
    /// Show where the tool list, settings, and local data live
    Config {
        /// Print only the tool list path, e.g. `$EDITOR "$(tsync config --path)"`
        #[arg(long)]
        path: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::resolve()?;

    if let Some(feature) = cli.feature {
        let enabled = match feature.as_slice() {
            [name, state] if name == "icons" && state == "on" => true,
            [name, state] if name == "icons" && state == "off" => false,
            _ => bail!("expected `--feature icons on` or `--feature icons off`"),
        };
        settings::set_icons(&paths.settings_file(), enabled)?;
        println!("Project icons {}", if enabled { "on" } else { "off" });
        return Ok(());
    }

    match cli.command.unwrap_or(Command::Sync(cli.sync)) {
        Command::Add {
            install,
            pull_only,
            pull,
            remote,
            name,
            yes,
        } => {
            let changes = add::Changes {
                install,
                pull_only,
                pull,
                yes,
            };
            add::run(&paths, &remote, name, changes)
        }
        Command::Remove { name, remote } => remove::run(&paths, name, &remote),
        Command::Sync(args) => {
            let icons = settings::icon_style(&paths.settings_file(), args.icons)?;
            if !sync::run(&paths, args.jobs, !args.pull, icons)? {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Status { offline, icons } => {
            let icons = settings::icon_style(&paths.settings_file(), icons)?;
            status::run(&paths, offline, icons)
        }
        Command::Config { path } => {
            if path {
                println!("{}", paths.tools_file().display());
            } else {
                let exists = |p: &std::path::Path| {
                    if p.exists() { "" } else { " (not created yet)" }
                };
                let rows = [
                    ("Tool list", paths.tools_file().clone()),
                    ("Settings", paths.settings_file()),
                    ("State", paths.state_file()),
                    ("Logs", paths.logs_dir()),
                ];
                for (label, p) in rows {
                    println!(
                        "{}  {}{}",
                        ui::bold(format!("{label:<9}")),
                        p.display(),
                        ui::dim(exists(&p))
                    );
                }
                if std::env::var_os("TOOLSYNC_HOME").is_some() {
                    println!("{}", ui::dim("Set by TOOLSYNC_HOME."));
                }
            }
            Ok(())
        }
    }
}
