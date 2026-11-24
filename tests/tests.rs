use ed25519_dalek::Signer;
use shard_core::chunker::Chunk;
use shard_core::commit::Commit;
use shard_core::manifest::FileManifest;
use shard_core::metadata::{self, MetadataFormat};
use shard_core::store::Store;
use std::fs;

fn init_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().to_path_buf();
    shard_core::init(&repo, "flat", "zstd", "fixed", None, false, false).unwrap();
    (dir, repo)
}

#[test]
fn workspace_init_smoke() {
    let (_dir, repo) = init_repo();
    assert!(repo.join(".shard").exists());
    assert!(repo.join(".shard/config.json").exists());
}

#[test]
fn workspace_add_commit_verify_roundtrip() {
    let (_dir, repo) = init_repo();
    fs::write(repo.join("hello.txt"), b"workspace integration test").unwrap();
    shard_core::add(&repo, &repo.join("hello.txt"), false).unwrap();
    let cid = shard_core::commit(&repo, "test", "Test <test@test.com>", false).unwrap();

    let (_, head) = shard_core::branch::resolve_head(&repo.join(".shard")).unwrap();
    assert_eq!(head.as_deref(), Some(cid.as_str()));
    shard_core::verify(&repo, &cid, false).unwrap();
}

#[test]
fn workspace_multi_file_commit() {
    let (_dir, repo) = init_repo();
    fs::write(repo.join("a.txt"), b"file a").unwrap();
    fs::write(repo.join("b.txt"), b"file b").unwrap();
    fs::write(repo.join("c.txt"), b"file c").unwrap();

    shard_core::add(&repo, &repo.join("a.txt"), false).unwrap();
    shard_core::add(&repo, &repo.join("b.txt"), false).unwrap();
    shard_core::add(&repo, &repo.join("c.txt"), false).unwrap();
    let cid = shard_core::commit(&repo, "multi", "Test <test@test.com>", false).unwrap();
    shard_core::verify(&repo, &cid, false).unwrap();
}

#[test]
fn workspace_empty_commit_fails() {
    let (_dir, repo) = init_repo();
    let result = shard_core::commit(&repo, "empty", "Test <test@test.com>", false);
    assert!(result.is_err());
}

#[test]
fn workspace_log_after_commit() {
    let (_dir, repo) = init_repo();
    fs::write(repo.join("f.txt"), b"data").unwrap();
    shard_core::add(&repo, &repo.join("f.txt"), false).unwrap();
    shard_core::commit(&repo, "first", "A <a@a.com>", false).unwrap();
    shard_core::log_cmd(&repo, false).unwrap();
}

#[test]
fn workspace_status_after_init() {
    let (_dir, repo) = init_repo();
    shard_core::status(&repo, false, false).unwrap();
}

#[test]
fn workspace_checkout_after_commit() {
    let (_dir, repo) = init_repo();
    fs::write(repo.join("checkme.txt"), b"checkout data").unwrap();
    shard_core::add(&repo, &repo.join("checkme.txt"), false).unwrap();
    let head = shard_core::commit(&repo, "checkout test", "Test <test@test.com>", false).unwrap();

    fs::remove_file(repo.join("checkme.txt")).unwrap();
    shard_core::checkout(&repo, &head, false).unwrap();
    assert_eq!(
        fs::read_to_string(repo.join("checkme.txt")).unwrap(),
        "checkout data"
    );
}

#[test]
fn workspace_prune_no_crash() {
    let (_dir, repo) = init_repo();
    fs::write(repo.join("p.txt"), b"prune me").unwrap();
    shard_core::add(&repo, &repo.join("p.txt"), false).unwrap();
    shard_core::commit(&repo, "prune test", "Test <test@test.com>", false).unwrap();
    shard_core::prune(&repo, false).unwrap();
}

#[test]
fn workspace_config_get_set() {
    let (_dir, repo) = init_repo();
    shard_core::config_set(&repo, "test.key", "test.value").unwrap();
    shard_core::config_get(&repo, Some("test.key")).unwrap();
    // Invalid config value must be rejected by validate_config
    assert!(shard_core::config_set(&repo, "compression", "gzip").is_err());
    assert!(shard_core::config_set(&repo, "compression", "zlib").is_ok());
}

#[test]
fn workspace_branch_create_switch_list() {
    let (_dir, repo) = init_repo();
    fs::write(repo.join("main.txt"), b"main").unwrap();
    shard_core::add(&repo, &repo.join("main.txt"), false).unwrap();
    shard_core::commit(&repo, "first", "Test <test@test.com>", false).unwrap();
    shard_core::branch_create(&repo, "feature/sub", None).unwrap();
    fs::write(repo.join("feature.txt"), b"feature").unwrap();
    shard_core::add(&repo, &repo.join("feature.txt"), false).unwrap();
    shard_core::commit(&repo, "feature work", "Test <test@test.com>", false).unwrap();
    shard_core::branch_list(&repo).unwrap();
}

#[test]
fn workspace_tag_add_list() {
    let (_dir, repo) = init_repo();
    fs::write(repo.join("t.txt"), b"tag").unwrap();
    shard_core::add(&repo, &repo.join("t.txt"), false).unwrap();
    let head = shard_core::commit(&repo, "tag test", "Test <test@test.com>", false).unwrap();
    shard_core::tag_add(&repo, "v1.0", &head).unwrap();
    shard_core::tag_list(&repo).unwrap();
}

#[test]
fn workspace_recover_no_crash() {
    let (_dir, repo) = init_repo();
    shard_core::recover(&repo, false).unwrap();
}

#[test]
fn workspace_private_init_creates_key() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    shard_core::init(repo, "flat", "zstd", "fixed", None, true, false).unwrap();
    let key_path = repo.join(".shard/keys/repo.key");
    assert!(key_path.exists());
    let key_hex = fs::read_to_string(&key_path).unwrap();
    assert_eq!(key_hex.trim().len(), 64);
}

#[test]
fn workspace_private_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    shard_core::init(repo, "flat", "zstd", "fixed", None, true, false).unwrap();
    fs::write(repo.join("secret.txt"), b"private data").unwrap();
    shard_core::add(repo, &repo.join("secret.txt"), false).unwrap();
    let head = shard_core::commit(repo, "private", "Test <test@test.com>", false).unwrap();
    shard_core::verify(repo, &head, false).unwrap();
}

#[test]
fn workspace_verify_rejects_unsigned_and_untrusted_forged_commit() {
    let (_dir, repo) = init_repo();
    let shard_dir = repo.join(".shard");
    let store = Store::open(&shard_dir).unwrap();

    // 1. Forge a commit signed by an attacker keypair (not in local keychain or authorized_keys)
    let attacker = shard_crypto::KeyPair::generate();
    let mut forged = Commit {
        commit_id: String::new(),
        parents: vec![],
        manifests: vec![],
        author: "Attacker".into(),
        message: "forged".into(),
        timestamp: 1000,
        public_key: Some(hex::encode(attacker.verifying_key.to_bytes())),
        signature: None,
        key_id: Some(shard_core::keychain::key_id_from_public_key(
            &attacker.verifying_key,
        )),
    };
    let unsigned_bytes = metadata::serialize_for_signing(&forged);
    let sig = attacker.signing_key.sign(&unsigned_bytes);
    forged.signature = Some(hex::encode(sig.to_bytes()));
    let forged_bytes = metadata::serialize(&forged, &MetadataFormat::Json);
    let forged_hash = blake3::hash(&forged_bytes);
    let forged_id = forged_hash.to_hex().to_string();
    store
        .put_chunk(&Chunk {
            hash: forged_hash,
            data: forged_bytes,
            offset: 0,
        })
        .unwrap();

    // Must fail because attacker key is NOT anchored in local keychain or authorized_keys!
    assert!(shard_core::verify(&repo, &forged_id, false).is_err());

    // Once explicitly added to authorized_keys, verify succeeds
    shard_core::add_authorized_key(&shard_dir, &hex::encode(attacker.verifying_key.to_bytes()))
        .unwrap();
    shard_core::verify(&repo, &forged_id, false).unwrap();

    // 2. Unsigned commit must fail by default, succeed only with allow_unsigned=true
    let unsigned = Commit {
        commit_id: String::new(),
        parents: vec![],
        manifests: vec![],
        author: "Legacy".into(),
        message: "unsigned".into(),
        timestamp: 1001,
        public_key: None,
        signature: None,
        key_id: None,
    };
    let u_bytes = metadata::serialize(&unsigned, &MetadataFormat::Json);
    let u_hash = blake3::hash(&u_bytes);
    let u_id = u_hash.to_hex().to_string();
    store
        .put_chunk(&Chunk {
            hash: u_hash,
            data: u_bytes,
            offset: 0,
        })
        .unwrap();

    assert!(shard_core::verify(&repo, &u_id, false).is_err());
    assert!(shard_core::verify_with_options(&repo, &u_id, false, true).is_ok());
}

#[test]
fn workspace_checkout_and_export_reject_path_traversal_manifest() {
    let (_dir, repo) = init_repo();
    let shard_dir = repo.join(".shard");
    let store = Store::open(&shard_dir).unwrap();

    let payload = b"evil payload";
    let p_hash = blake3::hash(payload);
    store
        .put_chunk(&Chunk {
            hash: p_hash,
            data: payload.to_vec(),
            offset: 0,
        })
        .unwrap();

    for bad_name in ["../escaped.txt", "/tmp/abs_escape.txt", ".shard/pwned"] {
        let manifest = FileManifest {
            name: bad_name.to_string(),
            size: payload.len() as u64,
            chunks: vec![p_hash.to_hex().to_string()],
            content_type: None,
            compression: "none".into(),
            merkle_root: None,
            created_by: None,
            created_at: None,
            signature: None,
        };
        let m_bytes = metadata::serialize(&manifest, &MetadataFormat::Json);
        let m_hash = blake3::hash(&m_bytes);
        store
            .put_chunk(&Chunk {
                hash: m_hash,
                data: m_bytes,
                offset: 0,
            })
            .unwrap();

        let commit = Commit {
            commit_id: String::new(),
            parents: vec![],
            manifests: vec![m_hash.to_hex().to_string()],
            author: "Evil".into(),
            message: "traversal".into(),
            timestamp: 1000,
            public_key: None,
            signature: None,
            key_id: None,
        };
        let c_bytes = metadata::serialize(&commit, &MetadataFormat::Json);
        let c_hash = blake3::hash(&c_bytes);
        let c_id = c_hash.to_hex().to_string();
        store
            .put_chunk(&Chunk {
                hash: c_hash,
                data: c_bytes,
                offset: 0,
            })
            .unwrap();

        assert!(
            shard_core::checkout(&repo, &c_id, false).is_err(),
            "checkout must reject traversal manifest '{}'",
            bad_name
        );
        let export_dir = tempfile::tempdir().unwrap();
        assert!(
            shard_core::export(&repo, &c_id, export_dir.path(), false).is_err(),
            "export must reject traversal manifest '{}'",
            bad_name
        );
    }
}

#[tokio::test]
async fn workspace_p2p_forged_public_key_signature_rejected() {
    use shard_net::libp2p::futures::StreamExt;
    use shard_net::libp2p::swarm::SwarmEvent;
    use shard_net::p2p::{Node, ShardBehaviourEvent, ShardContentProvider};

    let authorized_kp = shard_crypto::KeyPair::generate();
    let attacker_kp = shard_crypto::KeyPair::generate();
    let authorized_pk = authorized_kp.verifying_key;

    struct AuthOnlyProvider {
        allowed: ed25519_dalek::VerifyingKey,
    }
    impl ShardContentProvider for AuthOnlyProvider {
        fn get_manifest(&self, _id: &str) -> Option<Vec<u8>> {
            None
        }
        fn get_chunk(&self, _id: &str) -> Option<Vec<u8>> {
            None
        }
        fn put_chunk(&mut self, _id: &str, _data: &[u8]) -> bool {
            false
        }
        fn auth_required(&self) -> bool {
            true
        }
        fn authenticate(&self, _peer_id: &shard_net::libp2p::PeerId, public_key: &[u8]) -> bool {
            public_key == self.allowed.to_bytes().as_slice()
        }
        fn verify_auth(&self, public_key: &[u8], payload: &[u8], signature: &[u8]) -> bool {
            use ed25519_dalek::Verifier;
            let Ok(pk_arr) = <[u8; 32]>::try_from(public_key) else {
                return false;
            };
            let Ok(pk) = ed25519_dalek::VerifyingKey::from_bytes(&pk_arr) else {
                return false;
            };
            let Ok(sig_arr) = <[u8; 64]>::try_from(signature) else {
                return false;
            };
            let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
            pk == self.allowed && pk.verify(payload, &sig).is_ok()
        }
        fn repo_public_key(&self) -> Option<Vec<u8>> {
            Some(self.allowed.to_bytes().to_vec())
        }
    }

    let mut server = Node::new().await.unwrap();
    server.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    let server_peer_id = server.local_peer_id();

    // Wait for server listen addr
    let server_addr = loop {
        if let SwarmEvent::NewListenAddr { address, .. } = server.swarm.select_next_some().await {
            break address;
        }
    };
    let full_addr: shard_net::libp2p::Multiaddr = format!("{}/p2p/{}", server_addr, server_peer_id)
        .parse()
        .unwrap();

    let server_task = tokio::spawn(async move {
        let mut provider = AuthOnlyProvider {
            allowed: authorized_pk,
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(
                std::time::Duration::from_secs(2),
                server.swarm.select_next_some(),
            )
            .await
            {
                Ok(SwarmEvent::Behaviour(ShardBehaviourEvent::RequestResponse(
                    shard_net::libp2p::request_response::Event::Message { peer, message, .. },
                ))) => {
                    if let shard_net::libp2p::request_response::Message::Request {
                        request,
                        channel,
                        ..
                    } = message
                    {
                        server.serve_request(&peer, &mut provider, request, channel);
                    }
                }
                Ok(SwarmEvent::ConnectionClosed {
                    peer_id,
                    num_established: 0,
                    ..
                }) => {
                    server.on_connection_closed(&peer_id);
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    // 1. Spoofing attack: present authorized_pk, but sign with attacker's signing key -> MUST FAIL!
    let mut attacker_client = Node::new().await.unwrap();
    let res = attacker_client
        .request_authenticate(
            &full_addr,
            server_peer_id,
            authorized_pk.to_bytes().to_vec(),
            &attacker_kp.signing_key,
        )
        .await;
    assert!(
        res.is_err(),
        "forged public key with wrong signing key MUST be rejected!"
    );

    // 2. Legitimate client with matching private key -> MUST SUCCEED!
    let mut good_client = Node::new().await.unwrap();
    good_client
        .request_authenticate(
            &full_addr,
            server_peer_id,
            authorized_pk.to_bytes().to_vec(),
            &authorized_kp.signing_key,
        )
        .await
        .expect("authorized key with valid signature must succeed");

    server_task.abort();
}
