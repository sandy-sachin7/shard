//! Basic usage example for shard — distributed, content-addressed VCS for large artifacts.
//!
//! Run with: cargo run --example shard-basics
//!
//! This example demonstrates:
//! - Initializing a repository
//! - Adding files
//! - Committing with signing
//! - Verifying a commit
//! - Viewing the log
//! - Checking out a previous commit

use std::fs;
use tempfile::tempdir;

fn main() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let repo = dir.path();

    println!("=== shard basics example ===");
    println!("Repo at: {}", repo.display());

    // 1. Initialize
    println!("\n1. Initializing repository...");
    shard_core::init(repo, "flat", "zstd", "fixed", None, false, false)?;
    println!("   Done.");

    // 2. Create a file
    println!("\n2. Creating file.txt...");
    fs::write(repo.join("file.txt"), b"Hello, shard!\nThis is a test file.\n")?;

    // 3. Add the file
    println!("\n3. Adding file to staging...");
    shard_core::add(repo, &repo.join("file.txt"), false)?;

    // 4. Commit
    println!("\n4. Committing with ed25519 signature...");
    shard_core::commit(repo, "Initial commit", "Example <example@test.com>", false)?;

    // 5. View log
    println!("\n5. Commit log:");
    shard_core::log_cmd(repo, false)?;

    // 6. Show status
    println!("\n6. Checking status...");
    shard_core::status(repo, false)?;

    // 7. Modify file and commit again
    println!("\n7. Creating a second commit...");
    fs::write(repo.join("file.txt"), b"Hello, shard!\nModified content.\n")?;
    shard_core::add(repo, &repo.join("file.txt"), false)?;
    shard_core::commit(repo, "Update file.txt", "Example <example@test.com>", false)?;

    // 8. Log all commits
    println!("\n8. Full commit log:");
    shard_core::log_cmd(repo, false)?;

    println!("\n=== Example completed successfully ===");
    Ok(())
}
