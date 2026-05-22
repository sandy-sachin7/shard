pub mod chunker;
pub mod commit;
pub mod index;
pub mod manifest;
pub mod store;

use crate::chunker::Chunker;
use crate::commit::Commit;
use crate::index::Index;
use crate::manifest::FileManifest;
use crate::store::Store;
use anyhow::Result;
use ed25519_dalek::{Signer, Verifier};
use serde::Serialize;
use shard_crypto::KeyPair;
use shard_net::libp2p::futures::StreamExt;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn init(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if shard_dir.exists() {
        anyhow::bail!("Shard repository already initialized");
    }
    fs::create_dir_all(shard_dir.join("objects"))?;
    fs::create_dir_all(shard_dir.join("keys"))?;

    let keys = KeyPair::generate();
    keys.save(&shard_dir.join("keys"))?;

    // Generate a deterministic repo identity from the public key
    // (same key = same repo_id, so clones share the gossipsub topic)
    let pubkey = fs::read(shard_dir.join("keys/public.key"))?;
    let repo_id = blake3::hash(&pubkey).to_hex().to_string();
    let mut config = load_config(&shard_dir)?;
    config.insert("repo_id".to_string(), repo_id);
    save_config(&shard_dir, &config)?;

    println!(
        "Initialized empty Shard repository in {}",
        shard_dir.display()
    );
    Ok(())
}

pub fn add(path: &Path, file_path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let store = Store::new(&shard_dir);
    let mut index = Index::load(&shard_dir.join("index"))?;

    let file = fs::File::open(file_path)?;
    let mut chunker = Chunker::new(file);
    let mut chunk_hashes = Vec::new();
    let mut total_size = 0;

    while let Some(chunk) = chunker.next_chunk()? {
        store.put_chunk(&chunk)?;
        chunk_hashes.push(chunk.hash.to_hex().to_string());
        total_size += chunk.data.len() as u64;
    }

    let filename = file_path.file_name().unwrap().to_string_lossy().to_string();
    let manifest = FileManifest {
        name: filename.clone(),
        size: total_size,
        chunks: chunk_hashes,
        content_type: None,
    };

    index.files.insert(filename.clone(), manifest);
    index.save(&shard_dir.join("index"))?;

    println!("Added {} ({})", filename, total_size);
    Ok(())
}

pub fn commit(path: &Path, message: &str, author: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let store = Store::new(&shard_dir);
    let mut index = Index::load(&shard_dir.join("index"))?;

    if index.files.is_empty() {
        anyhow::bail!("Nothing to commit");
    }

    // 1. Store manifests
    let mut manifest_ids = Vec::new();
    for manifest in index.files.values() {
        // Canonical JSON
        let json = serde_json::to_vec(manifest)?;
        let hash = blake3::hash(&json);
        let chunk = crate::chunker::Chunk {
            hash,
            data: json,
            offset: 0,
        };
        store.put_chunk(&chunk)?;
        manifest_ids.push(hash.to_hex().to_string());
    }
    manifest_ids.sort(); // Canonical order

    // 2. Get parent
    let head_path = shard_dir.join("HEAD");
    let mut parents = Vec::new();
    if head_path.exists() {
        let head = fs::read_to_string(&head_path)?;
        parents.push(head.trim().to_string());
    }

    // 3. Create commit
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let keys = KeyPair::load(&shard_dir.join("keys"))?;
    let public_key_hex = hex::encode(keys.verifying_key.to_bytes());
    let mut commit = Commit {
        commit_id: String::new(),
        parents,
        manifests: manifest_ids,
        author: author.to_string(),
        message: message.to_string(),
        timestamp,
        public_key: Some(public_key_hex),
        signature: None,
    };

    // 4. Sign
    let signing_key = keys.signing_key;
    let json_unsigned = serde_json::to_vec(&commit)?;
    let signature = signing_key.sign(&json_unsigned);
    commit.signature = Some(hex::encode(signature.to_bytes()));

    // 5. Store commit
    let json_final = serde_json::to_vec(&commit)?;
    let hash = blake3::hash(&json_final);
    let chunk = crate::chunker::Chunk {
        hash,
        data: json_final,
        offset: 0,
    };
    store.put_chunk(&chunk)?;

    // 6. Update HEAD
    let commit_id = hash.to_hex().to_string();
    fs::write(head_path, &commit_id)?;

    // 7. Clear index
    index.files.clear();
    index.save(&shard_dir.join("index"))?;

    println!("Committed {} ({})", commit_id, message);
    Ok(())
}

pub fn verify(path: &Path, commit_id: &str, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let store = Store::new(&shard_dir);

    let prefix = &commit_id[..2];
    let obj_path = shard_dir.join("objects").join(prefix).join(commit_id);

    if !obj_path.exists() {
        anyhow::bail!("Commit object not found: {}", commit_id);
    }

    let data = fs::read(obj_path)?;
    let commit: Commit = serde_json::from_slice(&data)?;

    let mut sig_verified = false;
    let mut files_checked = 0u64;

    if let Some(sig_hex) = &commit.signature {
        let verifying_key = if let Some(pk_hex) = &commit.public_key {
            let pk_bytes = hex::decode(pk_hex)?;
            ed25519_dalek::VerifyingKey::from_bytes(pk_bytes.as_slice().try_into()?)?
        } else {
            let pub_key_path = shard_dir.join("keys/public.key");
            let pub_bytes = fs::read(pub_key_path)?;
            ed25519_dalek::VerifyingKey::from_bytes(pub_bytes.as_slice().try_into()?)?
        };

        let mut unsigned_commit = commit.clone();
        unsigned_commit.signature = None;
        let json_unsigned = serde_json::to_vec(&unsigned_commit)?;

        let sig_bytes = hex::decode(sig_hex)?;
        let signature = ed25519_dalek::Signature::from_bytes(sig_bytes.as_slice().try_into()?);

        verifying_key.verify(&json_unsigned, &signature)?;
        sig_verified = true;
        if !json {
            println!("Signature verified.");
        }
    } else if !json {
        println!("Warning: Commit is unsigned.");
    }

    for manifest_id in &commit.manifests {
        let manifest_data = store.get_chunk(manifest_id)?;
        let hash = blake3::hash(&manifest_data);
        if hash.to_hex().to_string() != *manifest_id {
            anyhow::bail!("Manifest hash mismatch: {}", manifest_id);
        }

        let manifest: FileManifest = serde_json::from_slice(&manifest_data)?;
        if !json {
            println!("Verifying file: {}", manifest.name);
        }

        for chunk_id in &manifest.chunks {
            let chunk_data = store.get_chunk(chunk_id)?;
            let hash = blake3::hash(&chunk_data);
            if hash.to_hex().to_string() != *chunk_id {
                anyhow::bail!("Chunk hash mismatch: {}", chunk_id);
            }
        }
        files_checked += 1;
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "commit_id": commit_id,
                "verified": true,
                "signature_verified": sig_verified,
                "files_checked": files_checked,
            }))?
        );
    } else {
        println!("Verification successful.");
    }
    Ok(())
}

fn load_commit(shard_dir: &Path, commit_id: &str) -> Result<Commit> {
    let prefix = &commit_id[..2];
    let path = shard_dir.join("objects").join(prefix).join(commit_id);
    let data = fs::read(path)?;
    let mut commit: Commit = serde_json::from_slice(&data)?;
    commit.commit_id = commit_id.to_string();
    Ok(commit)
}

#[derive(Serialize)]
pub struct LogEntry {
    pub commit_id: String,
    pub parents: Vec<String>,
    pub manifests: Vec<String>,
    pub author: String,
    pub message: String,
    pub timestamp: u64,
    pub signature: Option<String>,
}

impl From<Commit> for LogEntry {
    fn from(c: Commit) -> Self {
        LogEntry {
            commit_id: c.commit_id,
            parents: c.parents,
            manifests: c.manifests,
            author: c.author,
            message: c.message,
            timestamp: c.timestamp,
            signature: c.signature,
        }
    }
}

pub fn log_cmd(path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let head_path = shard_dir.join("HEAD");
    if !head_path.exists() {
        anyhow::bail!("No commits yet");
    }

    let head = fs::read_to_string(&head_path)?;
    let head = head.trim();

    let mut entries: Vec<LogEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![head.to_string()];

    while let Some(cid) = stack.pop() {
        if !seen.insert(cid.clone()) {
            continue;
        }
        let commit = load_commit(&shard_dir, &cid)?;
        for parent in &commit.parents {
            stack.push(parent.clone());
        }
        entries.push(commit.into());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        for entry in &entries {
            let ts = {
                let secs = entry.timestamp as i64;
                let tm = time::OffsetDateTime::from_unix_timestamp(secs)
                    .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
                tm.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_else(|_| entry.timestamp.to_string())
            };
            println!("commit {}", entry.commit_id);
            if !entry.parents.is_empty() {
                println!("parents: {}", entry.parents.join(" "));
            }
            println!("author: {}", entry.author);
            println!("date:   {}", ts);
            println!();
            for line in entry.message.lines() {
                println!("    {}", line);
            }
            println!();
        }
    }

    Ok(())
}

pub fn checkout(path: &Path, commit_id: &str, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let store = Store::new(&shard_dir);
    let commit = load_commit(&shard_dir, commit_id)?;
    let mut files = Vec::new();

    for manifest_id in &commit.manifests {
        let data = store.get_chunk(manifest_id)?;
        let hash = blake3::hash(&data);
        if hash.to_hex().to_string() != *manifest_id {
            anyhow::bail!("Manifest hash mismatch: {}", manifest_id);
        }
        let manifest: FileManifest = serde_json::from_slice(&data)?;
        if !json {
            println!("Checking out file: {}", manifest.name);
        }

        let mut file_data = Vec::new();
        for chunk_id in &manifest.chunks {
            let chunk_data = store.get_chunk(chunk_id)?;
            file_data.extend_from_slice(&chunk_data);
        }
        fs::write(path.join(&manifest.name), file_data)?;
        if !json {
            println!("  -> {}", manifest.name);
        }
        files.push(manifest.name);
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "commit_id": commit_id,
                "files": files,
            }))?
        );
    } else {
        println!("Checkout complete.");
    }
    Ok(())
}

pub fn status(path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let head_path = shard_dir.join("HEAD");
    let mut commit_id: Option<String> = None;
    if head_path.exists() {
        let head = fs::read_to_string(&head_path)?;
        commit_id = Some(head.trim().to_string());
        if !json {
            println!("On commit: {}", commit_id.as_ref().unwrap());
        }
    } else if !json {
        println!("No commits yet.");
    }

    let index = Index::load(&shard_dir.join("index"))?;
    let staged: Vec<String> = index.files.keys().cloned().collect();
    if !json {
        if staged.is_empty() {
            println!("Nothing staged.");
        } else {
            println!("\nStaged files:");
            for name in &staged {
                println!("  {} (to be committed)", name);
            }
        }
    }

    let mut deleted = Vec::new();
    let tracked_names: std::collections::HashSet<String> = if let Some(head) = &commit_id {
        let mut names = std::collections::HashSet::new();
        if let Ok(commit) = load_commit(&shard_dir, head) {
            let store = Store::new(&shard_dir);
            for manifest_id in &commit.manifests {
                if let Ok(data) = store.get_chunk(manifest_id) {
                    if let Ok(manifest) = serde_json::from_slice::<FileManifest>(&data) {
                        let file_path = path.join(&manifest.name);
                        if !file_path.exists() {
                            deleted.push(manifest.name.clone());
                        }
                        names.insert(manifest.name);
                    }
                }
            }
        }
        names
    } else {
        std::collections::HashSet::new()
    };

    if !json && !deleted.is_empty() {
        println!("\nDeleted files:");
        for name in &deleted {
            println!("  {} (deleted)", name);
        }
    }

    let mut untracked = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(ftype) = entry.file_type() {
                if ftype.is_file() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !name.starts_with('.')
                        && !index.files.contains_key(&name)
                        && !tracked_names.contains(&name)
                    {
                        untracked.push(name);
                    }
                }
            }
        }
    }
    if !json && !untracked.is_empty() {
        println!("\nUntracked files:");
        for name in &untracked {
            println!("  {}", name);
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "commit": commit_id,
                "staged": staged,
                "deleted": deleted,
                "untracked": untracked,
            }))?
        );
    }

    Ok(())
}

fn load_config(shard_dir: &Path) -> Result<std::collections::BTreeMap<String, String>> {
    let config_path = shard_dir.join("config.json");
    if config_path.exists() {
        let data = fs::read(&config_path)?;
        Ok(serde_json::from_slice(&data)?)
    } else {
        Ok(std::collections::BTreeMap::new())
    }
}

fn save_config(
    shard_dir: &Path,
    config: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let data = serde_json::to_string_pretty(config)?;
    fs::write(shard_dir.join("config.json"), data)?;
    Ok(())
}

pub fn config_get(path: &Path, key: Option<&str>) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }
    let config = load_config(&shard_dir)?;
    if let Some(key) = key {
        match config.get(key) {
            Some(value) => println!("{} = {}", key, value),
            None => anyhow::bail!("config key not found: {}", key),
        }
    } else {
        for (k, v) in &config {
            println!("{} = {}", k, v);
        }
    }
    Ok(())
}

pub fn config_set(path: &Path, key: &str, value: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }
    let mut config = load_config(&shard_dir)?;
    config.insert(key.to_string(), value.to_string());
    save_config(&shard_dir, &config)?;
    println!("{} = {}", key, value);
    Ok(())
}

fn load_tags(shard_dir: &Path) -> Result<std::collections::BTreeMap<String, String>> {
    let tags_path = shard_dir.join("tags.json");
    if tags_path.exists() {
        let data = fs::read(&tags_path)?;
        Ok(serde_json::from_slice(&data)?)
    } else {
        Ok(std::collections::BTreeMap::new())
    }
}

fn save_tags(shard_dir: &Path, tags: &std::collections::BTreeMap<String, String>) -> Result<()> {
    let data = serde_json::to_string_pretty(tags)?;
    fs::write(shard_dir.join("tags.json"), data)?;
    Ok(())
}

pub fn tag_add(path: &Path, name: &str, commit_id: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }
    // Verify commit exists
    load_commit(&shard_dir, commit_id)?;
    let mut tags = load_tags(&shard_dir)?;
    tags.insert(name.to_string(), commit_id.to_string());
    save_tags(&shard_dir, &tags)?;
    println!("Tagged '{}' -> {}", name, commit_id);
    Ok(())
}

pub fn tag_list(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }
    let tags = load_tags(&shard_dir)?;
    if tags.is_empty() {
        println!("No tags.");
    } else {
        for (name, commit_id) in &tags {
            println!("{} -> {}", name, commit_id);
        }
    }
    Ok(())
}

fn collect_reachable(
    store: &Store,
    shard_dir: &Path,
    commit_id: &str,
    seen_commits: &mut std::collections::HashSet<String>,
    reachable: &mut std::collections::HashSet<String>,
) -> Result<()> {
    if !seen_commits.insert(commit_id.to_string()) {
        return Ok(());
    }

    reachable.insert(commit_id.to_string());

    let commit = match load_commit(shard_dir, commit_id) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    for manifest_id in &commit.manifests {
        reachable.insert(manifest_id.clone());

        if let Ok(data) = store.get_chunk(manifest_id) {
            if let Ok(manifest) = serde_json::from_slice::<FileManifest>(&data) {
                for chunk_id in &manifest.chunks {
                    reachable.insert(chunk_id.clone());
                }
            }
        }
    }

    for parent_id in &commit.parents {
        collect_reachable(store, shard_dir, parent_id, seen_commits, reachable)?;
    }

    Ok(())
}

pub fn prune(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let store = Store::new(&shard_dir);
    let mut reachable: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1. Walk from HEAD commit
    let head_path = shard_dir.join("HEAD");
    if head_path.exists() {
        let head = fs::read_to_string(&head_path)?;
        let head = head.trim().to_string();
        collect_reachable(
            &store,
            &shard_dir,
            &head,
            &mut std::collections::HashSet::new(),
            &mut reachable,
        )?;
    }

    // 2. Walk from tags
    let tags = load_tags(&shard_dir)?;
    for commit_id in tags.values() {
        collect_reachable(
            &store,
            &shard_dir,
            commit_id,
            &mut std::collections::HashSet::new(),
            &mut reachable,
        )?;
    }

    // 3. Walk from index (staged files)
    let index = Index::load(&shard_dir.join("index"))?;
    for manifest in index.files.values() {
        let json = serde_json::to_vec(manifest)?;
        let hash = blake3::hash(&json);
        let hash_hex = hash.to_hex().to_string();
        reachable.insert(hash_hex);
        for chunk_hash in &manifest.chunks {
            reachable.insert(chunk_hash.clone());
        }
    }

    // 4. Scan objects and remove unreachable
    let objects_dir = shard_dir.join("objects");
    let mut pruned = 0u64;
    let mut kept = 0u64;
    if objects_dir.exists() {
        for entry in fs::read_dir(&objects_dir)? {
            let entry = entry?;
            let prefix_dir = entry.path();
            if prefix_dir.is_dir() {
                for file_entry in fs::read_dir(&prefix_dir)? {
                    let file_entry = file_entry?;
                    let hash_hex = file_entry.file_name().to_string_lossy().to_string();
                    if !reachable.contains(&hash_hex) {
                        fs::remove_file(file_entry.path())?;
                        pruned += 1;
                    } else {
                        kept += 1;
                    }
                }
            }
        }
    }

    println!("Pruned {} objects. {} objects remain.", pruned, kept);
    Ok(())
}

pub fn peer_add(path: &Path, multiaddr: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let peers_path = shard_dir.join("peers.json");
    let mut peers: Vec<String> = if peers_path.exists() {
        let data = fs::read(&peers_path)?;
        serde_json::from_slice(&data)?
    } else {
        Vec::new()
    };

    if !peers.contains(&multiaddr.to_string()) {
        peers.push(multiaddr.to_string());
        let data = serde_json::to_vec(&peers)?;
        fs::write(peers_path, data)?;
        println!("Added peer: {}", multiaddr);
    } else {
        println!("Peer already exists: {}", multiaddr);
    }

    Ok(())
}

fn load_peers(shard_dir: &Path) -> Result<Vec<String>> {
    let peers_path = shard_dir.join("peers.json");
    if peers_path.exists() {
        let data = fs::read(peers_path)?;
        Ok(serde_json::from_slice(&data)?)
    } else {
        Ok(Vec::new())
    }
}

struct RepoProvider {
    store: Store,
}

impl shard_net::p2p::ShardContentProvider for RepoProvider {
    fn get_manifest(&self, id: &str) -> Option<Vec<u8>> {
        self.store.get_chunk(id).ok()
    }
    fn get_chunk(&self, id: &str) -> Option<Vec<u8>> {
        self.store.get_chunk(id).ok()
    }
}

pub async fn share(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let mut node = shard_net::p2p::Node::new().await?;

    // Bootstrap from peers
    let peers = load_peers(&shard_dir)?;
    for peer in peers {
        if let Ok(addr) = peer.parse::<shard_net::libp2p::Multiaddr>() {
            let _ = node.swarm.dial(addr);
        }
    }

    node.listen("/ip4/0.0.0.0/tcp/0").await?; // Listen on random port (TCP)

    // In a real implementation, we would load the repo and serve requests.
    // For now, we just start the node to prove connectivity.
    println!("Sharing repository...");
    let store = Store::new(&shard_dir);
    let provider = RepoProvider { store };
    node.run(provider).await;

    Ok(())
}

pub async fn sync(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let config = load_config(&shard_dir)?;
    let repo_id = config
        .get("repo_id")
        .ok_or_else(|| anyhow::anyhow!("No repo_id in config. Run `shard init` to create one."))?;
    let topic_str = format!("/shard/repo/{}", repo_id);
    let topic = shard_net::libp2p::gossipsub::IdentTopic::new(topic_str);

    let mut node = shard_net::p2p::Node::new().await?;
    node.subscribe(&topic)?;
    node.listen("/ip4/0.0.0.0/tcp/0").await?;

    // Bootstrap from configured peers
    let peers = load_peers(&shard_dir)?;
    for peer in peers {
        if let Ok(addr) = peer.parse::<shard_net::libp2p::Multiaddr>() {
            let _ = node.swarm.dial(addr);
        }
    }

    let head_path = shard_dir.join("HEAD");

    // Initial announce (may fail with InsufficientPeers if no peers yet)
    if head_path.exists() {
        if let Ok(head) = fs::read_to_string(&head_path) {
            let head = head.trim().to_string();
            let msg = format!("announce:{}", head);
            match node.publish(&topic, msg.as_bytes()) {
                Ok(_) => println!("Announced commit {} on sync topic", head),
                Err(e) => eprintln!("Initial announce (will retry): {}", e),
            }
        }
    } else {
        println!("No commits to announce");
    }

    println!("Syncing on topic with peer id: {}", node.local_peer_id());
    let _ = std::io::stdout().flush();

    let store = Store::new(&shard_dir);
    let provider = RepoProvider { store };

    let mut interval = tokio::time::interval(Duration::from_secs(5));
    let mut address_book: HashMap<shard_net::libp2p::PeerId, Vec<shard_net::libp2p::Multiaddr>> =
        HashMap::new();
    let path_buf = path.to_path_buf();

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if head_path.exists() {
                    if let Ok(head) = fs::read_to_string(&head_path) {
                        let head = head.trim().to_string();
                        let msg = format!("announce:{}", head);
                        match node.publish(&topic, msg.as_bytes()) {
                            Ok(_) => println!("Re-announced commit {} on sync topic", head),
                            Err(e) => eprintln!("Re-announce failed: {}", e),
                        }
                    }
                }
            }
            event = node.swarm.select_next_some() => {
                match event {
                    shard_net::libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                        println!("Listening on {address:?}");
                        let _ = std::io::stdout().flush();
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Mdns(
                            shard_net::libp2p::mdns::Event::Discovered(list),
                        ),
                    ) => {
                        for (peer_id, multiaddr) in list {
                            println!("mDNS discovered: {peer_id} {multiaddr}");
                            address_book.entry(peer_id).or_default().push(multiaddr.clone());
                            node.swarm
                                .behaviour_mut()
                                .gossipsub
                                .add_explicit_peer(&peer_id);
                            node.swarm
                                .behaviour_mut()
                                .kademlia
                                .add_address(&peer_id, multiaddr);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Mdns(shard_net::libp2p::mdns::Event::Expired(
                            list,
                        )),
                    ) => {
                        for (peer_id, _multiaddr) in list {
                            println!("mDNS expired: {peer_id}");
                            node.swarm
                                .behaviour_mut()
                                .gossipsub
                                .remove_explicit_peer(&peer_id);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Gossipsub(
                            shard_net::libp2p::gossipsub::Event::Message {
                                propagation_source,
                                message,
                                ..
                            },
                        ),
                    ) => {
                        if let Ok(text) = String::from_utf8(message.data.clone()) {
                            if let Some(commit_id) = text.strip_prefix("announce:") {
                                println!(
                                    "Peer {} announced commit: {}",
                                    propagation_source, commit_id
                                );
                                let peer = propagation_source;
                                let commit_id_owned = commit_id.to_string();
                                // Reply with our HEAD if different (triggers peer to pull from us)
                                if head_path.exists() {
                                    if let Ok(head) = fs::read_to_string(&head_path) {
                                        let head = head.trim().to_string();
                                        if head != commit_id_owned {
                                            let msg = format!("announce:{}", head);
                                            let _ = node.publish(&topic, msg.as_bytes());
                                        }
                                    }
                                }
                                if let Some(addrs) = address_book.get(&peer) {
                                    if let Some(addr) = addrs.first() {
                                        let multiaddr_str = format!("{}/p2p/{}", addr, peer);
                                        let path_clone = path_buf.clone();
                                        tokio::spawn(async move {
                                            match pull(&path_clone, &multiaddr_str, &commit_id_owned).await {
                                                Ok(_) => println!("Auto-pulled commit {} from {}", commit_id_owned, peer),
                                                Err(e) => eprintln!("Auto-pull failed for commit {} from {}: {}", commit_id_owned, peer, e),
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::RequestResponse(
                            shard_net::libp2p::request_response::Event::Message { peer, message },
                        ),
                    ) => {
                        if let shard_net::libp2p::request_response::Message::Request {
                            request, channel, ..
                        } = message
                        {
                            println!("Received request from {}", peer);
                            node.serve_request(&provider, request, channel);
                        } else {
                            println!("Received Response from {}", peer);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::RequestResponse(
                            shard_net::libp2p::request_response::Event::OutboundFailure {
                                peer, error, ..
                            },
                        ),
                    ) => {
                        eprintln!("Outbound failure to {}: {:?}", peer, error);
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::RequestResponse(
                            shard_net::libp2p::request_response::Event::InboundFailure {
                                peer, error, ..
                            },
                        ),
                    ) => {
                        eprintln!("Inbound failure from {}: {:?}", peer, error);
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Identify(
                            shard_net::libp2p::identify::Event::Received { peer_id, info },
                        ),
                    ) => {
                        println!("Identify received from {}: {:?}", peer_id, info.listen_addrs);
                        for addr in info.listen_addrs {
                            address_book.entry(peer_id).or_default().push(addr);
                        }
                        let _ = std::io::stdout().flush();
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Identify(event),
                    ) => {
                        println!("Identify event: {:?}", event);
                    }
                    shard_net::libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                        println!("Connection established with {}", peer_id);
                        // Only store the address when we dialed (it's the peer's listen addr).
                        // For listener connections, send_back_addr is the ephemeral port — useless for dialing back.
                        if let shard_net::libp2p::core::ConnectedPoint::Dialer { address, .. } = &endpoint {
                            address_book.entry(peer_id).or_default().push(address.clone());
                        }
                        node.swarm
                            .behaviour_mut()
                            .gossipsub
                            .add_explicit_peer(&peer_id);
                        // Announce HEAD to the newly connected peer
                        if head_path.exists() {
                            if let Ok(head) = fs::read_to_string(&head_path) {
                                let head = head.trim().to_string();
                                let msg = format!("announce:{}", head);
                                let _ = node.publish(&topic, msg.as_bytes());
                            }
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::IncomingConnection {
                        local_addr,
                        send_back_addr,
                        ..
                    } => {
                        println!(
                            "Incoming connection from {} to {}",
                            send_back_addr, local_addr
                        );
                    }
                    e => {
                        println!("Event: {:?}", e);
                    }
                }
            }
        }
    }
}

pub async fn pull(path: &Path, peer: &str, commit_id: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    // pull can work on empty repo or existing one.
    // if !shard_dir.exists() { init(path)?; }

    if !shard_dir.exists() {
        init(path)?;
    }

    let store = Store::new(&shard_dir);

    let mut node = shard_net::p2p::Node::new().await?;

    // Parse peer multiaddr
    let multiaddr: shard_net::libp2p::Multiaddr = peer.parse()?;
    let peer_id = match multiaddr.iter().last() {
        Some(shard_net::libp2p::multiaddr::Protocol::P2p(peer_id)) => peer_id,
        _ => anyhow::bail!("Multiaddr must end with /p2p/<peer_id>"),
    };

    // 1. Get Commit (sequential — single request)
    println!("Pulling commit {} from {}...", commit_id, peer);
    let commit_data = node
        .request_manifest(&multiaddr, peer_id, commit_id.to_string())
        .await?;
    let hash = blake3::hash(&commit_data);
    if hash.to_hex().to_string() != commit_id {
        anyhow::bail!("Commit hash mismatch");
    }
    let chunk = crate::chunker::Chunk {
        hash,
        data: commit_data.clone(),
        offset: 0,
    };
    store.put_chunk(&chunk)?;

    let commit: Commit = serde_json::from_slice(&commit_data)?;
    println!("Got commit: {}", commit.message);

    // Set repo_id from commit's public key so clones share the gossipsub topic
    if let Some(pk_hex) = &commit.public_key {
        let pk_bytes = hex::decode(pk_hex)?;
        let repo_id = blake3::hash(&pk_bytes).to_hex().to_string();
        let mut config = load_config(&shard_dir)?;
        config.insert("repo_id".to_string(), repo_id);
        save_config(&shard_dir, &config)?;
    }

    // 2. Fetch all manifests in parallel
    let manifest_requests: Vec<(String, shard_net::protocol::ShardRequest)> = commit
        .manifests
        .iter()
        .map(|id| {
            (
                id.clone(),
                shard_net::protocol::ShardRequest::GetManifest(id.clone()),
            )
        })
        .collect();
    let manifest_results = node
        .request_parallel(&multiaddr, peer_id, manifest_requests)
        .await?;

    let mut all_chunk_ids: Vec<String> = Vec::new();
    let mut file_manifests: Vec<FileManifest> = Vec::new();

    for (manifest_id, manifest_data) in &manifest_results {
        let hash = blake3::hash(manifest_data);
        if hash.to_hex().to_string() != *manifest_id {
            anyhow::bail!("Manifest hash mismatch: {}", manifest_id);
        }
        let chunk = crate::chunker::Chunk {
            hash,
            data: manifest_data.clone(),
            offset: 0,
        };
        store.put_chunk(&chunk)?;
        let manifest: FileManifest = serde_json::from_slice(manifest_data)?;
        println!("Fetching file: {}", manifest.name);
        all_chunk_ids.extend(manifest.chunks.clone());
        file_manifests.push(manifest);
    }

    // 3. Fetch all missing chunks in parallel
    let needed_chunks: Vec<String> = all_chunk_ids
        .into_iter()
        .filter(|id| store.get_chunk(id).is_err())
        .collect();

    if !needed_chunks.is_empty() {
        println!("Fetching {} chunks...", needed_chunks.len());
        let chunk_requests: Vec<(String, shard_net::protocol::ShardRequest)> = needed_chunks
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    shard_net::protocol::ShardRequest::GetChunk(id.clone()),
                )
            })
            .collect();
        let chunk_results = node
            .request_parallel(&multiaddr, peer_id, chunk_requests)
            .await?;
        for (chunk_id, chunk_data) in &chunk_results {
            let hash = blake3::hash(chunk_data);
            if hash.to_hex().to_string() != *chunk_id {
                anyhow::bail!("Chunk hash mismatch: {}", chunk_id);
            }
            let chunk = crate::chunker::Chunk {
                hash,
                data: chunk_data.clone(),
                offset: 0,
            };
            store.put_chunk(&chunk)?;
        }
    }

    // 4. Reconstruct all files
    for manifest in &file_manifests {
        let mut file_data = Vec::new();
        for chunk_id in &manifest.chunks {
            let data = store.get_chunk(chunk_id)?;
            file_data.extend_from_slice(&data);
        }
        fs::write(path.join(&manifest.name), file_data)?;
        println!("Reconstructed file: {}", manifest.name);
    }

    println!("Pull complete.");
    Ok(())
}
