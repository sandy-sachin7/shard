#![allow(unreachable_code)]

use crate::protocol::{
    build_auth_challenge_payload, is_valid_object_id, ChunkPiece, ShardRequest, ShardResponse,
    MAX_WIRE_CHUNK_SIZE,
};
use anyhow::Result;
use ed25519_dalek::Signer;
use futures::StreamExt;
use libp2p::{
    autonat, dcutr, gossipsub, identify,
    kad::{self, store::MemoryStore},
    mdns, noise, ping, relay,
    request_response::{self, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use rand::RngCore;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio::signal;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

const MAX_RETRIES: u32 = 8;
const AUTH_CHALLENGE_TTL: Duration = Duration::from_secs(30);
const DEFAULT_REQUEST_DEADLINE: Duration = Duration::from_secs(30);

#[allow(unreachable_code)]
#[derive(NetworkBehaviour)]
pub struct ShardBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub request_response: request_response::cbor::Behaviour<ShardRequest, ShardResponse>,
    pub mdns: mdns::tokio::Behaviour,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
    pub relay: relay::Behaviour,
    pub dcutr: dcutr::Behaviour,
    pub autonat: autonat::Behaviour,
}

struct PendingAuth {
    public_key: Vec<u8>,
    nonce: [u8; 32],
    created_at: Instant,
}

pub struct Node {
    pub swarm: Swarm<ShardBehaviour>,
    /// Pending auth challenges keyed by peer_id with expiration TTL.
    pending_auth: HashMap<PeerId, PendingAuth>,
    /// Peers that have successfully completed Ed25519 challenge-response auth.
    authenticated_peers: HashSet<PeerId>,
}

impl Node {
    pub async fn new() -> Result<Self> {
        let swarm = SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                tcp::Config::new().nodelay(true),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_quic()
            .with_dns()?
            .with_behaviour(|key| {
                let local_peer_id = PeerId::from(key.public());
                debug!("Local peer id: {local_peer_id}");

                // Gossipsub — Signed authenticity provides unique per-publish sequence numbers
                // so re-announcements when new peers subscribe are never dropped as duplicates.
                let gossipsub_config = gossipsub::Config::default();
                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )
                .expect("Valid gossipsub behaviour");

                // Kademlia
                let store = MemoryStore::new(local_peer_id);
                let kademlia = kad::Behaviour::new(local_peer_id, store);

                // Request-Response (CBOR for compact binary encoding)
                let request_response = request_response::cbor::Behaviour::new(
                    [(StreamProtocol::new("/shard/1"), ProtocolSupport::Full)],
                    request_response::Config::default()
                        .with_request_timeout(Duration::from_secs(60)),
                );

                // mDNS
                let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)
                    .expect("mDNS start");

                // Identify
                let identify = identify::Behaviour::new(identify::Config::new(
                    "shard/1.0.0".to_string(),
                    key.public(),
                ));

                // Ping (keeps connections alive)
                let ping = ping::Behaviour::new(ping::Config::new());

                // Relay (circuit relay v2)
                let relay = relay::Behaviour::new(local_peer_id, relay::Config::default());

                // DCUtR (Direct Connection Upgrade through Relay)
                let dcutr = dcutr::Behaviour::new(local_peer_id);

                // AutoNAT (NAT type detection)
                let autonat = autonat::Behaviour::new(local_peer_id, autonat::Config::default());

                ShardBehaviour {
                    gossipsub,
                    kademlia,
                    request_response,
                    mdns,
                    identify,
                    ping,
                    relay,
                    dcutr,
                    autonat,
                }
            })?
            .with_swarm_config(|config| {
                config.with_idle_connection_timeout(Duration::from_secs(120))
            })
            .build();
        Ok(Self {
            swarm,
            pending_auth: HashMap::new(),
            authenticated_peers: HashSet::new(),
        })
    }

    /// Clear authentication and pending challenge state when a peer disconnects completely.
    pub fn on_connection_closed(&mut self, peer: &PeerId) {
        self.authenticated_peers.remove(peer);
        self.pending_auth.remove(peer);
    }

    /// Remove expired pending auth challenges to avoid unbounded map growth.
    fn purge_expired_auth(&mut self) {
        self.pending_auth
            .retain(|_, pending| pending.created_at.elapsed() <= AUTH_CHALLENGE_TTL);
    }
}

pub trait ShardContentProvider {
    fn get_manifest(&self, id: &str) -> Option<Vec<u8>>;
    fn get_chunk(&self, id: &str) -> Option<Vec<u8>>;
    fn put_chunk(&mut self, id: &str, data: &[u8]) -> bool;
    /// Verify that `signature` over the domain-separated `challenge_payload` was produced
    /// by the holder of `public_key` AND (when auth is required) that `public_key` is authorized.
    fn verify_auth(&self, public_key: &[u8], challenge_payload: &[u8], signature: &[u8]) -> bool;
    /// Return the repo's public key (if auth is required for this repo).
    fn repo_public_key(&self) -> Option<Vec<u8>>;
    /// Whether this provider requires peers to authenticate before serving content.
    fn auth_required(&self) -> bool {
        false
    }
    /// Check if `public_key` is permitted to authenticate.
    fn authenticate(&self, _peer_id: &PeerId, _public_key: &[u8]) -> bool {
        true
    }
}

impl Node {
    /// Serve a content or authentication request through the given response channel.
    pub fn serve_request(
        &mut self,
        peer: &PeerId,
        provider: &mut impl ShardContentProvider,
        request: ShardRequest,
        channel: request_response::ResponseChannel<ShardResponse>,
    ) {
        match request {
            ShardRequest::GetManifest(id) => {
                if provider.auth_required() && !self.authenticated_peers.contains(peer) {
                    warn!("Unauthenticated peer {} requested manifest", peer);
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::AuthDenied);
                    return;
                }
                if !is_valid_object_id(&id) {
                    warn!("Peer {} requested invalid manifest id", peer);
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::NotFound);
                    return;
                }
                let resp = match provider.get_manifest(&id) {
                    Some(data) => ShardResponse::Manifest(data),
                    None => ShardResponse::NotFound,
                };
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, resp);
            }
            ShardRequest::GetChunk(id) => {
                if provider.auth_required() && !self.authenticated_peers.contains(peer) {
                    warn!("Unauthenticated peer {} requested chunk", peer);
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::AuthDenied);
                    return;
                }
                if !is_valid_object_id(&id) {
                    warn!("Peer {} requested invalid chunk id", peer);
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::NotFound);
                    return;
                }
                let resp = match provider.get_chunk(&id) {
                    Some(data) => ShardResponse::Chunk(ChunkPiece {
                        hash: id,
                        offset: 0,
                        size: data.len() as u64,
                        data,
                    }),
                    None => ShardResponse::NotFound,
                };
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, resp);
            }
            ShardRequest::PutChunk(piece) => {
                if provider.auth_required() && !self.authenticated_peers.contains(peer) {
                    warn!("Unauthenticated peer {} tried to put chunk", peer);
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::AuthDenied);
                    return;
                }
                if !is_valid_object_id(&piece.hash)
                    || piece.data.len() > MAX_WIRE_CHUNK_SIZE
                    || piece.size != piece.data.len() as u64
                {
                    warn!(
                        "Peer {} sent invalid PutChunk (len={}, declared={})",
                        peer,
                        piece.data.len(),
                        piece.size
                    );
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::NotFound);
                    return;
                }
                let ok = provider.put_chunk(&piece.hash, &piece.data);
                let _ = self.swarm.behaviour_mut().request_response.send_response(
                    channel,
                    if ok {
                        ShardResponse::PutAck
                    } else {
                        ShardResponse::NotFound
                    },
                );
            }
            ShardRequest::Authenticate { public_key } => {
                self.purge_expired_auth();
                if public_key.len() != 32 {
                    warn!("Peer {} sent malformed public key during auth", peer);
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::AuthDenied);
                    return;
                }
                if provider.auth_required() && !provider.authenticate(peer, &public_key) {
                    warn!("Peer {} presented unauthorized public key", peer);
                    self.authenticated_peers.remove(peer);
                    self.pending_auth.remove(peer);
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::AuthDenied);
                    return;
                }
                let mut nonce = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut nonce);
                self.pending_auth.insert(
                    *peer,
                    PendingAuth {
                        public_key,
                        nonce,
                        created_at: Instant::now(),
                    },
                );
                let _ = self.swarm.behaviour_mut().request_response.send_response(
                    channel,
                    ShardResponse::AuthChallenge {
                        nonce: nonce.to_vec(),
                    },
                );
            }
            ShardRequest::AuthAnswer { signature } => {
                self.purge_expired_auth();
                let local_peer = self.local_peer_id();
                let verified = self
                    .pending_auth
                    .remove(peer)
                    .map(|pending| {
                        if pending.created_at.elapsed() > AUTH_CHALLENGE_TTL {
                            return false;
                        }
                        if provider.auth_required()
                            && !provider.authenticate(peer, &pending.public_key)
                        {
                            return false;
                        }
                        let payload =
                            build_auth_challenge_payload(peer, &local_peer, &pending.nonce);
                        provider.verify_auth(&pending.public_key, &payload, &signature)
                    })
                    .unwrap_or(false);

                if verified {
                    self.authenticated_peers.insert(*peer);
                } else {
                    self.authenticated_peers.remove(peer);
                }
                let _ = self.swarm.behaviour_mut().request_response.send_response(
                    channel,
                    if verified {
                        ShardResponse::AuthGranted
                    } else {
                        ShardResponse::AuthDenied
                    },
                );
            }
        }
    }

    pub async fn listen(&mut self, addr: &str) -> Result<()> {
        self.swarm.listen_on(addr.parse()?)?;
        Ok(())
    }

    pub async fn run(&mut self, mut provider: impl ShardContentProvider) {
        loop {
            tokio::select! {
                _ = signal::ctrl_c() => {
                    info!("Shutting down...");
                    return;
                }
                event = self.swarm.select_next_some() => {
                    match event {
                        SwarmEvent::NewListenAddr { address, .. } => {
                            info!("Listening on {address:?}");
                        }
                        SwarmEvent::Behaviour(ShardBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                            for (peer_id, multiaddr) in list {
                                debug!("mDNS discovered: {peer_id} {multiaddr}");
                                self.swarm
                                    .behaviour_mut()
                                    .gossipsub
                                    .add_explicit_peer(&peer_id);
                                self.swarm
                                    .behaviour_mut()
                                    .kademlia
                                    .add_address(&peer_id, multiaddr);
                            }
                        }
                        SwarmEvent::Behaviour(ShardBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                            for (peer_id, _multiaddr) in list {
                                debug!("mDNS expired: {peer_id}");
                                self.swarm
                                    .behaviour_mut()
                                    .gossipsub
                                    .remove_explicit_peer(&peer_id);
                            }
                        }
                        SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                            request_response::Event::Message { peer, message, .. },
                        )) => match message {
                            request_response::Message::Request {
                                request, channel, ..
                            } => {
                                debug!("Received request from {}", peer);
                                self.serve_request(&peer, &mut provider, request, channel);
                            }
                            request_response::Message::Response { .. } => {
                                debug!("Received Response from {}", peer);
                            }
                        },
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            debug!("Connection established with {}", peer_id);
                            self.swarm
                                .behaviour_mut()
                                .gossipsub
                                .add_explicit_peer(&peer_id);
                        }
                        SwarmEvent::ConnectionClosed { peer_id, num_established: 0, .. } => {
                            debug!("All connections closed with {}", peer_id);
                            self.on_connection_closed(&peer_id);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    pub async fn request_manifest(
        &mut self,
        multiaddr: &libp2p::Multiaddr,
        peer: PeerId,
        id: String,
    ) -> Result<Vec<u8>> {
        self.dial_send_wait(multiaddr, peer, ShardRequest::GetManifest(id))
            .await
    }

    pub async fn request_chunk(
        &mut self,
        multiaddr: &libp2p::Multiaddr,
        peer: PeerId,
        id: String,
    ) -> Result<Vec<u8>> {
        self.dial_send_wait(multiaddr, peer, ShardRequest::GetChunk(id))
            .await
    }

    /// Ensure we have an active connection to `peer`, dialing only if not already connected.
    async fn ensure_connected(
        &mut self,
        multiaddr: &libp2p::Multiaddr,
        peer: PeerId,
    ) -> Result<()> {
        self.swarm.add_peer_address(peer, multiaddr.clone());
        if self.swarm.is_connected(&peer) {
            return Ok(());
        }

        let mut attempts = 0u32;
        let deadline = tokio::time::Instant::now() + DEFAULT_REQUEST_DEADLINE;

        loop {
            if attempts > 0 {
                if attempts >= MAX_RETRIES {
                    anyhow::bail!(
                        "max retries ({}) exceeded connecting to {}",
                        MAX_RETRIES,
                        peer
                    );
                }
                let delay = Duration::from_millis(100 * (1u64 << (attempts - 1).min(5)));
                sleep(delay).await;
            }
            if self.swarm.is_connected(&peer) {
                return Ok(());
            }
            self.swarm.dial(multiaddr.clone())?;
            attempts += 1;

            loop {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    anyhow::bail!("timed out connecting to {}", peer);
                }
                let event =
                    match tokio::time::timeout(remaining, self.swarm.select_next_some()).await {
                        Ok(ev) => ev,
                        Err(_) => anyhow::bail!("timed out connecting to {}", peer),
                    };
                match event {
                    SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == peer => {
                        return Ok(());
                    }
                    SwarmEvent::OutgoingConnectionError {
                        peer_id: Some(p),
                        error,
                        ..
                    } if p == peer => {
                        debug!("Outgoing connection error to {}: {:?}", p, error);
                        break; // retry with backoff
                    }
                    SwarmEvent::OutgoingConnectionError {
                        peer_id: None,
                        error,
                        ..
                    } => {
                        debug!("Outgoing connection error: {:?}", error);
                        break; // retry with backoff
                    }
                    SwarmEvent::ConnectionClosed {
                        peer_id,
                        num_established: 0,
                        ..
                    } => {
                        self.on_connection_closed(&peer_id);
                    }
                    _ => {}
                }
            }
        }
    }

    pub async fn request_put_chunk(
        &mut self,
        multiaddr: &libp2p::Multiaddr,
        peer: PeerId,
        id: String,
        data: Vec<u8>,
    ) -> Result<()> {
        self.ensure_connected(multiaddr, peer).await?;
        let rid = self.swarm.behaviour_mut().request_response.send_request(
            &peer,
            ShardRequest::PutChunk(ChunkPiece {
                hash: id.clone(),
                offset: 0,
                size: data.len() as u64,
                data,
            }),
        );
        let deadline = tokio::time::Instant::now() + DEFAULT_REQUEST_DEADLINE;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                anyhow::bail!("timed out waiting for PutAck for {} from {}", id, peer);
            }
            let event = match tokio::time::timeout(remaining, self.swarm.select_next_some()).await {
                Ok(ev) => ev,
                Err(_) => anyhow::bail!("timed out waiting for PutAck for {} from {}", id, peer),
            };
            match event {
                SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    request_response::Event::Message {
                        message:
                            request_response::Message::Response {
                                request_id,
                                response,
                            },
                        ..
                    },
                )) if request_id == rid => {
                    return match response {
                        ShardResponse::PutAck => Ok(()),
                        ShardResponse::AuthDenied => {
                            anyhow::bail!("Authentication denied by peer")
                        }
                        ShardResponse::NotFound => anyhow::bail!("Peer rejected chunk {}", id),
                        _ => anyhow::bail!("Unexpected response to put"),
                    };
                }
                SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    request_response::Event::OutboundFailure {
                        peer: p,
                        request_id,
                        error,
                        ..
                    },
                )) if p == peer && request_id == rid => {
                    anyhow::bail!("Outbound failure to {}: {:?}", peer, error);
                }
                SwarmEvent::ConnectionClosed {
                    peer_id,
                    num_established: 0,
                    cause,
                    ..
                } if peer_id == peer => {
                    self.on_connection_closed(&peer_id);
                    anyhow::bail!("Connection closed to {} during put: {:?}", peer_id, cause);
                }
                _ => {}
            }
        }
    }

    /// Perform full Ed25519 challenge-response authentication handshake with the peer.
    /// Sends public_key, receives 32-byte nonce challenge, signs domain-separated payload
    /// bound to both PeerIds, and waits for AuthGranted.
    pub async fn request_authenticate(
        &mut self,
        multiaddr: &libp2p::Multiaddr,
        peer: PeerId,
        public_key: Vec<u8>,
        signing_key: &ed25519_dalek::SigningKey,
    ) -> Result<()> {
        self.ensure_connected(multiaddr, peer).await?;
        let local_peer = self.local_peer_id();
        let deadline = tokio::time::Instant::now() + DEFAULT_REQUEST_DEADLINE;

        let mut rid = self
            .swarm
            .behaviour_mut()
            .request_response
            .send_request(&peer, ShardRequest::Authenticate { public_key });
        let mut state = 1u8;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                anyhow::bail!("Authentication timed out with peer {}", peer);
            }
            let event = match tokio::time::timeout(remaining, self.swarm.select_next_some()).await {
                Ok(ev) => ev,
                Err(_) => anyhow::bail!("Authentication timed out with peer {}", peer),
            };
            match event {
                SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    request_response::Event::Message {
                        message:
                            request_response::Message::Response {
                                request_id,
                                response,
                            },
                        ..
                    },
                )) if request_id == rid => match (state, response) {
                    (1, ShardResponse::AuthChallenge { nonce }) => {
                        if nonce.len() != 32 {
                            anyhow::bail!("Invalid auth challenge nonce length: {}", nonce.len());
                        }
                        let payload = build_auth_challenge_payload(&local_peer, &peer, &nonce);
                        let signature = signing_key.sign(&payload);
                        rid = self.swarm.behaviour_mut().request_response.send_request(
                            &peer,
                            ShardRequest::AuthAnswer {
                                signature: signature.to_bytes().to_vec(),
                            },
                        );
                        state = 2;
                    }
                    (1, ShardResponse::AuthDenied) => {
                        anyhow::bail!("Authentication denied by peer");
                    }
                    (2, ShardResponse::AuthGranted) => return Ok(()),
                    (2, ShardResponse::AuthDenied) => {
                        anyhow::bail!("Authentication denied by peer");
                    }
                    (_, other) => anyhow::bail!("Unexpected auth response: {:?}", other),
                },
                SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    request_response::Event::OutboundFailure {
                        peer: p,
                        request_id,
                        error,
                        ..
                    },
                )) if p == peer && request_id == rid => {
                    anyhow::bail!("Outbound failure during auth to {}: {:?}", peer, error);
                }
                SwarmEvent::ConnectionClosed {
                    peer_id,
                    num_established: 0,
                    cause,
                    ..
                } if peer_id == peer => {
                    self.on_connection_closed(&peer_id);
                    anyhow::bail!("Connection closed to {} during auth: {:?}", peer_id, cause);
                }
                _ => {}
            }
        }
    }

    async fn dial_send_wait(
        &mut self,
        multiaddr: &libp2p::Multiaddr,
        peer: PeerId,
        request: ShardRequest,
    ) -> Result<Vec<u8>> {
        self.ensure_connected(multiaddr, peer).await?;
        let rid = self
            .swarm
            .behaviour_mut()
            .request_response
            .send_request(&peer, request);
        let deadline = tokio::time::Instant::now() + DEFAULT_REQUEST_DEADLINE;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                anyhow::bail!("Request timed out to {}", peer);
            }
            let event = match tokio::time::timeout(remaining, self.swarm.select_next_some()).await {
                Ok(ev) => ev,
                Err(_) => anyhow::bail!("Request timed out to {}", peer),
            };
            match event {
                SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    request_response::Event::Message {
                        message:
                            request_response::Message::Response {
                                request_id,
                                response,
                            },
                        ..
                    },
                )) if request_id == rid => {
                    return match response {
                        ShardResponse::Manifest(data) => Ok(data),
                        ShardResponse::Chunk(piece) => Ok(piece.data),
                        ShardResponse::AuthDenied => {
                            anyhow::bail!("Authentication denied by peer")
                        }
                        ShardResponse::NotFound => anyhow::bail!("Not found"),
                        _ => anyhow::bail!("Unexpected response"),
                    };
                }
                SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    request_response::Event::OutboundFailure {
                        peer: p,
                        request_id,
                        error,
                        ..
                    },
                )) if p == peer && request_id == rid => {
                    anyhow::bail!("Outbound failure to {}: {:?}", peer, error);
                }
                SwarmEvent::ConnectionClosed {
                    peer_id,
                    num_established: 0,
                    cause,
                    ..
                } if peer_id == peer => {
                    self.on_connection_closed(&peer_id);
                    anyhow::bail!("Connection closed to {}: {:?}", peer_id, cause);
                }
                SwarmEvent::Behaviour(ShardBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                    for (peer_id, multiaddr) in list {
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .add_explicit_peer(&peer_id);
                        self.swarm
                            .behaviour_mut()
                            .kademlia
                            .add_address(&peer_id, multiaddr);
                    }
                }
                _ => {}
            }
        }
    }

    pub async fn request_parallel(
        &mut self,
        multiaddr: &libp2p::Multiaddr,
        peer: PeerId,
        requests: Vec<(String, ShardRequest)>,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        self.ensure_connected(multiaddr, peer).await?;

        let mut request_map: HashMap<request_response::OutboundRequestId, String> = HashMap::new();
        let mut results: Vec<(String, Vec<u8>)> = Vec::with_capacity(requests.len());

        for (id, req) in &requests {
            let rid = self
                .swarm
                .behaviour_mut()
                .request_response
                .send_request(&peer, req.clone());
            request_map.insert(rid, id.clone());
        }

        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(30 + (requests.len() as u64).min(600));

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                anyhow::bail!(
                    "Parallel request timed out with {} pending items from {}",
                    request_map.len(),
                    peer
                );
            }
            let event = match tokio::time::timeout(remaining, self.swarm.select_next_some()).await {
                Ok(ev) => ev,
                Err(_) => anyhow::bail!(
                    "Parallel request timed out with {} pending items from {}",
                    request_map.len(),
                    peer
                ),
            };
            match event {
                SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    request_response::Event::Message {
                        message:
                            request_response::Message::Response {
                                request_id,
                                response,
                            },
                        ..
                    },
                )) => {
                    if let Some(original_id) = request_map.remove(&request_id) {
                        match response {
                            ShardResponse::Manifest(data) => {
                                results.push((original_id, data));
                            }
                            ShardResponse::Chunk(piece) => {
                                results.push((original_id, piece.data));
                            }
                            ShardResponse::AuthDenied => {
                                anyhow::bail!("Authentication denied by peer for: {}", original_id);
                            }
                            ShardResponse::NotFound => {
                                anyhow::bail!("Object not found: {}", original_id);
                            }
                            _ => {
                                anyhow::bail!("Unexpected response for: {}", original_id);
                            }
                        }
                        if request_map.is_empty() {
                            return Ok(results);
                        }
                    }
                }
                SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    request_response::Event::OutboundFailure {
                        peer: p,
                        request_id,
                        error,
                        ..
                    },
                )) if p == peer => {
                    if let Some(id) = request_map.remove(&request_id) {
                        anyhow::bail!("Failed to fetch {}: {:?}", id, error);
                    }
                }
                SwarmEvent::ConnectionClosed {
                    peer_id,
                    num_established: 0,
                    cause,
                    ..
                } if peer_id == peer => {
                    self.on_connection_closed(&peer_id);
                    error!(
                        "Connection closed to {} during parallel fetch: {:?}",
                        peer_id, cause
                    );
                    anyhow::bail!(
                        "Connection closed to {} with {} requests remaining",
                        peer_id,
                        request_map.len()
                    );
                }
                _ => {}
            }
        }
    }

    pub fn subscribe(&mut self, topic: &gossipsub::IdentTopic) -> Result<()> {
        self.swarm.behaviour_mut().gossipsub.subscribe(topic)?;
        Ok(())
    }

    pub fn publish(
        &mut self,
        topic: &gossipsub::IdentTopic,
        data: impl Into<Vec<u8>>,
    ) -> Result<()> {
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic.clone(), data)?;
        Ok(())
    }

    pub fn local_peer_id(&self) -> PeerId {
        *self.swarm.local_peer_id()
    }

    pub async fn wait_for_peer(&mut self, peer: PeerId) -> Result<()> {
        if self.swarm.is_connected(&peer) {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + DEFAULT_REQUEST_DEADLINE;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                anyhow::bail!("Timed out waiting for peer {}", peer);
            }
            let event = match tokio::time::timeout(remaining, self.swarm.select_next_some()).await {
                Ok(ev) => ev,
                Err(_) => anyhow::bail!("Timed out waiting for peer {}", peer),
            };
            match event {
                SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == peer => {
                    debug!("Connection established with {}", peer);
                    return Ok(());
                }
                SwarmEvent::OutgoingConnectionError {
                    peer_id: Some(p),
                    error,
                    ..
                } if p == peer => {
                    anyhow::bail!("Failed to connect to {}: {:?}", p, error);
                }
                SwarmEvent::Behaviour(ShardBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                    for (peer_id, multiaddr) in list {
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .add_explicit_peer(&peer_id);
                        self.swarm
                            .behaviour_mut()
                            .kademlia
                            .add_address(&peer_id, multiaddr);
                    }
                }
                _ => {}
            }
        }
    }
}
