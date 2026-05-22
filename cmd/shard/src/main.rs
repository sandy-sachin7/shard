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
        #[arg(long)]
        private: bool,
        #[arg(long, default_value = "flat")]
        db: String,
        #[arg(long, default_value = "zstd")]
        compression: String,
        #[arg(long, default_value = "fixed")]
        chunker: String,
        #[arg(long)]
        chunk_size: Option<u64>,
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
    /// Manage branches
    Branch {
        #[command(subcommand)]
        command: BranchCommands,
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
    /// Checkout a branch or commit
    Checkout {
        target: String,
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
    /// Recover from a crash using the write-ahead log
    Recover,
    /// Sync with peers via pubsub announcements
    Sync,
}

#[derive(Subcommand)]
enum BranchCommands {
    /// Create a new branch
    Create {
        name: String,
        /// Commit id to point to (defaults to HEAD)
        commit_id: Option<String>,
    },
    /// Delete a branch
    Delete { name: String },
    /// List all branches
    List,
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
        Commands::Init {
            private,
            db,
            compression,
            chunker,
            chunk_size,
        } => {
            let current_dir = env::current_dir()?;
            shard_core::init(&current_dir, db, compression, chunker, *chunk_size)?;
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
        Commands::Branch { command } => match command {
            BranchCommands::Create { name, commit_id } => {
                let current_dir = env::current_dir()?;
                shard_core::branch_create(&current_dir, name, commit_id.as_deref())?;
            }
            BranchCommands::Delete { name } => {
                let current_dir = env::current_dir()?;
                shard_core::branch_delete(&current_dir, name)?;
            }
            BranchCommands::List => {
                let current_dir = env::current_dir()?;
                shard_core::branch_list(&current_dir)?;
            }
        },
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
        Commands::Checkout { target, json } => {
            let current_dir = env::current_dir()?;
            shard_core::checkout(&current_dir, target, *json)?;
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
        Commands::Recover => {
            let current_dir = env::current_dir()?;
            shard_core::recover(&current_dir)?;
        }
        Commands::Sync => {
            let current_dir = env::current_dir()?;
            shard_core::sync(&current_dir).await?;
        }
    }

    Ok(())
}
