use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use std::env;
use std::io;
use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(global = true, long, default_value = "plain", value_parser = ["plain", "json"])]
    log_format: String,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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
        #[arg(long)]
        json: bool,
        #[arg(long)]
        passphrase: Option<String>,
    },
    Add {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Commit {
        #[arg(short, long)]
        message: String,
        #[arg(long, default_value = "User <user@example.com>")]
        author: String,
        #[arg(long)]
        json: bool,
    },
    Verify {
        commit_id: String,
        #[arg(long)]
        json: bool,
    },
    Branch {
        #[command(subcommand)]
        command: BranchCommands,
    },
    Diff {
        commit_a: String,
        commit_b: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Peer {
        #[command(subcommand)]
        command: PeerCommands,
    },
    Key {
        #[command(subcommand)]
        command: KeyCommands,
    },
    Share {
        #[arg(long)]
        json: bool,
    },
    Pull {
        peer: String,
        commit_id: String,
        #[arg(long)]
        json: bool,
    },
    Push {
        peer: String,
        #[arg(long)]
        json: bool,
    },
    Log {
        #[arg(long)]
        json: bool,
    },
    Merge {
        branch: String,
        #[arg(short, long)]
        message: String,
        #[arg(long, default_value = "User <user@example.com>")]
        author: String,
        #[arg(long)]
        json: bool,
    },
    Checkout {
        target: String,
        #[arg(long)]
        json: bool,
    },
    Status {
        #[arg(long)]
        json: bool,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    Tag {
        #[command(subcommand)]
        command: TagCommands,
    },
    Health {
        #[arg(long)]
        json: bool,
    },
    Prune {
        #[arg(long)]
        json: bool,
    },
    Recover {
        #[arg(long)]
        json: bool,
    },
    Sync {
        #[arg(long)]
        json: bool,
    },
    Backup {
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Export {
        commit_id: String,
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Import {
        path: PathBuf,
        #[arg(short, long)]
        message: String,
        #[arg(long, default_value = "User <user@example.com>")]
        author: String,
        #[arg(long)]
        json: bool,
    },
    Restore {
        backup: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Relay {
        #[arg(long, default_value = "/ip4/0.0.0.0/tcp/0")]
        listen: String,
        #[arg(long)]
        json: bool,
    },
    Transfer {
        #[command(subcommand)]
        command: TransferCommands,
    },
    Serve {
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: String,
        #[arg(long)]
        json: bool,
    },
    Unlock {
        #[arg(long)]
        passphrase: String,
    },
    Completions {
        shell: String,
    },
}

#[derive(Subcommand)]
enum BranchCommands {
    Create {
        name: String,
        commit_id: Option<String>,
    },
    Delete {
        name: String,
    },
    List,
}

#[derive(Subcommand)]
enum TagCommands {
    Add { name: String, commit_id: String },
    List,
}

#[derive(Subcommand)]
enum ConfigCommands {
    Get { key: Option<String> },
    Set { key: String, value: String },
}

#[derive(Subcommand)]
enum KeyCommands {
    Rotate,
    List {
        #[arg(long)]
        json: bool,
    },
    Verify {
        #[arg(long)]
        json: bool,
    },
    AddAuthorized {
        public_key_hex: String,
    },
    RemoveAuthorized {
        public_key_hex: String,
    },
    ListAuthorized {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TransferCommands {
    List {
        #[arg(long)]
        json: bool,
    },
    Remove {
        commit_id: String,
    },
}

#[derive(Subcommand)]
enum PeerCommands {
    Add {
        multiaddr: String,
        #[arg(long)]
        public_key: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.log_format.as_str() {
        "json" => {
            tracing_subscriber::fmt().json().with_ansi(false).init();
        }
        _ => {
            tracing_subscriber::fmt()
                .with_ansi(false)
                .without_time()
                .with_target(false)
                .with_level(false)
                .init();
        }
    }

    match &cli.command {
        Commands::Init {
            private,
            db,
            compression,
            chunker,
            chunk_size,
            json,
            passphrase,
        } => {
            let current_dir = env::current_dir()?;
            let pass = passphrase.as_deref().unwrap_or("");
            shard_core::init_with_passphrase(
                &current_dir,
                db,
                compression,
                chunker,
                *chunk_size,
                *private,
                *json,
                pass,
            )?;
        }
        Commands::Add { path, json } => {
            let current_dir = env::current_dir()?;
            shard_core::add(&current_dir, path, *json)?;
        }
        Commands::Commit {
            message,
            author,
            json,
        } => {
            let current_dir = env::current_dir()?;
            shard_core::commit(&current_dir, message, author, *json)?;
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
        Commands::Diff {
            commit_a,
            commit_b,
            json,
        } => {
            let current_dir = env::current_dir()?;
            shard_core::diff(&current_dir, commit_a, commit_b.as_deref(), *json)?;
        }
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
            KeyCommands::AddAuthorized { public_key_hex } => {
                let current_dir = env::current_dir()?;
                let shard_dir = current_dir.join(".shard");
                shard_core::add_authorized_key(&shard_dir, public_key_hex)?;
            }
            KeyCommands::RemoveAuthorized { public_key_hex } => {
                let current_dir = env::current_dir()?;
                let shard_dir = current_dir.join(".shard");
                shard_core::remove_authorized_key(&shard_dir, public_key_hex)?;
            }
            KeyCommands::ListAuthorized { json } => {
                let current_dir = env::current_dir()?;
                let shard_dir = current_dir.join(".shard");
                shard_core::list_authorized_keys(&shard_dir, *json)?;
            }
        },
        Commands::Peer { command } => match command {
            PeerCommands::Add {
                multiaddr,
                public_key,
                json,
            } => {
                let current_dir = env::current_dir()?;
                shard_core::peer_add(&current_dir, multiaddr, *json)?;
                if let Some(pk) = public_key {
                    let shard_dir = current_dir.join(".shard");
                    shard_core::add_authorized_key(&shard_dir, pk)?;
                }
            }
        },
        Commands::Share { json } => {
            let current_dir = env::current_dir()?;
            shard_core::share(&current_dir, *json).await?;
        }
        Commands::Pull {
            peer,
            commit_id,
            json,
        } => {
            let current_dir = env::current_dir()?;
            shard_core::pull(&current_dir, peer, commit_id, *json).await?;
        }
        Commands::Push { peer, json } => {
            let current_dir = env::current_dir()?;
            shard_core::push(&current_dir, peer, *json).await?;
        }
        Commands::Log { json } => {
            let current_dir = env::current_dir()?;
            shard_core::log_cmd(&current_dir, *json)?;
        }
        Commands::Merge {
            branch,
            message,
            author,
            json,
        } => {
            let current_dir = env::current_dir()?;
            shard_core::merge(&current_dir, branch, message, author, *json)?;
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
        Commands::Health { json } => {
            let current_dir = env::current_dir()?;
            shard_core::health(&current_dir, *json)?;
        }
        Commands::Prune { json } => {
            let current_dir = env::current_dir()?;
            shard_core::prune(&current_dir, *json)?;
        }
        Commands::Recover { json } => {
            let current_dir = env::current_dir()?;
            shard_core::recover(&current_dir, *json)?;
        }
        Commands::Sync { json } => {
            let current_dir = env::current_dir()?;
            shard_core::sync(&current_dir, *json).await?;
        }
        Commands::Backup { output, json } => {
            let current_dir = env::current_dir()?;
            shard_core::backup(&current_dir, output, *json)?;
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
            json,
        } => {
            let current_dir = env::current_dir()?;
            shard_core::import(&current_dir, path, message, author, *json)?;
        }
        Commands::Restore { backup, json } => {
            let current_dir = env::current_dir()?;
            shard_core::restore(&current_dir, backup, *json)?;
        }
        Commands::Relay { listen, json } => {
            shard_core::relay(listen, *json).await?;
        }
        Commands::Transfer { command } => match command {
            TransferCommands::List { json } => {
                let current_dir = env::current_dir()?;
                shard_core::transfer_list(&current_dir, *json)?;
            }
            TransferCommands::Remove { commit_id } => {
                let current_dir = env::current_dir()?;
                shard_core::transfer_remove(&current_dir, commit_id)?;
            }
        },
        Commands::Serve { addr, json } => {
            let current_dir = env::current_dir()?;
            shard_core::api::serve(&current_dir, addr, *json).await?;
        }
        Commands::Unlock { passphrase } => {
            shard_core::cache_passphrase(passphrase)?;
            eprintln!("Passphrase cached for this session.");
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            let shell: Shell = shell.parse().map_err(|e| anyhow::anyhow!("{}", e))?;
            generate(shell, &mut cmd, "shard", &mut io::stdout());
        }
    }

    Ok(())
}
