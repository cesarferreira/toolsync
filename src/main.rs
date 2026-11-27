mod add;
mod git;
mod paths;
mod progress;
mod remove;
mod state;
mod status;
mod sync;
mod tools;
mod ui;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::paths::Paths;

#[derive(Parser, Debug)]
#[command(
    name = "tsync",
    version,
    about = "Keep personal Rust tools synced across machines"
)]
struct Cli {
    /// With no command, runs `sync`
    #[command(subcommand)]
    command: Option<Command>,
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
        /// Command that updates the checkout (default: built-in git fast-forward; "" resets)
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
    /// Stop tracking a tool and delete its managed checkout on this machine
    Remove {
        /// Entry name (defaults to the current repo's entry)
        name: Option<String>,
        /// Remote used to find the current repo's entry
        #[arg(long, default_value = "origin")]
        remote: String,
    },
    /// Pull every registered tool from main and install as needed (the default)
    Sync {
        /// How many tools to sync at once (default: all)
        #[arg(short, long, value_name = "N")]
        jobs: Option<usize>,
    },
    /// Show this machine's last known state (no network access)
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::resolve()?;

    match cli.command.unwrap_or(Command::Sync { jobs: None }) {
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
        Command::Sync { jobs } => {
            if !sync::run(&paths, jobs)? {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Status => status::run(&paths),
    }
}
