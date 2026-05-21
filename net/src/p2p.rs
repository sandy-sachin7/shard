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
    pub async fn listen(&mut self, addr: &str) -> Result<()> {
        self.swarm.listen_on(addr.parse()?)?;
        Ok(())
    }

    pub async fn run(&mut self, provider: impl ShardContentProvider) {
        loop {
            match self.swarm.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } => println!("Listening on {address:?}"),
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
                    } => match request {
                        ShardRequest::GetManifest(id) => {
                            println!("Received GetManifest({}) from {}", id, peer);
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
                            println!("Received GetChunk({}) from {}", id, peer);
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
                    },
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
