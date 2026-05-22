use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;

use crate::protocol::{ShardRequest, ShardResponse};
use anyhow::Result;
use futures::StreamExt;
use libp2p::{
    gossipsub,
    kad::{self, store::MemoryStore},
    mdns, noise, ping,
    request_response::{self, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use tokio::signal;

#[derive(NetworkBehaviour)]
pub struct ShardBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub request_response: request_response::cbor::Behaviour<ShardRequest, ShardResponse>,
    pub mdns: mdns::tokio::Behaviour,
    pub identify: libp2p::identify::Behaviour,
    pub ping: ping::Behaviour,
}

pub struct Node {
    pub swarm: Swarm<ShardBehaviour>,
}

impl Node {
    pub async fn new() -> Result<Self> {
        let swarm = SwarmBuilder::with_new_identity()
            .with_tokio()
            // .with_quic()
            .with_tcp(
                tcp::Config::new().nodelay(true),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|key| {
                let local_peer_id = PeerId::from(key.public());
                println!("Local peer id: {local_peer_id}");
                let _ = std::io::stdout().flush();

                // Gossipsub
                let gossipsub_config = gossipsub::Config::default();
                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )
                .expect("Valid gossipsub config");

                // Kademlia
                let store = MemoryStore::new(local_peer_id);
                let kademlia = kad::Behaviour::new(local_peer_id, store);

                // Request-Response (CBOR for compact binary encoding)
                let request_response = request_response::cbor::Behaviour::new(
                    [(StreamProtocol::new("/shard/1"), ProtocolSupport::Full)],
                    request_response::Config::default(),
                );

                // mDNS
                let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)
                    .expect("mDNS start");

                // Identify
                let identify = libp2p::identify::Behaviour::new(libp2p::identify::Config::new(
                    "shard/1.0.0".to_string(),
                    key.public(),
                ));

                // Ping (keeps connections alive)
                let ping = ping::Behaviour::new(ping::Config::new());

                ShardBehaviour {
                    gossipsub,
                    kademlia,
                    request_response,
                    mdns,
                    identify,
                    ping,
                }
            })?
            .with_swarm_config(|config| {
                config.with_idle_connection_timeout(Duration::from_secs(120))
            })
            .build();

        Ok(Self { swarm })
    }
}

pub trait ShardContentProvider {
    fn get_manifest(&self, id: &str) -> Option<Vec<u8>>;
    fn get_chunk(&self, id: &str) -> Option<Vec<u8>>;
}

impl Node {
    /// Serve a content request through the given response channel.
    pub fn serve_request(
        &mut self,
        provider: &impl ShardContentProvider,
        request: ShardRequest,
        channel: request_response::ResponseChannel<ShardResponse>,
    ) {
        match request {
            ShardRequest::GetManifest(id) => {
                if let Some(data) = provider.get_manifest(&id) {
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::Manifest(data));
                } else {
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::NotFound);
                }
            }
            ShardRequest::GetChunk(id) => {
                if let Some(data) = provider.get_chunk(&id) {
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::Chunk(data));
                } else {
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::NotFound);
                }
            }
        }
    }

    pub async fn listen(&mut self, addr: &str) -> Result<()> {
        self.swarm.listen_on(addr.parse()?)?;
        Ok(())
    }

    pub async fn run(&mut self, provider: impl ShardContentProvider) {
        loop {
            tokio::select! {
                _ = signal::ctrl_c() => {
                    println!("\nShutting down...");
                    let _ = std::io::stdout().flush();
                    return;
                }
                event = self.swarm.select_next_some() => {
                    match event {
                        SwarmEvent::NewListenAddr { address, .. } => {
                            println!("Listening on {address:?}");
                            let _ = std::io::stdout().flush();
                        }
                        SwarmEvent::Behaviour(ShardBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                            for (peer_id, multiaddr) in list {
                                println!("mDNS discovered: {peer_id} {multiaddr}");
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
                                println!("mDNS expired: {peer_id}");
                                self.swarm
                                    .behaviour_mut()
                                    .gossipsub
                                    .remove_explicit_peer(&peer_id);
                            }
                        }
                        SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                            request_response::Event::Message { peer, message },
                        )) => match message {
                            request_response::Message::Request {
                                request, channel, ..
                            } => {
                                println!("Received request from {}", peer);
                                self.serve_request(&provider, request, channel);
                            }
                            request_response::Message::Response { .. } => {
                                println!("Received Response from {}", peer);
                            }
                        },
                        SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                            request_response::Event::OutboundFailure { peer, error, .. },
                        )) => {
                            println!("Outbound failure to {}: {:?}", peer, error);
                        }
                        SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                            request_response::Event::InboundFailure { peer, error, .. },
                        )) => {
                            println!("Inbound failure from {}: {:?}", peer, error);
                        }
                        SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                            request_response::Event::ResponseSent { peer, .. },
                        )) => {
                            println!("Response sent to {}", peer);
                        }
                        SwarmEvent::Behaviour(ShardBehaviourEvent::Identify(event)) => {
                            println!("Identify event: {:?}", event);
                        }
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            println!("Connection established with {}", peer_id);
                            self.swarm
                                .behaviour_mut()
                                .gossipsub
                                .add_explicit_peer(&peer_id);
                        }
                        SwarmEvent::IncomingConnection {
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

    async fn dial_send_wait(
        &mut self,
        multiaddr: &libp2p::Multiaddr,
        peer: PeerId,
        request: ShardRequest,
    ) -> Result<Vec<u8>> {
        self.swarm.add_peer_address(peer, multiaddr.clone());
        self.swarm.dial(multiaddr.clone())?;
        let mut request_id = None;

        loop {
            match self.swarm.select_next_some().await {
                SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == peer => {
                    let rid = self
                        .swarm
                        .behaviour_mut()
                        .request_response
                        .send_request(&peer, request.clone());
                    request_id = Some(rid);
                }
                SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    request_response::Event::Message { message, .. },
                )) => {
                    if let Some(rid) = &request_id {
                        if let request_response::Message::Response {
                            request_id: actual_rid,
                            response,
                        } = message
                        {
                            if *rid == actual_rid {
                                return match response {
                                    ShardResponse::Manifest(data) | ShardResponse::Chunk(data) => {
                                        Ok(data)
                                    }
                                    ShardResponse::NotFound => anyhow::bail!("Not found"),
                                };
                            }
                        }
                    }
                }
                SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    request_response::Event::OutboundFailure { peer, error, .. },
                )) => {
                    anyhow::bail!("Outbound failure to {}: {:?}", peer, error);
                }
                SwarmEvent::ConnectionClosed { peer_id, cause, .. } if peer_id == peer => {
                    eprintln!("Connection closed to {}: {:?}", peer_id, cause);
                    let _ = self.swarm.dial(multiaddr.clone());
                }
                SwarmEvent::OutgoingConnectionError {
                    peer_id: Some(p), ..
                } if p == peer => {
                    eprintln!("Outgoing connection error: {:?}", p);
                    let _ = self.swarm.dial(multiaddr.clone());
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
        self.swarm.add_peer_address(peer, multiaddr.clone());
        self.swarm.dial(multiaddr.clone())?;

        let mut request_map: HashMap<libp2p::request_response::OutboundRequestId, String> =
            HashMap::new();
        let mut results: Vec<(String, Vec<u8>)> = Vec::with_capacity(requests.len());
        let mut sent = false;

        loop {
            match self.swarm.select_next_some().await {
                SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == peer => {
                    sent = true;
                    for (id, req) in &requests {
                        let rid = self
                            .swarm
                            .behaviour_mut()
                            .request_response
                            .send_request(&peer, req.clone());
                        request_map.insert(rid, id.clone());
                    }
                }
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
                            ShardResponse::Manifest(data) | ShardResponse::Chunk(data) => {
                                results.push((original_id, data));
                            }
                            ShardResponse::NotFound => {
                                anyhow::bail!("Object not found: {}", original_id);
                            }
                        }
                        if request_map.is_empty() && sent {
                            return Ok(results);
                        }
                    }
                }
                SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    request_response::Event::OutboundFailure {
                        peer: p,
                        request_id,
                        error,
                    },
                )) if p == peer => {
                    if let Some(id) = request_map.remove(&request_id) {
                        anyhow::bail!("Failed to fetch {}: {:?}", id, error);
                    }
                }
                SwarmEvent::OutgoingConnectionError {
                    peer_id: Some(p), ..
                } if p == peer => {
                    let _ = self.swarm.dial(multiaddr.clone());
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
        loop {
            match self.swarm.select_next_some().await {
                SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == peer => {
                    println!("Connection established with {}", peer);
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
                SwarmEvent::Behaviour(ShardBehaviourEvent::Identify(event)) => {
                    println!("Identify event: {:?}", event);
                }
                _ => {}
            }
        }
    }
}
