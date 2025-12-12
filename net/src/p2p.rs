use libp2p::{
    core::upgrade,
    gossipsub,
    kad::{self, store::MemoryStore},
    mdns,
    noise,
    request_response::{self, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp,
    yamux,
    Swarm, SwarmBuilder,
    PeerId,
    Transport,
    identity,
    StreamProtocol,
};
use std::time::Duration;
use futures::StreamExt;
use crate::protocol::{ShardRequest, ShardResponse};
use anyhow::Result;

#[derive(NetworkBehaviour)]
pub struct ShardBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub request_response: request_response::cbor::Behaviour<ShardRequest, ShardResponse>,
    pub mdns: mdns::tokio::Behaviour,
}

pub struct Node {
    pub swarm: Swarm<ShardBehaviour>,
}

impl Node {
    pub async fn new() -> Result<Self> {
        let swarm = SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
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
                ).expect("Valid gossipsub config");

                // Kademlia
                let store = MemoryStore::new(local_peer_id);
                let kademlia = kad::Behaviour::new(local_peer_id, store);

                // Request-Response
                let request_response = request_response::cbor::Behaviour::new(
                    [(StreamProtocol::new("/shard/1"), ProtocolSupport::Full)],
                    request_response::Config::default(),
                );

                // mDNS
                let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id).expect("mDNS start");

                ShardBehaviour {
                    gossipsub,
                    kademlia,
                    request_response,
                    mdns,
                }
            })?
            .build();

        Ok(Self { swarm })
    }

    pub async fn listen(&mut self, addr: &str) -> Result<()> {
        self.swarm.listen_on(addr.parse()?)?;
        Ok(())
    }

    pub async fn run(&mut self) {
        loop {
            match self.swarm.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } => println!("Listening on {address:?}"),
                SwarmEvent::Behaviour(event) => {
                    // Handle events
                    match event {
                        ShardBehaviourEvent::Mdns(mdns::Event::Discovered(list)) => {
                            for (peer_id, multiaddr) in list {
                                println!("mDNS discovered: {peer_id} {multiaddr}");
                                self.swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                                self.swarm.behaviour_mut().kademlia.add_address(&peer_id, multiaddr);
                            }
                        }
                        ShardBehaviourEvent::Mdns(mdns::Event::Expired(list)) => {
                             for (peer_id, _multiaddr) in list {
                                println!("mDNS expired: {peer_id}");
                                self.swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
}
