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
    /// Manage signing keys (rotate, list, verify)
    Key {
        #[command(subcommand)]
        command: KeyCommands,
    },
    /// Share the repository with the network
    Share,
    /// Pull a commit from a peer
    Pull { peer: String, commit_id: String },
    /// Push all reachable objects to a peer
    Push { peer: String },
    /// Show commit log
    Log {
        #[arg(long)]
        json: bool,
    },
    /// Merge a branch into the current branch
    Merge {
        branch: String,
        #[arg(short, long)]
        message: String,
        #[arg(long, default_value = "User <user@example.com>")]
        author: String,
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
    /// Create a backup archive of the .shard directory
    Backup {
        /// Output path for the backup tar.gz file
        output: PathBuf,
    },
    /// Export commit files to a directory
    Export {
        commit_id: String,
        /// Output directory for exported files
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Import files from a directory as a new commit
    Import {
        /// Source directory to import from
        path: PathBuf,
        #[arg(short, long)]
        message: String,
        #[arg(long, default_value = "User <user@example.com>")]
        author: String,
    },
    /// Restore a repository from a backup archive
    Restore {
        /// Path to the backup tar.gz file
        backup: PathBuf,
    },
    /// Start a circuit relay v2 server for NAT traversal
    Relay {
        /// Listen address (default: /ip4/0.0.0.0/tcp/0)
        #[arg(long, default_value = "/ip4/0.0.0.0/tcp/0")]
        listen: String,
    },
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
enum KeyCommands {
    /// Rotate the signing key (archive old, generate new, sign rotation)
    Rotate,
    /// List all keys in the keychain
    List {
        #[arg(long)]
        json: bool,
    },
    /// Verify the integrity of the keychain
    Verify {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum PeerCommands {
    /// Add a peer
    Add {
        multiaddr: String,
        /// ed25519 public key (64 hex chars) to authorize
        #[arg(long)]
        public_key: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_level(false)
        .init();

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
            shard_core::init(
                &current_dir,
                db,
                compression,
                chunker,
                *chunk_size,
                *private,
            )?;
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
        Commands::Key { command } => match command {
            KeyCommands::Rotate => {
                let current_dir = env::current_dir()?;
                shard_core::key_rotate(&current_dir)?;
            }
            KeyCommands::List { json } => {
                let current_dir = env::current_dir()?;
                shard_core::key_list(&current_dir, *json)?;
            }
            KeyCommands::Verify { json } => {
                let current_dir = env::current_dir()?;
                shard_core::key_verify(&current_dir, *json)?;
            }
        },
        Commands::Peer { command } => match command {
            PeerCommands::Add {
                multiaddr,
                public_key,
            } => {
                let current_dir = env::current_dir()?;
                shard_core::peer_add(&current_dir, multiaddr)?;
                if let Some(pk) = public_key {
                    let shard_dir = current_dir.join(".shard");
                    shard_core::add_authorized_key(&shard_dir, pk)?;
                }
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
        Commands::Push { peer } => {
            let current_dir = env::current_dir()?;
            shard_core::push(&current_dir, peer).await?;
        }
        Commands::Log { json } => {
            let current_dir = env::current_dir()?;
            shard_core::log_cmd(&current_dir, *json)?;
        }
        Commands::Merge {
            branch,
            message,
            author,
        } => {
            let current_dir = env::current_dir()?;
            shard_core::merge(&current_dir, branch, message, author)?;
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
        Commands::Backup { output } => {
            let current_dir = env::current_dir()?;
            shard_core::backup(&current_dir, output)?;
        }
        Commands::Export {
            commit_id,
            output,
            json,
        } => {
            let current_dir = env::current_dir()?;
            shard_core::export(&current_dir, commit_id, output, *json)?;
        }
        Commands::Import {
            path,
            message,
            author,
        } => {
            let current_dir = env::current_dir()?;
            shard_core::import(&current_dir, path, message, author)?;
        }
        Commands::Restore { backup } => {
            let current_dir = env::current_dir()?;
            shard_core::restore(&current_dir, backup)?;
        }
        Commands::Relay { listen } => {
            shard_core::relay(listen).await?;
        }
    }

    Ok(())
}
