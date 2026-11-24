//! Peer-to-peer networking layer for the Shard distributed VCS.
//!
//! Built on `libp2p` with TCP + QUIC transports, Noise encryption, Yamux multiplexing,
//! mDNS LAN discovery, Kademlia DHT, Gossipsub commit announcements, CBOR
//! request-response object transfers, Circuit Relay v2, DCUtR hole-punching, AutoNAT,
//! and a 2-step domain-separated Ed25519 challenge-response authentication protocol.
//!
//! # Quickstart
//!
//! ```rust,no_run
//! use shard_net::p2p::Node;
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! let mut node = Node::new().await?;
//! node.listen("/ip4/127.0.0.1/tcp/0").await?;
//! println!("Local PeerId: {}", node.local_peer_id());
//! # Ok(())
//! # }
//! ```

pub mod p2p;
pub mod protocol;

pub use libp2p;
