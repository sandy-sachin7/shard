use clap::{Parser, Subcommand};
use anyhow::Result;
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
    Init,
    /// Add a file to the staging area
    Add {
        path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Init => {
            let current_dir = env::current_dir()?;
            shard_core::init(&current_dir)?;
        }
        Commands::Add { path } => {
            let current_dir = env::current_dir()?;
            shard_core::add(&current_dir, path)?;
        }
    }

    Ok(())
}
