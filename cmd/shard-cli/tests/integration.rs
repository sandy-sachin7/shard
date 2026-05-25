use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn shard_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_shard"))
}

fn repo_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("shard-tests").join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn shard(args: &[&str], cwd: &Path) -> Command {
    let mut cmd = Command::new(shard_bin());
    cmd.args(args).current_dir(cwd);
    cmd
}

#[test]
fn test_init_creates_dot_shard() {
    let dir = repo_dir("init-test");
    let output = shard(&["init"], &dir).output().unwrap();
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.join(".shard").is_dir());
    assert!(dir.join(".shard/objects").is_dir());
    assert!(dir.join(".shard/keys").is_dir());
    assert!(dir.join(".shard/keys/secret.key").exists());
    assert!(dir.join(".shard/keys/public.key").exists());
}

#[test]
fn test_init_twice_fails() {
    let dir = repo_dir("init-twice");
    shard(&["init"], &dir).output().unwrap();
    let output = shard(&["init"], &dir).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already initialized"),
        "wrong error: {stderr}"
    );
}

#[test]
fn test_add_commit_verify_roundtrip() {
    let dir = repo_dir("add-commit-verify");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("hello.txt"), b"Hello, Shard!").unwrap();
    let output = shard(&["add", "hello.txt"], &dir).output().unwrap();
    assert!(
        output.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = shard(&["commit", "-m", "first", "--author", "Test"], &dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "commit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let commit_id = stdout
        .split_whitespace()
        .nth(1)
        .expect("no commit id in output");

    let output = shard(&["verify", commit_id], &dir).output().unwrap();
    assert!(
        output.status.success(),
        "verify failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Signature verified"),
        "no signature verification: {stdout}"
    );
    assert!(
        stdout.contains("Verification successful"),
        "verify not successful: {stdout}"
    );
}

#[test]
fn test_verify_fails_on_tampered_chunk() {
    let dir = repo_dir("tamper-test");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("data.bin"), b"important data").unwrap();
    shard(&["add", "data.bin"], &dir).output().unwrap();
    let output = shard(&["commit", "-m", "tamper-me", "--author", "Test"], &dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap().to_string();

    // Tamper with the chunk
    let objects_dir = dir.join(".shard/objects");
    for entry in walkdir::WalkDir::new(&objects_dir) {
        let entry = entry.unwrap();
        if entry.file_type().is_file() && entry.path().extension().is_none() {
            fs::write(entry.path(), b"TAMPERED").unwrap();
            break;
        }
    }

    let output = shard(&["verify", &commit_id], &dir).output().unwrap();
    assert!(
        !output.status.success(),
        "verify should have failed after tampering"
    );
}

#[test]
fn test_empty_commit_fails() {
    let dir = repo_dir("empty-commit");
    shard(&["init"], &dir).output().unwrap();
    let output = shard(&["commit", "-m", "empty", "--author", "Test"], &dir)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nothing to commit"),
        "wrong error: {stderr}"
    );
}

#[test]
fn test_verify_nonexistent_commit_fails() {
    let dir = repo_dir("bad-verify");
    shard(&["init"], &dir).output().unwrap();
    let output = shard(
        &[
            "verify",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ],
        &dir,
    )
    .output()
    .unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_multiple_adds_and_commits() {
    let dir = repo_dir("multi-commit");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("a.txt"), b"file a").unwrap();
    shard(&["add", "a.txt"], &dir).output().unwrap();
    let out1 = shard(&["commit", "-m", "commit a", "--author", "T"], &dir)
        .output()
        .unwrap();
    let id1 = String::from_utf8(out1.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    fs::write(dir.join("b.txt"), b"file b").unwrap();
    shard(&["add", "b.txt"], &dir).output().unwrap();
    let out2 = shard(&["commit", "-m", "commit b", "--author", "T"], &dir)
        .output()
        .unwrap();
    let id2 = String::from_utf8(out2.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    assert_ne!(id1, id2, "commit ids should differ");

    shard(&["verify", &id1], &dir).output().unwrap();
    shard(&["verify", &id2], &dir).output().unwrap();
}

#[test]
fn test_large_file_chunking() {
    let dir = repo_dir("large-file");
    shard(&["init"], &dir).output().unwrap();

    let data = vec![0xABu8; 5 * 1024 * 1024]; // 5 MiB (crosses 4 MiB chunk boundary)
    fs::write(dir.join("large.bin"), &data).unwrap();
    shard(&["add", "large.bin"], &dir).output().unwrap();
    let output = shard(&["commit", "-m", "large", "--author", "T"], &dir)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap();

    shard(&["verify", commit_id], &dir).output().unwrap();
}

#[test]
fn test_log_shows_commits() {
    let dir = repo_dir("log-test");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("a.txt"), b"alpha").unwrap();
    assert!(shard(&["add", "a.txt"], &dir)
        .output()
        .unwrap()
        .status
        .success());
    assert!(shard(&["commit", "-m", "first", "--author", "A"], &dir)
        .output()
        .unwrap()
        .status
        .success());

    fs::write(dir.join("b.txt"), b"beta").unwrap();
    assert!(shard(&["add", "b.txt"], &dir)
        .output()
        .unwrap()
        .status
        .success());
    assert!(shard(&["commit", "-m", "second", "--author", "B"], &dir)
        .output()
        .unwrap()
        .status
        .success());

    let output = shard(&["log"], &dir).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("second"), "missing second commit: {stdout}");
    assert!(stdout.contains("first"), "missing first commit: {stdout}");
    assert!(stdout.contains("author: B"), "missing author B: {stdout}");
    assert!(stdout.contains("author: A"), "missing author A: {stdout}");
}

#[test]
fn test_log_json_output() {
    let dir = repo_dir("log-json");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("x.txt"), b"data").unwrap();
    assert!(shard(&["add", "x.txt"], &dir)
        .output()
        .unwrap()
        .status
        .success());
    assert!(shard(&["commit", "-m", "json-test", "--author", "J"], &dir)
        .output()
        .unwrap()
        .status
        .success());

    let output = shard(&["log", "--json"], &dir).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Should be parseable JSON array
    let entries: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let arr = entries.as_array().unwrap();
    assert!(!arr.is_empty(), "expected non-empty log");
    assert!(arr[0].get("commit_id").and_then(|v| v.as_str()).is_some());
    assert!(arr[0].get("message").and_then(|v| v.as_str()) == Some("json-test"));
}

#[test]
fn test_log_empty_repo_fails() {
    let dir = repo_dir("log-empty");
    shard(&["init"], &dir).output().unwrap();
    let output = shard(&["log"], &dir).output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_checkout_restores_files() {
    let dir = repo_dir("checkout-test");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("hello.txt"), b"Hello, checkout!").unwrap();
    shard(&["add", "hello.txt"], &dir).output().unwrap();
    let output = shard(&["commit", "-m", "checkout-me", "--author", "T"], &dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap().to_string();

    // Remove the file
    fs::remove_file(dir.join("hello.txt")).unwrap();

    // Checkout restores it
    let output = shard(&["checkout", &commit_id], &dir).output().unwrap();
    assert!(
        output.status.success(),
        "checkout failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.join("hello.txt").exists());
    assert_eq!(
        fs::read_to_string(dir.join("hello.txt")).unwrap(),
        "Hello, checkout!"
    );
}

#[test]
fn test_checkout_multiple_files() {
    let dir = repo_dir("checkout-multi");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("a.txt"), b"AAA").unwrap();
    fs::write(dir.join("b.txt"), b"BBB").unwrap();
    shard(&["add", "a.txt"], &dir).output().unwrap();
    shard(&["add", "b.txt"], &dir).output().unwrap();
    let output = shard(&["commit", "-m", "multi", "--author", "T"], &dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap().to_string();

    fs::remove_file(dir.join("a.txt")).unwrap();
    fs::remove_file(dir.join("b.txt")).unwrap();

    let output = shard(&["checkout", &commit_id], &dir).output().unwrap();
    assert!(output.status.success());
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "AAA");
    assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "BBB");
}

#[test]
fn test_checkout_wrong_commit_fails() {
    let dir = repo_dir("checkout-bad");
    shard(&["init"], &dir).output().unwrap();
    let output = shard(
        &[
            "checkout",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ],
        &dir,
    )
    .output()
    .unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_status_after_init() {
    let dir = repo_dir("status-init");
    shard(&["init"], &dir).output().unwrap();
    let output = shard(&["status"], &dir).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("No commits"));
}

#[test]
fn test_status_after_commit() {
    let dir = repo_dir("status-commit");
    shard(&["init"], &dir).output().unwrap();
    fs::write(dir.join("f.txt"), b"data").unwrap();
    shard(&["add", "f.txt"], &dir).output().unwrap();
    shard(&["commit", "-m", "first", "--author", "T"], &dir)
        .output()
        .unwrap();
    let output = shard(&["status"], &dir).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("On branch:"));
    assert!(stdout.contains("Nothing staged"));
}

#[test]
fn test_status_shows_untracked() {
    let dir = repo_dir("status-untracked");
    shard(&["init"], &dir).output().unwrap();
    fs::write(dir.join("tracked.txt"), b"tracked").unwrap();
    shard(&["add", "tracked.txt"], &dir).output().unwrap();
    shard(&["commit", "-m", "first", "--author", "T"], &dir)
        .output()
        .unwrap();
    fs::write(dir.join("untracked.txt"), b"untracked").unwrap();
    let output = shard(&["status"], &dir).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("untracked.txt"));
}

#[test]
fn test_config_set_and_get() {
    let dir = repo_dir("config-test");
    shard(&["init"], &dir).output().unwrap();

    let out = shard(&["config", "set", "user.name", "Alice"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = shard(&["config", "get", "user.name"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout).unwrap().contains("Alice"));
}

#[test]
fn test_config_get_all() {
    let dir = repo_dir("config-all");
    shard(&["init"], &dir).output().unwrap();

    shard(&["config", "set", "a", "1"], &dir).output().unwrap();
    shard(&["config", "set", "b", "2"], &dir).output().unwrap();

    let out = shard(&["config", "get"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("a = 1"));
    assert!(stdout.contains("b = 2"));
}

#[test]
fn test_config_get_missing_fails() {
    let dir = repo_dir("config-missing");
    shard(&["init"], &dir).output().unwrap();

    let out = shard(&["config", "get", "nonexistent"], &dir)
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn test_tag_add_and_list() {
    let dir = repo_dir("tag-test");
    shard(&["init"], &dir).output().unwrap();
    fs::write(dir.join("f.txt"), b"data").unwrap();
    shard(&["add", "f.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "first", "--author", "T"], &dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let cid = stdout.split_whitespace().nth(1).unwrap().to_string();

    let out = shard(&["tag", "add", "v1", &cid], &dir).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8(out.stdout).unwrap().contains("Tagged"));

    let out = shard(&["tag", "list"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("v1"));
    assert!(stdout.contains(&cid));
}

#[test]
fn test_tag_bad_commit_fails() {
    let dir = repo_dir("tag-bad");
    shard(&["init"], &dir).output().unwrap();
    let out = shard(
        &[
            "tag",
            "add",
            "bad",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ],
        &dir,
    )
    .output()
    .unwrap();
    assert!(!out.status.success());
}

#[test]
fn test_prune_removes_unreachable_objects() {
    let dir = repo_dir("prune-test");
    shard(&["init"], &dir).output().unwrap();

    // Create and commit a file
    fs::write(dir.join("keep.txt"), b"keep me").unwrap();
    shard(&["add", "keep.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "first", "--author", "T"], &dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap().to_string();

    // Create an orphan object (unreachable)
    let orphan_hash = "aa00000000000000000000000000000000000000000000000000000000000000";
    let orphan_dir = dir.join(".shard/objects/aa");
    fs::create_dir_all(&orphan_dir).unwrap();
    fs::write(orphan_dir.join(orphan_hash), b"orphan").unwrap();

    // Prune
    let out = shard(&["prune"], &dir).output().unwrap();
    assert!(
        out.status.success(),
        "prune failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Pruned 1"), "expected 1 pruned: {stdout}");

    // Verify orphan is gone
    assert!(
        !orphan_dir.join(orphan_hash).exists(),
        "orphan should be removed"
    );

    // Verify reachable commit still exists
    let prefix = &commit_id[..2];
    assert!(
        dir.join(".shard/objects")
            .join(prefix)
            .join(&commit_id)
            .exists(),
        "commit should still exist"
    );
}

#[test]
fn test_prune_keeps_staged_objects() {
    let dir = repo_dir("prune-staged");
    shard(&["init"], &dir).output().unwrap();

    // Stage a file (chunks stored but not committed)
    fs::write(dir.join("staged.txt"), b"staged data").unwrap();
    shard(&["add", "staged.txt"], &dir).output().unwrap();

    // Create an orphan object
    let orphan_hash = "bb00000000000000000000000000000000000000000000000000000000000000";
    let orphan_dir = dir.join(".shard/objects/bb");
    fs::create_dir_all(&orphan_dir).unwrap();
    fs::write(orphan_dir.join(orphan_hash), b"orphan").unwrap();

    // Prune
    let out = shard(&["prune"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Pruned 1"), "expected 1 pruned: {stdout}");
    assert!(
        !orphan_dir.join(orphan_hash).exists(),
        "orphan should be removed"
    );
}

#[test]
fn test_verify_json_output() {
    let dir = repo_dir("verify-json");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("f.txt"), b"data").unwrap();
    shard(&["add", "f.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "json-verify", "--author", "T"], &dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap();

    let out = shard(&["verify", "--json", commit_id], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["verified"], true);
    assert_eq!(v["signature_verified"], true);
    assert!(v["files_checked"].as_u64().unwrap_or(0) >= 1);
}

#[test]
fn test_status_json_output() {
    let dir = repo_dir("status-json");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("a.txt"), b"alpha").unwrap();
    shard(&["add", "a.txt"], &dir).output().unwrap();
    shard(&["commit", "-m", "first", "--author", "T"], &dir)
        .output()
        .unwrap();

    let out = shard(&["status", "--json"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(v["commit"].is_string());
    assert!(v["staged"].is_array());
    assert!(v["deleted"].is_array());
    assert!(v["untracked"].is_array());
}

#[test]
fn test_checkout_json_output() {
    let dir = repo_dir("checkout-json");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("f.txt"), b"checkout json").unwrap();
    shard(&["add", "f.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "json-co", "--author", "T"], &dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap().to_string();

    fs::remove_file(dir.join("f.txt")).unwrap();

    let out = shard(&["checkout", "--json", &commit_id], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["commit_id"], commit_id);
    assert!(v["files"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("f.txt")));
}

#[test]
fn test_init_private_sets_config() {
    let dir = repo_dir("init-private");
    let out = shard(&["init", "--private"], &dir).output().unwrap();
    assert!(out.status.success());

    let out = shard(&["config", "get", "private"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("private = true"));
}

#[test]
fn test_three_node_pull() {
    let dir_a = repo_dir("3node-a");
    let dir_b = repo_dir("3node-b");
    let dir_c = repo_dir("3node-c");

    // Setup node A: init, add, commit
    shard(&["init"], &dir_a).output().unwrap();
    fs::write(dir_a.join("data.txt"), b"shared data").unwrap();
    shard(&["add", "data.txt"], &dir_a).output().unwrap();
    let out = shard(&["commit", "-m", "shared", "--author", "T"], &dir_a)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap().to_string();

    // Start share on node A in background, pipe stdout to capture peer ID + listen addr
    let mut child = Command::new(shard_bin())
        .arg("share")
        .current_dir(&dir_a)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start share");

    let reader = BufReader::new(child.stdout.take().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut peer_id = String::new();
        let mut listen_addr = String::new();
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if let Some(id) = line.strip_prefix("Local peer id: ") {
                peer_id = id.to_string();
            }
            if let Some(addr) = line.strip_prefix("Listening on ") {
                listen_addr = addr.trim().trim_matches('"').to_string();
            }
            if !peer_id.is_empty() && !listen_addr.is_empty() {
                let _ = tx.send((peer_id.clone(), listen_addr.clone()));
                peer_id.clear();
                listen_addr.clear();
            }
        }
    });

    let (peer_id, listen_addr) = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("timed out waiting for share output");
    let multiaddr = format!("{}/p2p/{}", listen_addr, peer_id);

    // Node B pulls from A
    let out = shard(&["pull", &multiaddr, &commit_id], &dir_b)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "B pull failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir_b.join("data.txt").exists());
    assert_eq!(
        fs::read_to_string(dir_b.join("data.txt")).unwrap(),
        "shared data"
    );

    // Node C pulls from A
    let out = shard(&["pull", &multiaddr, &commit_id], &dir_c)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "C pull failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir_c.join("data.txt").exists());
    assert_eq!(
        fs::read_to_string(dir_c.join("data.txt")).unwrap(),
        "shared data"
    );

    // Cleanup
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn test_sync_auto_pull() {
    let dir_a = repo_dir("sync-a");
    let dir_b = repo_dir("sync-b");

    // Setup node A: init, add, commit
    shard(&["init"], &dir_a).output().unwrap();
    fs::write(dir_a.join("shared.txt"), b"sync test data").unwrap();
    shard(&["add", "shared.txt"], &dir_a).output().unwrap();
    let out = shard(&["commit", "-m", "sync-test", "--author", "T"], &dir_a)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap().to_string();

    // Start sync on A in background
    let mut child_a = Command::new(shard_bin())
        .arg("sync")
        .current_dir(&dir_a)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start sync A");

    let reader_a = BufReader::new(child_a.stdout.take().unwrap());
    let (tx_a, rx_a) = mpsc::channel();
    std::thread::spawn(move || {
        let mut peer_id = String::new();
        let mut listen_addr = String::new();
        for line in reader_a.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if let Some(id) = line.strip_prefix("Local peer id: ") {
                peer_id = id.to_string();
            }
            if let Some(addr) = line.strip_prefix("Listening on ") {
                listen_addr = addr.trim().trim_matches('"').to_string();
            }
            if !peer_id.is_empty() && !listen_addr.is_empty() {
                let _ = tx_a.send((peer_id.clone(), listen_addr.clone()));
                peer_id.clear();
                listen_addr.clear();
            }
        }
    });

    let (peer_id_a, listen_addr_a) = rx_a
        .recv_timeout(Duration::from_secs(30))
        .expect("timed out waiting for sync A output");
    let multiaddr_a = format!("{}/p2p/{}", listen_addr_a, peer_id_a);

    // Configure B with A's address as a peer
    shard(&["init"], &dir_b).output().unwrap();
    // Copy A's repo_id so both repos share the same gossipsub topic
    let a_config: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(dir_a.join(".shard/config.json")).unwrap()).unwrap();
    let repo_id = a_config.get("repo_id").expect("A has repo_id");
    let out = shard(&["config", "set", "repo_id", repo_id], &dir_b)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "config set repo_id failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = shard(&["peer", "add", &multiaddr_a], &dir_b)
        .output()
        .unwrap();
    assert!(out.status.success(), "peer add failed");

    // Start sync on B in background
    let mut child_b = Command::new(shard_bin())
        .arg("sync")
        .current_dir(&dir_b)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start sync B");

    let reader_b = BufReader::new(child_b.stdout.take().unwrap());
    let stderr_b = child_b.stderr.take().unwrap();
    let (tx_b, rx_b) = mpsc::channel();
    std::thread::spawn(move || {
        let stderr_reader = BufReader::new(stderr_b);
        let stderr_tx = tx_b.clone();
        std::thread::spawn(move || {
            for l in stderr_reader.lines().map_while(Result::ok) {
                eprintln!("[sync-B stderr] {}", l);
                if l.contains("auto-pull") || l.contains("Auto-pull") || l.contains("announce") {
                    let _ = stderr_tx.send(format!("stderr:{}", l));
                }
            }
        });
        for line in reader_b.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            eprintln!("[sync-B stdout] {}", line);
            if line.starts_with("Auto-pulled commit") {
                let _ = tx_b.send(line);
            }
        }
    });

    // Wait for B to auto-pull
    let result = rx_b.recv_timeout(Duration::from_secs(60));
    assert!(
        result.is_ok(),
        "B did not auto-pull commit within timeout: {:?}",
        result.err()
    );
    let msg = result.unwrap();
    assert!(msg.contains(&commit_id), "auto-pulled wrong commit: {msg}");

    // Verify file was pulled
    assert!(dir_b.join("shared.txt").exists());
    assert_eq!(
        fs::read_to_string(dir_b.join("shared.txt")).unwrap(),
        "sync test data"
    );

    // Cleanup
    let _ = child_a.kill();
    let _ = child_a.wait();
    let _ = child_b.kill();
    let _ = child_b.wait();
}

#[test]
fn test_init_sqlite_creates_dot_shard() {
    let dir = repo_dir("sqlite-init");
    let output = shard(&["init", "--db", "sqlite"], &dir).output().unwrap();
    assert!(
        output.status.success(),
        "sqlite init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.join(".shard").is_dir());
    // Config must store the sqlite backend
    let config: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(dir.join(".shard/config.json")).unwrap()).unwrap();
    assert_eq!(
        config.get("storage_backend").map(|s| s.as_str()),
        Some("sqlite")
    );
}

#[test]
fn test_sqlite_add_commit_verify_roundtrip() {
    let dir = repo_dir("sqlite-roundtrip");
    shard(&["init", "--db", "sqlite"], &dir).output().unwrap();

    fs::write(dir.join("hello.txt"), b"Hello, SQLite!").unwrap();
    let output = shard(&["add", "hello.txt"], &dir).output().unwrap();
    assert!(
        output.status.success(),
        "sqlite add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // DB file should now exist after first chunk stored
    assert!(dir.join(".shard/objects.db").exists());

    let output = shard(&["commit", "-m", "sqlite-test", "--author", "Test"], &dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "sqlite commit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).expect("no commit id");

    let output = shard(&["verify", commit_id], &dir).output().unwrap();
    assert!(
        output.status.success(),
        "sqlite verify failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Verification successful"));
}

#[test]
fn test_sqlite_checkout_restores_files() {
    let dir = repo_dir("sqlite-checkout");
    shard(&["init", "--db", "sqlite"], &dir).output().unwrap();

    fs::write(dir.join("restore.txt"), b"SQLite checkout").unwrap();
    shard(&["add", "restore.txt"], &dir).output().unwrap();
    let output = shard(&["commit", "-m", "co-test", "--author", "T"], &dir)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap().to_string();

    fs::remove_file(dir.join("restore.txt")).unwrap();
    let output = shard(&["checkout", &commit_id], &dir).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(dir.join("restore.txt")).unwrap(),
        "SQLite checkout"
    );
}

#[test]
fn test_private_init_creates_repo_key_and_config() {
    let dir = repo_dir("private-init-key");
    let out = shard(&["init", "--private"], &dir).output().unwrap();
    assert!(out.status.success(), "private init failed");

    // repo.key must exist and be a 64-char hex string (32 bytes)
    let key_path = dir.join(".shard/keys/repo.key");
    assert!(key_path.exists(), "repo.key not created for private repo");
    let key_hex = fs::read_to_string(&key_path).unwrap();
    let key_hex = key_hex.trim();
    assert_eq!(
        key_hex.len(),
        64,
        "repo.key should be 64 hex chars (32 bytes)"
    );

    // Config must have private=true
    let out = shard(&["config", "get", "private"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("private = true"));
}

#[test]
fn test_private_add_commit_verify_checkout_roundtrip() {
    let dir = repo_dir("private-roundtrip");
    let out = shard(&["init", "--private"], &dir).output().unwrap();
    assert!(out.status.success());

    fs::write(dir.join("secret.txt"), b"this is private data").unwrap();
    let out = shard(&["add", "secret.txt"], &dir).output().unwrap();
    assert!(
        out.status.success(),
        "private add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = shard(
        &["commit", "-m", "private-commit", "--author", "Test"],
        &dir,
    )
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "private commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).expect("no commit id");

    // Verify must succeed on encrypted data
    let out = shard(&["verify", commit_id], &dir).output().unwrap();
    assert!(
        out.status.success(),
        "private verify failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Verification successful"));

    // Checkout must produce original content
    fs::remove_file(dir.join("secret.txt")).unwrap();
    let out = shard(&["checkout", commit_id], &dir).output().unwrap();
    assert!(
        out.status.success(),
        "private checkout failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.join("secret.txt")).unwrap(),
        "this is private data"
    );

    // Verify stored chunk is NOT the plaintext (encryption active)
    let objects_dir = dir.join(".shard/objects");
    if objects_dir.is_dir() {
        let plaintext = b"this is private data";
        for entry in walkdir::WalkDir::new(&objects_dir) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let content = fs::read(entry.path()).unwrap();
                assert!(
                    !content.windows(plaintext.len()).any(|w| w == plaintext),
                    "stored chunk contains plaintext — encryption not active"
                );
            }
        }
    }
}
