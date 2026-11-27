mod add;
mod apply;
mod config;
mod git;
mod manifest;
mod paths;
mod remove;
mod state;
mod status;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::paths::Paths;

#[derive(Parser, Debug)]
#[command(
    name = "toolsync",
    version,
    about = "Keep personal Rust tools synced across machines"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Enroll this machine by cloning the shared manifest repo
    Init {
        /// Git URL of the shared manifest repo
        manifest_url: String,
    },
    /// Register the current repo's remote `main` in the local manifest
    Add {
        /// Run `make install-release` after each new commit on main
        #[arg(long, conflicts_with = "pull_only")]
        install: bool,
        /// Explicitly switch an existing entry back to pull only
        #[arg(long)]
        pull_only: bool,
        /// Remote to register
        #[arg(long, default_value = "origin")]
        remote: String,
        /// Entry name (defaults to the repo name from the remote URL)
        #[arg(long)]
        name: Option<String>,
    },
    /// Remove a tool from the local manifest
    Remove {
        /// Entry name (defaults to the current repo's entry)
        name: Option<String>,
        /// Remote used to find the current repo's entry
        #[arg(long, default_value = "origin")]
        remote: String,
        /// Also delete this machine's managed checkout, log, and state
        #[arg(long)]
        purge: bool,
    },
    /// Commit and push local manifest changes
    Publish {
        /// Commit message
        #[arg(short, long, default_value = "Update toolsync manifest")]
        message: String,
    },
    /// Refresh the manifest, sync managed checkouts, and install as needed
    Apply,
    /// Show this machine's last known state (no network access)
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::resolve()?;

    match cli.command {
        Command::Init { manifest_url } => manifest::init(&paths, &manifest_url),
        Command::Add {
            install,
            pull_only,
            remote,
            name,
        } => {
            let request = if install {
                add::Request::Install
            } else if pull_only {
                add::Request::PullOnly
            } else {
                add::Request::Default
            };
            add::run(&paths, &remote, request, name)
        }
        Command::Remove {
            name,
            remote,
            purge,
        } => remove::run(&paths, name, &remote, purge),
        Command::Publish { message } => manifest::publish(&paths, &message),
        Command::Apply => apply::run(&paths),
        Command::Status => status::run(&paths),
    }
}
