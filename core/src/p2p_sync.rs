use crate::auth;
use crate::branch;
use crate::chunk_codec;
use crate::chunker::Chunk;
use crate::commit::Commit;
use crate::compression::Compression;
use crate::config;
use crate::encryption::RepoCipher;
use crate::error::ShardError;
use crate::fs_util::{atomic_write, safe_repo_join, validate_object_id};
use crate::keychain;
use crate::manifest::FileManifest;
use crate::metadata;
use crate::partial;
use crate::repo;
use crate::store::Store;
use anyhow::Result;
use shard_net::libp2p::futures::StreamExt;
use shard_net::libp2p::{Multiaddr, PeerId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

const MAX_CONCURRENT_AUTO_PULLS: usize = 4;

/// Execute a blocking closure with `tokio::task::block_in_place` when running on a
/// multi-threaded Tokio runtime, falling back gracefully on `CurrentThread` runtimes
/// (where `block_in_place` would panic).
fn run_blocking_in_runtime<F: FnOnce()>(f: F) {
    if matches!(
        tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()),
        Ok(tokio::runtime::RuntimeFlavor::MultiThread)
    ) {
        tokio::task::block_in_place(f);
    } else {
        f();
    }
}

/// Sliding-window per-peer rate limiter honoring `rate_limit_max_requests` and
/// `rate_limit_window_secs` in both `share` and `sync`.
struct PeerRateLimiter {
    max_requests: u32,
    window: Duration,
    buckets: HashMap<PeerId, VecDeque<Instant>>,
}

impl PeerRateLimiter {
    fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            max_requests: max_requests.max(1),
            window: Duration::from_secs(window_secs.max(1)),
            buckets: HashMap::new(),
        }
    }

    fn allow(&mut self, peer: PeerId) -> bool {
        let now = Instant::now();
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        let q = self.buckets.entry(peer).or_default();
        while q.front().is_some_and(|&ts| ts < cutoff) {
            q.pop_front();
        }
        if q.len() >= self.max_requests as usize {
            false
        } else {
            q.push_back(now);
            true
        }
    }

    fn prune_stale(&mut self) {
        let now = Instant::now();
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        self.buckets.retain(|_, q| {
            while q.front().is_some_and(|&ts| ts < cutoff) {
                q.pop_front();
            }
            !q.is_empty()
        });
    }
}

pub fn peer_add(path: &Path, multiaddr: &str, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_peer_add
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "peer_add", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }

        if multiaddr.is_empty() || multiaddr.parse::<Multiaddr>().is_err() {
            anyhow::bail!(
                "invalid multiaddr '{}' (must be a valid libp2p multiaddr, e.g. /ip4/1.2.3.4/tcp/5678/p2p/...)",
                multiaddr
            );
        }

        let peers_path = shard_dir.join("peers.json");
        let mut peers: Vec<String> = if peers_path.exists() {
            let data = fs::read(&peers_path)?;
            serde_json::from_slice(&data)?
        } else {
            Vec::new()
        };

        let added = if !peers.contains(&multiaddr.to_string()) {
            peers.push(multiaddr.to_string());
            let data = serde_json::to_vec_pretty(&peers)?;
            atomic_write(&peers_path, &data)?;
            true
        } else {
            false
        };

        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "multiaddr": multiaddr,
                    "added": added,
                }))?
            );
        } else if added {
            println!("Added peer: {}", multiaddr);
        } else {
            println!("Peer already exists: {}", multiaddr);
        }

        Ok(())
    })
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

/// Ensure a multiaddr has exactly one trailing `/p2p/<peer_id>` component.
fn format_peer_multiaddr(addr: &Multiaddr, peer: &PeerId) -> String {
    let already_has_p2p = matches!(
        addr.iter().last(),
        Some(shard_net::libp2p::multiaddr::Protocol::P2p(_))
    );
    if already_has_p2p {
        addr.to_string()
    } else {
        format!("{}/p2p/{}", addr, peer)
    }
}

struct RepoProvider {
    store: Store,
    shard_dir: PathBuf,
    cipher: Option<RepoCipher>,
}

impl shard_net::p2p::ShardContentProvider for RepoProvider {
    fn auth_required(&self) -> bool {
        auth::authorized_keys_path(&self.shard_dir).exists()
    }

    fn authenticate(&self, _peer_id: &PeerId, public_key: &[u8]) -> bool {
        if !self.auth_required() {
            return true;
        }
        let Ok(arr) = <[u8; 32]>::try_from(public_key) else {
            return false;
        };
        let Ok(pk) = ed25519_dalek::VerifyingKey::from_bytes(&arr) else {
            return false;
        };
        auth::load_authorized_keys(&self.shard_dir)
            .map(|keys| keys.contains(&pk))
            .unwrap_or(false)
    }

    fn get_manifest(&self, id: &str) -> Option<Vec<u8>> {
        validate_object_id(id).ok()?;
        self.store.get_chunk(id).ok()
    }

    fn get_chunk(&self, id: &str) -> Option<Vec<u8>> {
        validate_object_id(id).ok()?;
        self.store.get_chunk(id).ok()
    }

    fn put_chunk(&mut self, id: &str, data: &[u8]) -> bool {
        if !chunk_codec::verify_wire_object(id, data, self.cipher.as_ref()) {
            return false;
        }
        let Ok(id_bytes) = hex::decode(id) else {
            return false;
        };
        let Ok(hash_arr) = <[u8; 32]>::try_from(id_bytes.as_slice()) else {
            return false;
        };
        let hash = blake3::Hash::from_bytes(hash_arr);
        let Some(repo_root) = self.shard_dir.parent() else {
            return false;
        };
        let object_id = id.to_string();
        crate::with_write_op(repo_root, "p2p_put_chunk", || {
            self.store.put_chunk(&Chunk {
                hash,
                data: data.to_vec(),
                offset: 0,
            })?;
            self.store.flush()?;
            crate::inbound_pins::pin_object(&self.shard_dir, &object_id)?;
            Ok(())
        })
        .is_ok()
    }

    fn verify_auth(&self, public_key: &[u8], challenge_payload: &[u8], signature: &[u8]) -> bool {
        use ed25519_dalek::Verifier;
        let pk_bytes: [u8; 32] = match public_key.try_into() {
            Ok(b) => b,
            Err(_) => return false,
        };
        let pk = match ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes) {
            Ok(k) => k,
            Err(_) => return false,
        };
        let sig_bytes: [u8; 64] = match signature.try_into() {
            Ok(b) => b,
            Err(_) => return false,
        };
        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        if pk.verify(challenge_payload, &sig).is_err() {
            return false;
        }
        if self.auth_required() {
            if let Ok(keys) = auth::load_authorized_keys(&self.shard_dir) {
                return keys.contains(&pk);
            }
            return false;
        }
        true
    }

    fn repo_public_key(&self) -> Option<Vec<u8>> {
        auth::load_public_key(&self.shard_dir).ok()
    }
}

/// Compute file count and total size for a commit using an already-opened `Store`
/// (avoids Sled double-open exclusive lock errors).
fn commit_stats(store: &Store, commit_id: &str) -> Result<(u64, u64)> {
    let commit = repo::load_commit(store, commit_id)?;
    let mut file_count = 0u64;
    let mut total_size = 0u64;
    for mid in &commit.manifests {
        if let Ok(data) = store.get_chunk(mid) {
            if let Ok(m) = metadata::deserialize::<FileManifest>(&data) {
                file_count += 1;
                total_size += m.size;
            }
        }
    }
    Ok((file_count, total_size))
}

pub async fn share(path: &Path, listen_addr: &str, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_share
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        return Err(ShardError::NotARepo.into());
    }

    let config = crate::load_config(&shard_dir)?;
    let repo_id = config.get("repo_id").cloned().unwrap_or_default();
    let repo_name = config.get("repo_name").cloned().unwrap_or_default();
    let rate_limit_max = config::config_get_rate_limit_max(&config);
    let rate_limit_window = config::config_get_rate_limit_window(&config);

    let mut node = shard_net::p2p::Node::new().await?;
    let ann_topic = shard_net::libp2p::gossipsub::IdentTopic::new("shard:ann");
    let repo_topic =
        shard_net::libp2p::gossipsub::IdentTopic::new(format!("/shard/repo/{}", repo_id));
    node.subscribe(&ann_topic)?;
    node.subscribe(&repo_topic)?;

    for peer in load_peers(&shard_dir)? {
        if let Ok(addr) = peer.parse::<Multiaddr>() {
            if let Err(e) = node.swarm.dial(addr.clone()) {
                warn!("Failed to dial peer {}: {}", addr, e);
            }
        }
    }
    node.swarm.behaviour_mut().kademlia.bootstrap().ok();
    node.listen(listen_addr).await?;

    let mut peer_multiaddr = String::new();
    let store = Store::open(&shard_dir)?;
    let cipher = auth::maybe_load_cipher(&shard_dir)?;
    let mut provider = RepoProvider {
        store,
        shard_dir: shard_dir.clone(),
        cipher,
    };

    let publish_ann = |node: &mut shard_net::p2p::Node, ann: &shard_net::protocol::Announcement| {
        if let Ok(payload) = serde_json::to_vec(ann) {
            let _ = node.publish(&ann_topic, payload.clone());
            let _ = node.publish(&repo_topic, payload);
        }
    };

    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "status": "sharing",
                "peer_id": node.local_peer_id().to_string(),
            }))?
        );
    } else {
        println!("Local peer id: {}", node.local_peer_id());
        println!("Sharing repository...");
        println!("Peer ID: {}", node.local_peer_id());
        let _ = std::io::stdout().flush();
    }

    let mut interval = tokio::time::interval(Duration::from_secs(30));
    let mut rate_limiter = PeerRateLimiter::new(rate_limit_max, rate_limit_window);
    let gc_task = crate::gc::GcTaskHandle::spawn_if_enabled(path);

    let res = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Share shutting down...");
                break Ok(());
            }
            _ = interval.tick() => {
                rate_limiter.prune_stale();
                if let Some(ref head) = branch::resolve_head(&shard_dir)?.1 {
                    let (fc, ts) = commit_stats(&provider.store, head).unwrap_or_default();
                    let ann = shard_net::protocol::Announcement {
                        commit_id: head.clone(),
                        file_count: fc,
                        total_size: ts,
                        repo_name: repo_name.clone(),
                        peer_multiaddr: if peer_multiaddr.is_empty() {
                            String::new()
                        } else {
                            format!("{}/p2p/{}", peer_multiaddr, node.local_peer_id())
                        },
                    };
                    publish_ann(&mut node, &ann);
                }
            }
            event = node.swarm.select_next_some() => {
                match event {
                    shard_net::libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                        if peer_multiaddr.is_empty() {
                            peer_multiaddr = address.to_string();
                        }
                        if !json {
                            println!("Listening on \"{}\"", address);
                            let _ = std::io::stdout().flush();
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Mdns(
                            shard_net::libp2p::mdns::Event::Discovered(list),
                        ),
                    ) => {
                        for (peer_id, multiaddr) in list {
                            debug!("mDNS discovered: {peer_id} {multiaddr}");
                            node.swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                            node.swarm.behaviour_mut().kademlia.add_address(&peer_id, multiaddr);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Mdns(
                            shard_net::libp2p::mdns::Event::Expired(list),
                        ),
                    ) => {
                        for (peer_id, _) in list {
                            node.swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Gossipsub(
                            shard_net::libp2p::gossipsub::Event::Subscribed { peer_id, .. },
                        ),
                    ) => {
                        debug!("Peer {} subscribed to gossipsub topic", peer_id);
                        if let Some(ref head) = branch::resolve_head(&shard_dir)?.1 {
                            let (fc, ts) = commit_stats(&provider.store, head).unwrap_or_default();
                            let ann = shard_net::protocol::Announcement {
                                commit_id: head.clone(),
                                file_count: fc,
                                total_size: ts,
                                repo_name: repo_name.clone(),
                                peer_multiaddr: if peer_multiaddr.is_empty() {
                                    String::new()
                                } else {
                                    format!("{}/p2p/{}", peer_multiaddr, node.local_peer_id())
                                },
                            };
                            publish_ann(&mut node, &ann);
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
                        if let Ok(ann) = serde_json::from_slice::<shard_net::protocol::Announcement>(&message.data) {
                            debug!("Peer {} announced commit: {}", propagation_source, ann.commit_id);
                            if let Ok((_, Some(our_head))) = branch::resolve_head(&shard_dir) {
                                if our_head != ann.commit_id {
                                    let (fc, ts) = commit_stats(&provider.store, &our_head).unwrap_or_default();
                                    let reply = shard_net::protocol::Announcement {
                                        commit_id: our_head,
                                        file_count: fc,
                                        total_size: ts,
                                        repo_name: repo_name.clone(),
                                        peer_multiaddr: if peer_multiaddr.is_empty() {
                                            String::new()
                                        } else {
                                            format!("{}/p2p/{}", peer_multiaddr, node.local_peer_id())
                                        },
                                    };
                                    publish_ann(&mut node, &reply);
                                }
                            }
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::RequestResponse(
                            shard_net::libp2p::request_response::Event::Message {
                                peer,
                                message:
                                    shard_net::libp2p::request_response::Message::Request {
                                        request,
                                        channel,
                                        ..
                                    },
                                ..
                            },
                        ),
                    ) => {
                        if !rate_limiter.allow(peer) {
                            warn!("Dropping request from {} in share: rate limit exceeded", peer);
                        } else {
                            // `PutChunk` takes a blocking repo write guard; do not
                            // stall the runtime while another task owns it.
                            run_blocking_in_runtime(|| {
                                node.serve_request(&peer, &mut provider, request, channel);
                            });
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        debug!("Connection established with {}", peer_id);
                        node.swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                        if let Some(ref head) = branch::resolve_head(&shard_dir)?.1 {
                            let (fc, ts) = commit_stats(&provider.store, head).unwrap_or_default();
                            let ann = shard_net::protocol::Announcement {
                                commit_id: head.clone(),
                                file_count: fc,
                                total_size: ts,
                                repo_name: repo_name.clone(),
                                peer_multiaddr: if peer_multiaddr.is_empty() {
                                    String::new()
                                } else {
                                    format!("{}/p2p/{}", peer_multiaddr, node.local_peer_id())
                                },
                            };
                            publish_ann(&mut node, &ann);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::ConnectionClosed { peer_id, num_established: 0, .. } => {
                        node.on_connection_closed(&peer_id);
                    }
                    _ => {}
                }
            }
        }
    };
    gc_task.shutdown().await;
    res
}

/// Start a circuit relay v2 server for NAT traversal.
pub async fn relay(listen_addr: &str, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_relay
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut node = shard_net::p2p::Node::new().await?;
    node.listen(listen_addr).await?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "status": "relay active",
                "listen": listen_addr,
                "peer_id": node.local_peer_id().to_string(),
            }))?
        );
    } else {
        println!("Relay server active on {}", listen_addr);
        println!("Peer ID: {}", node.local_peer_id());
        println!("Ready to accept circuit relay v2 reservations");
    }
    node.run(EmptyProvider).await;
    Ok(())
}

/// Minimal provider for relay-only mode (no repo content needed).
struct EmptyProvider;
impl shard_net::p2p::ShardContentProvider for EmptyProvider {
    fn get_manifest(&self, _id: &str) -> Option<Vec<u8>> {
        None
    }
    fn get_chunk(&self, _id: &str) -> Option<Vec<u8>> {
        None
    }
    fn put_chunk(&mut self, _id: &str, _data: &[u8]) -> bool {
        false
    }
    fn verify_auth(&self, _public_key: &[u8], _nonce: &[u8], _signature: &[u8]) -> bool {
        false
    }
    fn repo_public_key(&self) -> Option<Vec<u8>> {
        None
    }
}

pub async fn sync(path: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_sync
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        return Err(ShardError::NotARepo.into());
    }

    let config = crate::load_config(&shard_dir)?;
    let repo_id = config.get("repo_id").cloned().unwrap_or_default();
    let repo_name = config.get("repo_name").cloned().unwrap_or_default();
    let rate_limit_max = config::config_get_rate_limit_max(&config);
    let rate_limit_window = config::config_get_rate_limit_window(&config);

    let ann_topic = shard_net::libp2p::gossipsub::IdentTopic::new("shard:ann");
    let repo_topic =
        shard_net::libp2p::gossipsub::IdentTopic::new(format!("/shard/repo/{}", repo_id));

    let mut node = shard_net::p2p::Node::new().await?;
    node.subscribe(&ann_topic)?;
    node.subscribe(&repo_topic)?;
    node.listen("/ip4/0.0.0.0/tcp/0").await?;

    let mut peer_multiaddr = String::new();
    let mut address_book: HashMap<PeerId, Vec<Multiaddr>> = HashMap::new();

    // Bootstrap from configured peers (known trusted peers for SSRF protection)
    for peer_str in load_peers(&shard_dir)? {
        if let Ok(addr) = peer_str.parse::<Multiaddr>() {
            if let Some(shard_net::libp2p::multiaddr::Protocol::P2p(pid)) = addr.iter().last() {
                address_book.entry(pid).or_default().push(addr.clone());
            }
            let _ = node.swarm.dial(addr);
        }
    }
    node.swarm.behaviour_mut().kademlia.bootstrap().ok();

    let publish_ann = |node: &mut shard_net::p2p::Node, ann: &shard_net::protocol::Announcement| {
        if let Ok(payload) = serde_json::to_vec(ann) {
            let _ = node.publish(&ann_topic, payload.clone());
            let _ = node.publish(&repo_topic, payload);
        }
    };

    if !json {
        println!("Local peer id: {}", node.local_peer_id());
        println!("Syncing on topic with peer id: {}", node.local_peer_id());
        let _ = std::io::stdout().flush();
    }

    let store = Store::open(&shard_dir)?;
    let cipher = auth::maybe_load_cipher(&shard_dir)?;
    let mut provider = RepoProvider {
        store,
        shard_dir: shard_dir.clone(),
        cipher,
    };

    let mut interval = tokio::time::interval(Duration::from_secs(3));
    let mut rate_limiter = PeerRateLimiter::new(rate_limit_max, rate_limit_window);
    let mut announce_limiter = PeerRateLimiter::new(10, rate_limit_window);
    let in_flight_pulls: Arc<parking_lot::Mutex<HashSet<String>>> =
        Arc::new(parking_lot::Mutex::new(HashSet::new()));
    let pull_semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_AUTO_PULLS));
    let path_buf = path.to_path_buf();
    let gc_task = crate::gc::GcTaskHandle::spawn_if_enabled(path);

    let res = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Sync shutting down...");
                break Ok(());
            }
            _ = interval.tick() => {
                rate_limiter.prune_stale();
                announce_limiter.prune_stale();
                if let Some(ref head) = branch::resolve_head(&shard_dir)?.1 {
                    let (fc, ts) = commit_stats(&provider.store, head).unwrap_or_default();
                    let ann = shard_net::protocol::Announcement {
                        commit_id: head.clone(),
                        file_count: fc,
                        total_size: ts,
                        repo_name: repo_name.clone(),
                        peer_multiaddr: if peer_multiaddr.is_empty() {
                            String::new()
                        } else {
                            format!("{}/p2p/{}", peer_multiaddr, node.local_peer_id())
                        },
                    };
                    publish_ann(&mut node, &ann);
                }
            }
            event = node.swarm.select_next_some() => {
                match event {
                    shard_net::libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                        if peer_multiaddr.is_empty() {
                            peer_multiaddr = address.to_string();
                        }
                        if !json {
                            println!("Listening on \"{}\"", address);
                            let _ = std::io::stdout().flush();
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Mdns(
                            shard_net::libp2p::mdns::Event::Discovered(list),
                        ),
                    ) => {
                        for (peer_id, multiaddr) in list {
                            debug!("mDNS discovered: {peer_id} {multiaddr}");
                            address_book.entry(peer_id).or_default().push(multiaddr.clone());
                            node.swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                            node.swarm.behaviour_mut().kademlia.add_address(&peer_id, multiaddr);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Mdns(
                            shard_net::libp2p::mdns::Event::Expired(list),
                        ),
                    ) => {
                        for (peer_id, _) in list {
                            node.swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Gossipsub(
                            shard_net::libp2p::gossipsub::Event::Subscribed { peer_id, .. },
                        ),
                    ) => {
                        debug!("Peer {} subscribed to gossipsub topic", peer_id);
                        if let Some(ref head) = branch::resolve_head(&shard_dir)?.1 {
                            let (fc, ts) = commit_stats(&provider.store, head).unwrap_or_default();
                            let ann = shard_net::protocol::Announcement {
                                commit_id: head.clone(),
                                file_count: fc,
                                total_size: ts,
                                repo_name: repo_name.clone(),
                                peer_multiaddr: if peer_multiaddr.is_empty() {
                                    String::new()
                                } else {
                                    format!("{}/p2p/{}", peer_multiaddr, node.local_peer_id())
                                },
                            };
                            publish_ann(&mut node, &ann);
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
                        if let Ok(ann) = serde_json::from_slice::<shard_net::protocol::Announcement>(&message.data) {
                            let peer = propagation_source;
                            let commit_id_owned = ann.commit_id;
                            if validate_object_id(&commit_id_owned).is_err() {
                                warn!("Ignoring announcement with invalid commit_id from {}", peer);
                                continue;
                            }
                            if !announce_limiter.allow(peer) {
                                warn!("Announcement rate limit exceeded for peer {}", peer);
                                continue;
                            }

                            // Reply with our HEAD if different
                            let our_head = branch::resolve_head(&shard_dir)?.1.unwrap_or_default();
                            if !our_head.is_empty() && our_head != commit_id_owned {
                                let (fc, ts) = commit_stats(&provider.store, &our_head).unwrap_or_default();
                                let reply = shard_net::protocol::Announcement {
                                    commit_id: our_head,
                                    file_count: fc,
                                    total_size: ts,
                                    repo_name: repo_name.clone(),
                                    peer_multiaddr: if peer_multiaddr.is_empty() {
                                        String::new()
                                    } else {
                                        format!("{}/p2p/{}", peer_multiaddr, node.local_peer_id())
                                    },
                                };
                                publish_ann(&mut node, &reply);
                            }

                            // Only auto-pull announcements for THIS repository's topic (never
                            // unrelated repositories broadcasting on the global shard:ann topic).
                            if message.topic != repo_topic.hash() {
                                continue;
                            }

                            // SSRF defense: ONLY auto-pull from peers we already know via
                            // configured `peers.json`, mDNS, or an established dialed connection
                            // (never dial arbitrary unverified multiaddrs from gossipsub messages!).
                            if provider.store.has_chunk(&commit_id_owned) {
                                continue;
                            }

                            if let Some(addr) = address_book.get(&peer).and_then(|a| a.first()) {
                                {
                                    let mut guard = in_flight_pulls.lock();
                                    if !guard.insert(commit_id_owned.clone()) {
                                        continue;
                                    }
                                }
                                let multiaddr_str = format_peer_multiaddr(addr, &peer);
                                let path_clone = path_buf.clone();
                                let sem = pull_semaphore.clone();
                                let in_flight_clone = in_flight_pulls.clone();
                                tokio::spawn(async move {
                                    let Ok(_permit) = sem.acquire_owned().await else {
                                        in_flight_clone.lock().remove(&commit_id_owned);
                                        return;
                                    };
                                    match pull_inner(&path_clone, &multiaddr_str, &commit_id_owned, false, true).await {
                                        Ok(_) => {
                                            println!("Auto-pulled commit {} from {}", commit_id_owned, peer);
                                            let _ = std::io::stdout().flush();
                                        }
                                        Err(e) => {
                                            error!("Auto-pull failed for commit {} from {}: {}", commit_id_owned, peer, e);
                                            // Allow retry on subsequent announcements
                                            in_flight_clone.lock().remove(&commit_id_owned);
                                        }
                                    }
                                });
                            }
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::RequestResponse(
                            shard_net::libp2p::request_response::Event::Message {
                                peer,
                                message:
                                    shard_net::libp2p::request_response::Message::Request {
                                        request,
                                        channel,
                                        ..
                                    },
                                ..
                            },
                        ),
                    ) => {
                        if !rate_limiter.allow(peer) {
                            warn!("Dropping request from {}: rate limit exceeded", peer);
                        } else {
                            // `PutChunk` acquires a blocking repo write guard. Keep the
                            // event-loop worker from blocking the runtime while an
                            // auto-pull owns that guard across network awaits.
                            run_blocking_in_runtime(|| {
                                node.serve_request(&peer, &mut provider, request, channel);
                            });
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::Behaviour(
                        shard_net::p2p::ShardBehaviourEvent::Identify(
                            shard_net::libp2p::identify::Event::Received { peer_id, info, .. },
                        ),
                    ) => {
                        // Only enrich address_book for already-known/connected peers
                        if address_book.contains_key(&peer_id) {
                            for addr in info.listen_addrs {
                                address_book.entry(peer_id).or_default().push(addr);
                            }
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                        debug!("Connection established with {}", peer_id);
                        if let shard_net::libp2p::core::ConnectedPoint::Dialer { address, .. } = &endpoint {
                            address_book.entry(peer_id).or_default().push(address.clone());
                        }
                        node.swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                        if let Some(ref head) = branch::resolve_head(&shard_dir)?.1 {
                            let (fc, ts) = commit_stats(&provider.store, head).unwrap_or_default();
                            let ann = shard_net::protocol::Announcement {
                                commit_id: head.clone(),
                                file_count: fc,
                                total_size: ts,
                                repo_name: repo_name.clone(),
                                peer_multiaddr: if peer_multiaddr.is_empty() {
                                    String::new()
                                } else {
                                    format!("{}/p2p/{}", peer_multiaddr, node.local_peer_id())
                                },
                            };
                            publish_ann(&mut node, &ann);
                        }
                    }
                    shard_net::libp2p::swarm::SwarmEvent::ConnectionClosed { peer_id, num_established: 0, .. } => {
                        node.on_connection_closed(&peer_id);
                    }
                    _ => {}
                }
            }
        }
    };
    gc_task.shutdown().await;
    res
}

pub async fn pull(path: &Path, peer: &str, commit_id: &str, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_pull
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    pull_inner(path, peer, commit_id, json, false).await
}

async fn pull_inner(
    path: &Path,
    peer: &str,
    commit_id: &str,
    json: bool,
    from_sync: bool,
) -> Result<()> {
    validate_object_id(commit_id)?;
    let shard_dir = path.join(".shard");

    if !shard_dir.exists() {
        repo::init(path, "flat", "zstd", "fixed", None, false, false)?;
    }

    // Keep GC and other repository mutations out for the full transfer. The lock
    // must remain held while network fetches and worktree reconstruction are in flight.
    let queue = crate::OP_QUEUES.get_or_create(path);
    let _repo_guard = tokio::task::spawn_blocking(move || {
        queue.acquire(crate::ops::OpKind::Write, "pull".to_string())
    })
    .await??;

    let store = Store::open(&shard_dir)?;
    let cipher = auth::maybe_load_cipher(&shard_dir)?;

    let mut node = shard_net::p2p::Node::new().await?;
    let _ = node.listen("/ip4/0.0.0.0/tcp/0").await;

    let multiaddr: Multiaddr = peer.parse()?;
    let peer_id = match multiaddr.iter().last() {
        Some(shard_net::libp2p::multiaddr::Protocol::P2p(peer_id)) => peer_id,
        _ => anyhow::bail!("Multiaddr must end with /p2p/<peer_id>"),
    };

    // Full Ed25519 challenge-response authentication
    let keypair = auth::load_keypair(&shard_dir)?;
    let public_key = keypair.verifying_key.to_bytes().to_vec();
    node.request_authenticate(&multiaddr, peer_id, public_key, &keypair.signing_key)
        .await?;

    // 1. Fetch & cryptographically verify Commit BEFORE trusting any manifest or writing files
    if !json && !from_sync {
        println!("Pulling commit {} from {}...", commit_id, peer);
    }
    let commit_data = node
        .request_manifest(&multiaddr, peer_id, commit_id.to_string())
        .await?;
    let hash = blake3::hash(&commit_data);
    if hash.to_hex().as_str() != commit_id {
        anyhow::bail!("Commit hash mismatch");
    }
    let commit: Commit = metadata::deserialize(&commit_data)?;

    // Require valid commit signature (and authorized_keys trust when configured)
    auth::verify_commit_signature(&shard_dir, commit_id, &commit, false, false)?;

    store.put_chunk(&Chunk {
        hash,
        data: commit_data,
        offset: 0,
    })?;

    if !json && !from_sync {
        println!("Got commit: {}", commit.message);
    }

    // Set repo_id from commit's public key if missing
    if let Some(pk_hex) = &commit.public_key {
        let pk_bytes = hex::decode(pk_hex)?;
        let repo_id = blake3::hash(&pk_bytes).to_hex().to_string();
        let mut config = crate::load_config_file_only(&shard_dir)?;
        if config.get("repo_id").map(|s| s.as_str()) != Some(repo_id.as_str()) {
            config.insert("repo_id".to_string(), repo_id);
            crate::save_config(&shard_dir, &config)?;
        }
    }

    // 2. Fetch all manifests in parallel & pre-validate every manifest path!
    let manifest_requests: Vec<(String, shard_net::protocol::ShardRequest)> = commit
        .manifests
        .iter()
        .map(|id| {
            validate_object_id(id)?;
            Ok((
                id.clone(),
                shard_net::protocol::ShardRequest::GetManifest(id.clone()),
            ))
        })
        .collect::<Result<Vec<_>>>()?;

    let manifest_results = node
        .request_parallel(&multiaddr, peer_id, manifest_requests)
        .await?;

    let mut all_chunk_ids: Vec<String> = Vec::new();
    let mut file_manifests: Vec<FileManifest> = Vec::new();
    let mut chunk_compression: HashMap<String, Compression> = HashMap::new();

    for (manifest_id, manifest_data) in &manifest_results {
        let hash = blake3::hash(manifest_data);
        if hash.to_hex().as_str() != manifest_id.as_str() {
            anyhow::bail!("Manifest hash mismatch: {}", manifest_id);
        }
        let manifest: FileManifest = metadata::deserialize(manifest_data)?;
        // Reject path traversal in remote manifests immediately!
        let _ = safe_repo_join(path, &manifest.name)?;
        let comp = manifest.compression.parse::<Compression>()?;

        store.put_chunk(&Chunk {
            hash,
            data: manifest_data.clone(),
            offset: 0,
        })?;
        if !json && !from_sync {
            println!(
                "Fetching file: {} (compression: {})",
                manifest.name, manifest.compression
            );
        }
        for cid in &manifest.chunks {
            validate_object_id(cid)?;
            chunk_compression.insert(cid.clone(), comp);
        }
        all_chunk_ids.extend(manifest.chunks.clone());
        file_manifests.push(manifest);
    }

    // 3. Resume: recover valid chunks from partial transfer directory (works with zstd/zlib/private!)
    let partial = partial::PartialTransfer::new(&shard_dir, commit_id)?;
    let partial_chunks = partial.list_chunks()?;
    let mut recovered = 0usize;
    for chunk_id in &partial_chunks {
        if let Ok(data) = partial.load_chunk(chunk_id) {
            let fallback_comp = chunk_compression
                .get(chunk_id)
                .copied()
                .unwrap_or(Compression::None);
            if chunk_codec::decode_chunk(chunk_id, &data, fallback_comp, cipher.as_ref()).is_ok() {
                if let Ok(id_bytes) = hex::decode(chunk_id) {
                    if let Ok(arr) = <[u8; 32]>::try_from(id_bytes.as_slice()) {
                        let hash = blake3::Hash::from_bytes(arr);
                        if store
                            .put_chunk(&Chunk {
                                hash,
                                data,
                                offset: 0,
                            })
                            .is_ok()
                        {
                            recovered += 1;
                        }
                    }
                }
            } else {
                let _ = partial.remove_chunk(chunk_id);
            }
        }
    }
    if recovered > 0 && !json && !from_sync {
        println!("Recovered {} chunks from partial transfer", recovered);
    }

    // 4. Fetch all missing chunks
    let mut seen_needed = HashSet::new();
    let needed_chunks: Vec<String> = all_chunk_ids
        .into_iter()
        .filter(|id| seen_needed.insert(id.clone()) && !store.has_chunk(id))
        .collect();

    if !needed_chunks.is_empty() {
        if !json && !from_sync {
            println!("Fetching {} chunks...", needed_chunks.len());
        }
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
            let fallback_comp = chunk_compression
                .get(chunk_id)
                .copied()
                .unwrap_or(Compression::None);
            // Verify decrypted + decompressed plaintext hashes to `chunk_id`!
            let plaintext =
                chunk_codec::decode_chunk(chunk_id, chunk_data, fallback_comp, cipher.as_ref())?;
            let hash = blake3::hash(&plaintext);
            // Persist partial first, then store
            partial.save_chunk(chunk_id, chunk_data)?;
            store.put_chunk(&Chunk {
                hash,
                data: chunk_data.clone(),
                offset: 0,
            })?;
        }
    }

    store.flush()?;

    // 5. Stream-reconstruct all files safely (no full-file RAM buffering, safe_repo_join enforced)
    for manifest in &file_manifests {
        repo::stream_manifest_to_file(&store, manifest, path, cipher.as_ref())?;
        if !json && !from_sync {
            println!(
                "Reconstructed file: {} ({} bytes)",
                manifest.name, manifest.size
            );
        }
    }

    // 6. Clean up partial transfer tracking
    partial.cleanup()?;

    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "status": "pull complete",
                "commit_id": commit_id,
            }))?
        );
    } else if !from_sync {
        println!("Pull complete.");
    }
    Ok(())
}

pub fn transfer_list(path: &Path, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_transfer_list
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_read_op(path, "transfer_list", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        let transfers = partial::list_incomplete_transfers(&shard_dir)?;
        if json {
            println!("{}", serde_json::to_string(&transfers)?);
        } else if transfers.is_empty() {
            println!("No incomplete transfers.");
        } else {
            for t in &transfers {
                println!("Incomplete transfer: {}", t);
            }
        }
        Ok(())
    })
}

pub fn transfer_remove(path: &Path, commit_id: &str) -> Result<()> {
    crate::metrics::METRICS
        .ops_transfer_remove
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::with_write_op(path, "transfer_remove", || {
        let shard_dir = path.join(".shard");
        if !shard_dir.exists() {
            return Err(ShardError::NotARepo.into());
        }
        partial::remove_transfer(&shard_dir, commit_id)?;
        println!("Removed transfer tracking for {}", commit_id);
        Ok(())
    })
}

/// Stream all reachable object IDs from `HEAD` to `peer` one object at a time (O(1) memory).
pub async fn push(path: &Path, peer: &str, json: bool) -> Result<()> {
    crate::metrics::METRICS
        .ops_push
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        return Err(ShardError::NotARepo.into());
    }

    // Keep local GC and mutations from changing the object graph during transfer.
    let queue = crate::OP_QUEUES.get_or_create(path);
    let _repo_guard = tokio::task::spawn_blocking(move || {
        queue.acquire(crate::ops::OpKind::Read, "push".to_string())
    })
    .await??;

    let (_, head_id) = branch::resolve_head(&shard_dir)?;
    let head_id = head_id.ok_or_else(|| anyhow::anyhow!("No commits to push"))?;
    let store = Store::open(&shard_dir)?;

    // Collect reachable object IDs only (64 bytes each, NOT full blobs!)
    let mut object_ids: Vec<String> = Vec::new();
    let mut seen_objects: HashSet<String> = HashSet::new();
    let mut seen_commits: HashSet<String> = HashSet::new();
    // Rotation objects not yet in the store are streamed from memory instead of
    // written here: this function holds only a read guard on the repository.
    let mut extra_objects: HashMap<String, Vec<u8>> = HashMap::new();
    let mut stack = vec![head_id.clone()];

    while let Some(cid) = stack.pop() {
        if !seen_commits.insert(cid.clone()) {
            continue;
        }
        let data = store.get_chunk(&cid)?;
        if seen_objects.insert(cid.clone()) {
            object_ids.push(cid.clone());
        }
        let commit: Commit = metadata::deserialize(&data)?;
        if let Some(kid) = &commit.key_id {
            let keys_dir = shard_dir.join("keys");
            if let Ok(chain) = keychain::collect_rotation_chain(&keys_dir, kid) {
                for rot in &chain {
                    let rj = serde_json::to_vec(rot)?;
                    let rh = blake3::hash(&rj);
                    let rh_hex = rh.to_hex().to_string();
                    if !store.has_chunk(&rh_hex) {
                        extra_objects.insert(rh_hex.clone(), rj);
                    }
                    if seen_objects.insert(rh_hex.clone()) {
                        object_ids.push(rh_hex);
                    }
                }
            }
        }
        for mid in &commit.manifests {
            let manifest_data = store.get_chunk(mid)?;
            if seen_objects.insert(mid.clone()) {
                object_ids.push(mid.clone());
            }
            let manifest: FileManifest = metadata::deserialize(&manifest_data)?;
            for chunk_id in &manifest.chunks {
                if seen_objects.insert(chunk_id.clone()) {
                    object_ids.push(chunk_id.clone());
                }
            }
        }
        for parent in &commit.parents {
            stack.push(parent.clone());
        }
    }

    if !json {
        println!("Pushing {} objects...", object_ids.len());
    }

    let mut node = shard_net::p2p::Node::new().await?;
    let _ = node.listen("/ip4/0.0.0.0/tcp/0").await;
    let multiaddr: Multiaddr = peer.parse()?;
    let peer_id = match multiaddr.iter().last() {
        Some(shard_net::libp2p::multiaddr::Protocol::P2p(peer_id)) => peer_id,
        _ => anyhow::bail!("Multiaddr must end with /p2p/<peer_id>"),
    };

    let keypair = auth::load_keypair(&shard_dir)?;
    let public_key = keypair.verifying_key.to_bytes().to_vec();
    node.request_authenticate(&multiaddr, peer_id, public_key, &keypair.signing_key)
        .await?;

    // Stream each object directly from the store one at a time
    for id in &object_ids {
        let data = match extra_objects.get(id) {
            Some(data) => data.clone(),
            None => store.get_chunk(id)?,
        };
        node.request_put_chunk(&multiaddr, peer_id, id.clone(), data)
            .await?;
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "status": "push complete",
                "commit_id": head_id,
                "objects": object_ids.len(),
            }))?
        );
    } else {
        println!(
            "Push complete ({} objects, HEAD {}).",
            object_ids.len(),
            head_id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shard_net::p2p::ShardContentProvider;

    #[test]
    fn inbound_object_survives_gc_before_checkout() {
        let temp = tempfile::tempdir().unwrap();
        repo::init(temp.path(), "flat", "none", "fixed", None, false, false).unwrap();
        let shard_dir = temp.path().join(".shard");
        let store = Store::open(&shard_dir).unwrap();
        let mut provider = RepoProvider {
            store,
            shard_dir: shard_dir.clone(),
            cipher: None,
        };
        let plaintext = b"an incoming object that is not yet checked out";
        let object_id = blake3::hash(plaintext).to_hex().to_string();
        let wire_data =
            chunk_codec::encode_chunk(&object_id, plaintext, Compression::None, None).unwrap();

        assert!(provider.put_chunk(&object_id, &wire_data));
        crate::gc::gc(temp.path(), false).unwrap();

        assert_eq!(provider.store.get_chunk(&object_id).unwrap(), wire_data);
        assert!(crate::inbound_pins::active_object_ids(&shard_dir)
            .unwrap()
            .contains(&object_id));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_blocking_in_runtime_does_not_panic_on_current_thread() {
        let mut ran = false;
        run_blocking_in_runtime(|| {
            ran = true;
        });
        assert!(ran);
    }
}
