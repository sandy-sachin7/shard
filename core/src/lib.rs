use std::path::Path;
use anyhow::Result;
use std::fs;
use shard_crypto::KeyPair;

pub fn init(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if shard_dir.exists() {
        anyhow::bail!("Shard repository already initialized");
    }
    fs::create_dir_all(shard_dir.join("objects"))?;
    fs::create_dir_all(shard_dir.join("keys"))?;

    let keys = KeyPair::generate();
    keys.save(&shard_dir.join("keys"))?;

    println!("Initialized empty Shard repository in {}", shard_dir.display());
    Ok(())
}
