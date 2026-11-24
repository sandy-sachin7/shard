//! Basic usage example for shard — distributed, content-addressed VCS for large artifacts.
//!
//! Run with: `cargo run -p shard-core --example shard-basics`
//!
//! This example demonstrates:
//! - Initializing a repository
//! - Adding files
//! - Committing with Ed25519 signing
//! - Verifying a commit (hashes, Merkle roots, and signature)
//! - Viewing the log and status
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
    let v1_content = b"Hello, shard!\nThis is a test file.\n";
    fs::write(repo.join("file.txt"), v1_content)?;

    // 3. Add the file
    println!("\n3. Adding file to staging...");
    shard_core::add(repo, &repo.join("file.txt"), false)?;

    // 4. Commit
    println!("\n4. Committing with Ed25519 signature...");
    let first_commit =
        shard_core::commit(repo, "Initial commit", "Example <example@test.com>", false)?;

    // 5. Verify commit integrity and signature
    println!("\n5. Verifying first commit ({})...", &first_commit[..12]);
    shard_core::verify(repo, &first_commit, false)?;

    // 6. View log & status
    println!("\n6. Commit log:");
    shard_core::log_cmd(repo, false)?;
    println!("7. Checking status...");
    shard_core::status(repo, false, false)?;

    // 8. Modify file and commit again
    println!("\n8. Creating a second commit...");
    fs::write(repo.join("file.txt"), b"Hello, shard!\nModified content.\n")?;
    shard_core::add(repo, &repo.join("file.txt"), false)?;
    let second_commit =
        shard_core::commit(repo, "Update file.txt", "Example <example@test.com>", false)?;
    shard_core::verify(repo, &second_commit, false)?;

    // 9. Checkout the first commit and confirm contents are restored
    println!(
        "\n9. Checking out first commit ({})...",
        &first_commit[..12]
    );
    shard_core::checkout(repo, &first_commit, false)?;
    let restored = fs::read(repo.join("file.txt"))?;
    anyhow::ensure!(
        restored == v1_content,
        "restored file.txt did not match v1 content"
    );
    println!("   Restored file.txt verified byte-for-byte.");

    println!("\n=== Example completed successfully ===");
    Ok(())
}
