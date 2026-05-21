use anyhow::Result;
use clap::{Parser, Subcommand};
use std::env;

use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new Shard repository
    Init {
        /// Initialize as a private repository
        #[arg(long)]
        private: bool,
    },
    /// Add a file to the staging area
    Add { path: PathBuf },
    /// Record changes to the repository
    Commit {
        #[arg(short, long)]
        message: String,
        #[arg(long, default_value = "User <user@example.com>")]
        author: String,
    },
    /// Verify the integrity of a commit
    Verify {
        commit_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Manage peers
    Peer {
        #[command(subcommand)]
        command: PeerCommands,
    },
    /// Share the repository with the network
    Share,
    /// Pull a commit from a peer
    Pull { peer: String, commit_id: String },
    /// Show commit log
    Log {
        #[arg(long)]
        json: bool,
    },
    /// Checkout files from a commit
    Checkout {
        commit_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Show working tree status
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Get or set configuration values
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Manage tags
    Tag {
        #[command(subcommand)]
        command: TagCommands,
    },
    /// Prune unreachable objects
    Prune,
}

#[derive(Subcommand)]
enum TagCommands {
    /// Add a tag pointing to a commit
    Add { name: String, commit_id: String },
    /// List all tags
    List,
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Get a config value (or all if no key given)
    Get { key: Option<String> },
    /// Set a config value
    Set { key: String, value: String },
}

#[derive(Subcommand)]
enum PeerCommands {
    /// Add a peer
    Add { multiaddr: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Init { private } => {
            let current_dir = env::current_dir()?;
            shard_core::init(&current_dir)?;
            if *private {
                shard_core::config_set(&current_dir, "private", "true")?;
            }
        }
        Commands::Add { path } => {
            let current_dir = env::current_dir()?;
            shard_core::add(&current_dir, path)?;
        }
        Commands::Commit { message, author } => {
            let current_dir = env::current_dir()?;
            shard_core::commit(&current_dir, message, author)?;
        }
        Commands::Verify { commit_id, json } => {
            let current_dir = env::current_dir()?;
            shard_core::verify(&current_dir, commit_id, *json)?;
        }
        Commands::Peer { command } => match command {
            PeerCommands::Add { multiaddr } => {
                let current_dir = env::current_dir()?;
                shard_core::peer_add(&current_dir, multiaddr)?;
            }
        },
        Commands::Share => {
            let current_dir = env::current_dir()?;
            shard_core::share(&current_dir).await?;
        }
        Commands::Pull { peer, commit_id } => {
            let current_dir = env::current_dir()?;
            shard_core::pull(&current_dir, peer, commit_id).await?;
        }
        Commands::Log { json } => {
            let current_dir = env::current_dir()?;
            shard_core::log_cmd(&current_dir, *json)?;
        }
        Commands::Checkout { commit_id, json } => {
            let current_dir = env::current_dir()?;
            shard_core::checkout(&current_dir, commit_id, *json)?;
        }
        Commands::Status { json } => {
            let current_dir = env::current_dir()?;
            shard_core::status(&current_dir, *json)?;
        }
        Commands::Tag { command } => match command {
            TagCommands::Add { name, commit_id } => {
                let current_dir = env::current_dir()?;
                shard_core::tag_add(&current_dir, name, commit_id)?;
            }
            TagCommands::List => {
                let current_dir = env::current_dir()?;
                shard_core::tag_list(&current_dir)?;
            }
        },
        Commands::Config { command } => match command {
            ConfigCommands::Get { key } => {
                let current_dir = env::current_dir()?;
                shard_core::config_get(&current_dir, key.as_deref())?;
            }
            ConfigCommands::Set { key, value } => {
                let current_dir = env::current_dir()?;
                shard_core::config_set(&current_dir, key, value)?;
            }
        },
        Commands::Prune => {
            let current_dir = env::current_dir()?;
            shard_core::prune(&current_dir)?;
        }
    }

    Ok(())
}
