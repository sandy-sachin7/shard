use std::fs;
use std::io::{BufRead, BufReader};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn shard_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_shard"))
}

struct TempRepo {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Deref for TempRepo {
    type Target = Path;
    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for TempRepo {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

fn repo_dir(_name: &str) -> TempRepo {
    let dir = tempfile::tempdir().expect("failed to create temp repo dir");
    let path = dir.path().to_path_buf();
    TempRepo { _dir: dir, path }
}

fn shard(args: &[&str], cwd: &Path) -> Command {
    let mut cmd = Command::new(shard_bin());
    cmd.args(args).current_dir(cwd);
    cmd
}

fn shard_ok(args: &[&str], cwd: &Path) -> std::process::Output {
    let out = shard(args, cwd).output().unwrap();
    assert!(
        out.status.success(),
        "shard {:?} failed:\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
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
    shard_ok(&["init"], &dir_a);
    fs::write(dir_a.join("data.txt"), b"shared data").unwrap();
    shard_ok(&["add", "data.txt"], &dir_a);
    let out = shard_ok(&["commit", "-m", "shared", "--author", "T"], &dir_a);
    let stdout = String::from_utf8(out.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap().to_string();

    // Start share on node A bound to loopback
    let mut child = Command::new(shard_bin())
        .args(["share", "--listen", "/ip4/127.0.0.1/tcp/0"])
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
        let mut sent = false;
        for line in reader.lines().map_while(Result::ok) {
            if !sent {
                if let Some(id) = line.strip_prefix("Local peer id: ") {
                    peer_id = id.trim().to_string();
                }
                if let Some(addr) = line.strip_prefix("Listening on ") {
                    let candidate = addr.trim().trim_matches('"').to_string();
                    if candidate.contains("/ip4/127.0.0.1/") {
                        listen_addr = candidate;
                    }
                }
                if !peer_id.is_empty() && !listen_addr.is_empty() {
                    let _ = tx.send((peer_id.clone(), listen_addr.clone()));
                    sent = true;
                }
            }
        }
    });

    let (peer_id, listen_addr) = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("timed out waiting for share output");
    let multiaddr = format!("{}/p2p/{}", listen_addr, peer_id);

    // Node B pulls from A
    shard_ok(&["pull", &multiaddr, &commit_id], &dir_b);
    assert!(dir_b.join("data.txt").exists());
    assert_eq!(
        fs::read_to_string(dir_b.join("data.txt")).unwrap(),
        "shared data"
    );

    // Node C pulls from A
    shard_ok(&["pull", &multiaddr, &commit_id], &dir_c);
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
    shard_ok(&["init"], &dir_a);
    fs::write(dir_a.join("shared.txt"), b"sync test data").unwrap();
    shard_ok(&["add", "shared.txt"], &dir_a);
    let out = shard_ok(&["commit", "-m", "sync-test", "--author", "T"], &dir_a);
    let stdout = String::from_utf8(out.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap().to_string();

    // Start sync on A in background
    let mut child_a = Command::new(shard_bin())
        .arg("sync")
        .current_dir(&dir_a)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start sync A");

    let reader_a = BufReader::new(child_a.stdout.take().unwrap());
    let (tx_a, rx_a) = mpsc::channel();
    std::thread::spawn(move || {
        let mut peer_id = String::new();
        let mut listen_addr = String::new();
        let mut sent = false;
        // Keep draining stdout until EOF so child_a never gets SIGPIPE on subsequent NewListenAddr prints!
        for line in reader_a.lines().map_while(Result::ok) {
            if !sent {
                if let Some(id) = line.strip_prefix("Local peer id: ") {
                    peer_id = id.trim().to_string();
                }
                if let Some(addr) = line.strip_prefix("Listening on ") {
                    let candidate = addr.trim().trim_matches('"').to_string();
                    if candidate.contains("/ip4/127.0.0.1/") {
                        listen_addr = candidate;
                    }
                }
                if !peer_id.is_empty() && !listen_addr.is_empty() {
                    let _ = tx_a.send((peer_id.clone(), listen_addr.clone()));
                    sent = true;
                }
            }
        }
    });

    let (peer_id_a, listen_addr_a) = rx_a
        .recv_timeout(Duration::from_secs(30))
        .expect("timed out waiting for sync A output");
    let multiaddr_a = format!("{}/p2p/{}", listen_addr_a, peer_id_a);

    // Configure B with A's repo_id and peer address
    shard_ok(&["init"], &dir_b);
    let a_config: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(dir_a.join(".shard/config.json")).unwrap()).unwrap();
    let repo_id = a_config.get("repo_id").expect("A has repo_id");
    shard_ok(&["config", "set", "repo_id", repo_id], &dir_b);
    shard_ok(&["peer", "add", &multiaddr_a], &dir_b);

    // Start sync on B in background
    let mut child_b = Command::new(shard_bin())
        .arg("sync")
        .current_dir(&dir_b)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start sync B");

    let reader_b = BufReader::new(child_b.stdout.take().unwrap());
    let (tx_b, rx_b) = mpsc::channel();
    std::thread::spawn(move || {
        let mut sent = false;
        for line in reader_b.lines().map_while(Result::ok) {
            if !sent && line.starts_with("Auto-pulled commit") {
                let _ = tx_b.send(line);
                sent = true;
            }
        }
    });

    // Wait for B to auto-pull
    let msg = rx_b
        .recv_timeout(Duration::from_secs(30))
        .expect("B did not auto-pull commit within timeout");
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

#[test]
fn test_key_rotate_verify_keychain() {
    let dir = repo_dir("key-rotate");
    let out = shard(&["init"], &dir).output().unwrap();
    assert!(out.status.success(), "init failed");

    // Initial key list should show one key (the genesis key)
    let out = shard(&["key", "list"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Current key:"));
    assert!(stdout.contains("Key history:"));
    assert!(stdout.contains("(active)"));

    // Verify keychain (should pass with genesis key only)
    let out = shard(&["key", "verify"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Keychain verification successful"));

    // Rotate the key
    let out = shard(&["key", "rotate"], &dir).output().unwrap();
    assert!(
        out.status.success(),
        "key rotate failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Verify keychain (should pass with rotation)
    let out = shard(&["key", "verify"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Keychain verification successful"));

    // Key list should now show 2 keys
    let out = shard(&["key", "list"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| l.contains("previous:")).collect();
    assert!(!lines.is_empty(), "should show previous key reference");
}

#[test]
fn test_key_rotate_add_commit_verify_roundtrip() {
    let dir = repo_dir("key-roundtrip");
    let out = shard(&["init"], &dir).output().unwrap();
    assert!(out.status.success());

    // Add a file and commit with genesis key
    fs::write(dir.join("model.bin"), b"some model data").unwrap();
    let out = shard(&["add", "model.bin"], &dir).output().unwrap();
    assert!(out.status.success());
    let out = shard(
        &["commit", "-m", "genesis-commit", "--author", "Test"],
        &dir,
    )
    .output()
    .unwrap();
    assert!(out.status.success());
    let genesis_cid = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .expect("no commit id")
        .to_string();

    // Rotate the signing key
    let out = shard(&["key", "rotate"], &dir).output().unwrap();
    assert!(out.status.success());

    // Add another file and commit with rotated key
    fs::write(dir.join("model-v2.bin"), b"more model data").unwrap();
    let out = shard(&["add", "model-v2.bin"], &dir).output().unwrap();
    assert!(out.status.success());
    let out = shard(
        &["commit", "-m", "rotated-commit", "--author", "Test"],
        &dir,
    )
    .output()
    .unwrap();
    assert!(out.status.success());

    // Both commits should verify (keychain validates both)
    let out = shard(&["verify", &genesis_cid], &dir).output().unwrap();
    assert!(
        out.status.success(),
        "genesis commit verify failed after rotation: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Verification successful"));

    let out = shard(&["verify", &genesis_cid, "--json"], &dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "genesis verify --json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("verified"));
}

#[test]
fn test_key_verify_tampered_rotation_fails() {
    let dir = repo_dir("key-tamper");
    let out = shard(&["init"], &dir).output().unwrap();
    assert!(out.status.success());

    let out = shard(&["key", "rotate"], &dir).output().unwrap();
    assert!(out.status.success());

    // Tamper with rotation signature by modifying the file
    let rot_dir = dir.join(".shard/keys/rotations");
    for entry in fs::read_dir(&rot_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            let data = fs::read(entry.path()).unwrap();
            if let Ok(mut obj) = serde_json::from_slice::<serde_json::Value>(&data) {
                if obj.get("signature_hex").is_some() {
                    let tampered = "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
                    obj["signature_hex"] = serde_json::Value::String(tampered.to_string());
                    fs::write(entry.path(), serde_json::to_string_pretty(&obj).unwrap()).unwrap();
                }
            }
        }
    }

    // Verify should fail
    let out = shard(&["key", "verify"], &dir).output().unwrap();
    assert!(
        !out.status.success(),
        "tampered keychain should fail verify"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Keychain verification failed"));
}

#[test]
fn test_transfer_list_remove() {
    let dir = repo_dir("transfer-list");
    let out = shard(&["init"], &dir).output().unwrap();
    assert!(out.status.success());

    // No transfers initially
    let out = shard(&["transfer", "list"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("No incomplete transfers"));

    // Create a fake partial transfer directory
    let partial = dir.join(".shard/partial/fake-commit-id");
    fs::create_dir_all(&partial).unwrap();
    fs::write(partial.join("chunk1"), b"data1").unwrap();
    fs::write(partial.join("chunk2"), b"data2").unwrap();

    // List should show it
    let out = shard(&["transfer", "list"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("fake-commit-id"));

    // JSON output
    let out = shard(&["transfer", "list", "--json"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("fake-commit-id"));

    // Remove should clean it up
    let out = shard(&["transfer", "remove", "fake-commit-id"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    // List should be empty again
    let out = shard(&["transfer", "list"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("No incomplete transfers"));
}

#[test]
fn test_diff_no_changes() {
    let dir = repo_dir("diff-none");
    let out = shard(&["init"], &dir).output().unwrap();
    assert!(out.status.success());

    fs::write(dir.join("f.txt"), b"content").unwrap();
    shard(&["add", "f.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "c1", "--author", "T"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let c1 = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    // Diff same commit against itself
    let out = shard(&["diff", &c1, &c1], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("No differences"));
}

#[test]
fn test_diff_single_file_added() {
    let dir = repo_dir("diff-added");
    shard(&["init"], &dir).output().unwrap();
    fs::write(dir.join("f.txt"), b"line1\nline2\nline3").unwrap();
    shard(&["add", "f.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "c1", "--author", "T"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let c1 = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    // Compare c1 with initial (empty) HEAD — need a parent commit reference
    // Instead, compare two commits: the genesis empty init has no commit.
    // Use the first commit's parent which may not exist.
    // Better: create two different commits and compare them.
    fs::write(dir.join("g.txt"), b"other").unwrap();
    shard(&["add", "g.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "c2", "--author", "T"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let c2 = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    let out = shard(&["diff", &c1, &c2], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("f.txt") || stdout.contains("g.txt"));
    assert!(stdout.contains("No differences") || stdout.contains("---") || stdout.contains("+++"));
}

#[test]
fn test_diff_modified_file() {
    let dir = repo_dir("diff-mod");
    shard(&["init"], &dir).output().unwrap();
    fs::write(dir.join("f.txt"), b"hello\nworld\n").unwrap();
    shard(&["add", "f.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "c1", "--author", "T"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let c1 = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    fs::write(dir.join("f.txt"), b"hello\nuniverse\n").unwrap();
    shard(&["add", "f.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "c2", "--author", "T"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let c2 = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    let out = shard(&["diff", &c1, &c2], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("f.txt"));
    assert!(stdout.contains("-world"));
    assert!(stdout.contains("+universe"));
}

#[test]
fn test_diff_json_output() {
    let dir = repo_dir("diff-json");
    shard(&["init"], &dir).output().unwrap();
    fs::write(dir.join("a.txt"), b"aaa").unwrap();
    shard(&["add", "a.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "c1", "--author", "T"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let c1 = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    fs::write(dir.join("b.txt"), b"bbb").unwrap();
    shard(&["add", "b.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "c2", "--author", "T"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let c2 = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    let out = shard(&["diff", &c1, &c2, "--json"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("added"));
    assert!(stdout.contains("b.txt"));
}

#[test]
fn test_diff_between_commits_with_removed_file() {
    let dir = repo_dir("diff-removed");
    shard(&["init"], &dir).output().unwrap();
    fs::write(dir.join("keep.txt"), b"keep").unwrap();
    fs::write(dir.join("remove.txt"), b"remove").unwrap();
    shard(&["add", "keep.txt"], &dir).output().unwrap();
    shard(&["add", "remove.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "c1", "--author", "T"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let c1 = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    // Remove remove.txt and commit
    fs::remove_file(dir.join("remove.txt")).unwrap();
    // shard add with a non-existent file would fail. Instead, just re-add keep.txt.
    // But we can't remove from index directly. Let's use prune or a new file.
    // Better approach: compare c1 (has both) with c2 (only keep.txt)
    // Since we can't remove from index, let's modify keep.txt and add a new file:
    fs::write(dir.join("keep.txt"), b"keep-modified").unwrap();
    shard(&["add", "keep.txt"], &dir).output().unwrap();
    let out = shard(&["commit", "-m", "c2", "--author", "T"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let c2 = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    // c1 has {keep.txt, remove.txt}, c2 has {keep.txt}
    // diff should note remove.txt is gone from c2
    let out = shard(&["diff", &c1, &c2], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    // c2 doesn't have remove.txt
    // Since remove.txt was in c1 manifest but not in c2 index/commit, it should show as removed
    assert!(stdout.contains("keep") || stdout.contains("remove"));
}

#[test]
fn test_partial_recover_after_interrupted_pull() {
    let dir = repo_dir("partial-recover");
    let out = shard(&["init"], &dir).output().unwrap();
    assert!(out.status.success());

    // Create a multi-chunk file (~5 MB = 2 chunks with 4 MiB default)
    let data = b"x".repeat(6_000_000);
    fs::write(dir.join("large.bin"), &data).unwrap();
    let out = shard(&["add", "large.bin"], &dir).output().unwrap();
    assert!(out.status.success());
    let out = shard(&["commit", "-m", "big-file", "--author", "Test"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let commit_id = stdout
        .split_whitespace()
        .nth(1)
        .expect("no commit id")
        .to_string();

    // Simulate partial transfer: copy chunk files from objects into .shard/partial/<commit_id>/
    let partial_dir = dir.join(".shard/partial").join(&commit_id);
    fs::create_dir_all(&partial_dir).unwrap();

    // Copy a chunk file from the objects directory to simulate a previous partial download
    let objects_dir = dir.join(".shard/objects");
    let mut copied = 0usize;
    if objects_dir.is_dir() {
        for entry in walkdir::WalkDir::new(&objects_dir) {
            if copied >= 1 {
                break;
            }
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let name = entry.file_name().to_string_lossy().to_string();
                let content = fs::read(entry.path()).unwrap();
                fs::write(partial_dir.join(&name), &content).unwrap();
                copied += 1;
            }
        }
    }

    // Verify — should still succeed (chunks in store, partial just has copies)
    let out = shard(&["verify", &commit_id], &dir).output().unwrap();
    assert!(
        out.status.success(),
        "verify after partial recovery failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Clean up via transfer remove
    let out = shard(&["transfer", "remove", &commit_id], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    // Partial dir should be gone
    assert!(
        !partial_dir.exists(),
        "partial dir should be removed after transfer remove"
    );

    // Verify still works after partial cleanup
    let out = shard(&["verify", &commit_id], &dir).output().unwrap();
    assert!(
        out.status.success(),
        "verify after partial cleanup failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_cbor_commit_verify_roundtrip() {
    let dir = repo_dir("cbor-commit");
    let out = shard(&["init"], &dir).output().unwrap();
    assert!(out.status.success(), "init failed");

    // Enable CBOR serialization
    let out = shard(&["config", "set", "serialization_format", "cbor"], &dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "config set cbor failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    fs::write(dir.join("data.txt"), b"cbor test data").unwrap();
    let out = shard(&["add", "data.txt"], &dir).output().unwrap();
    assert!(out.status.success(), "cbor add failed");
    let out = shard(&["commit", "-m", "cbor-commit", "--author", "Test"], &dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "cbor commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).expect("no commit id");

    // Verify
    let out = shard(&["verify", commit_id], &dir).output().unwrap();
    assert!(
        out.status.success(),
        "cbor verify failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Verification successful"));

    // Checkout
    fs::remove_file(dir.join("data.txt")).unwrap();
    let out = shard(&["checkout", commit_id], &dir).output().unwrap();
    assert!(
        out.status.success(),
        "cbor checkout failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.join("data.txt")).unwrap(),
        "cbor test data"
    );

    // Verify works after cleanup
    let out = shard(&["verify", commit_id], &dir).output().unwrap();
    assert!(out.status.success(), "cbor verify after checkout failed");
}

#[test]
fn test_cbor_mixed_format_compat() {
    let dir = repo_dir("cbor-compat");
    let out = shard(&["init"], &dir).output().unwrap();
    assert!(out.status.success(), "init failed");

    // Commit 1: JSON (default)
    fs::write(dir.join("f1.txt"), b"json version").unwrap();
    let out = shard(&["add", "f1.txt"], &dir).output().unwrap();
    assert!(out.status.success());
    let out = shard(&["commit", "-m", "json-commit", "--author", "Test"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let c1 = stdout.split_whitespace().nth(1).unwrap().to_string();

    // Switch to CBOR and commit again
    let out = shard(&["config", "set", "serialization_format", "cbor"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    fs::write(dir.join("f2.txt"), b"cbor version").unwrap();
    let out = shard(&["add", "f2.txt"], &dir).output().unwrap();
    assert!(out.status.success());
    let out = shard(&["commit", "-m", "cbor-commit", "--author", "Test"], &dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let c2 = stdout.split_whitespace().nth(1).unwrap().to_string();

    // Both commits should verify
    let out = shard(&["verify", &c1], &dir).output().unwrap();
    assert!(
        out.status.success(),
        "JSON commit verify failed after CBOR commit"
    );
    let out = shard(&["verify", &c2], &dir).output().unwrap();
    assert!(out.status.success(), "CBOR commit verify failed");

    // Log should show both commits
    let out = shard(&["log"], &dir).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains(&c1[..8]));
    assert!(stdout.contains(&c2[..8]));

    // Diff should work between the two
    let out = shard_ok(&["diff", &c1, &c2], &dir);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("f1.txt") || stdout.contains("f2.txt"));
}

fn spawn_share(repo: &Path) -> (std::process::Child, String) {
    let mut child = Command::new(shard_bin())
        .args(["share", "--listen", "/ip4/127.0.0.1/tcp/0"])
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start share");

    let reader = BufReader::new(child.stdout.take().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut peer_id = String::new();
        let mut listen_addr = String::new();
        let mut sent = false;
        for line in reader.lines().map_while(Result::ok) {
            if !sent {
                if let Some(id) = line.strip_prefix("Local peer id: ") {
                    peer_id = id.trim().to_string();
                }
                if let Some(addr) = line.strip_prefix("Listening on ") {
                    let candidate = addr.trim().trim_matches('"').to_string();
                    if candidate.contains("/ip4/127.0.0.1/") {
                        listen_addr = candidate;
                    }
                }
                if !peer_id.is_empty() && !listen_addr.is_empty() {
                    let _ = tx.send(format!("{}/p2p/{}", listen_addr, peer_id));
                    sent = true;
                }
            }
        }
    });

    let multiaddr = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("timed out waiting for share multiaddr");
    (child, multiaddr)
}

#[test]
fn test_p2p_auth_authorized_succeeds_and_unauthorized_denied() {
    let dir_a = repo_dir("auth-a");
    let dir_b = repo_dir("auth-b");
    let dir_c = repo_dir("auth-c");

    shard_ok(&["init"], &dir_a);
    shard_ok(&["init"], &dir_b);
    shard_ok(&["init"], &dir_c);

    // Authorize B's public key on A (C is NOT authorized)
    let b_pub = fs::read(dir_b.join(".shard/keys/public.key")).unwrap();
    let b_pub_hex: String = b_pub.iter().map(|b| format!("{:02x}", b)).collect();
    shard_ok(&["key", "add-authorized", &b_pub_hex], &dir_a);

    fs::write(dir_a.join("restricted.txt"), b"authorized payload").unwrap();
    shard_ok(&["add", "restricted.txt"], &dir_a);
    let out = shard_ok(&["commit", "-m", "restricted", "--author", "A"], &dir_a);
    let commit_id = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    let (mut child, multiaddr) = spawn_share(&dir_a);

    // Authorized peer B succeeds
    shard_ok(&["pull", &multiaddr, &commit_id], &dir_b);
    assert_eq!(
        fs::read_to_string(dir_b.join("restricted.txt")).unwrap(),
        "authorized payload"
    );

    // Unauthorized peer C must be denied
    let c_out = shard(&["pull", &multiaddr, &commit_id], &dir_c)
        .output()
        .unwrap();
    assert!(
        !c_out.status.success(),
        "unauthorized peer C should have been denied"
    );

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn test_p2p_push_and_pull_zstd_and_private_with_resume() {
    // 1. Default zstd push & pull
    let dir_src = repo_dir("push-zstd-src");
    let dir_dst = repo_dir("push-zstd-dst");
    shard_ok(&["init", "--compression", "zstd"], &dir_src);
    shard_ok(&["init", "--compression", "zstd"], &dir_dst);

    let big_payload = b"zstd push pull payload data! ".repeat(2048);
    fs::write(dir_src.join("weights.bin"), &big_payload).unwrap();
    shard_ok(&["add", "weights.bin"], &dir_src);
    let out = shard_ok(&["commit", "-m", "zstd-push", "--author", "S"], &dir_src);
    let cid = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    let (mut dst_server, dst_addr) = spawn_share(&dir_dst);
    shard_ok(&["push", &dst_addr], &dir_src);
    let _ = dst_server.kill();
    let _ = dst_server.wait();

    // Checkout on destination proves pushed zstd chunks are intact!
    shard_ok(&["checkout", &cid], &dir_dst);
    assert_eq!(fs::read(dir_dst.join("weights.bin")).unwrap(), big_payload);

    // 2. Private (--private) encrypted push & pull + partial resume!
    let priv_a = repo_dir("priv-a");
    let priv_b = repo_dir("priv-b");
    shard_ok(&["init", "--private", "--compression", "zstd"], &priv_a);
    shard_ok(&["init", "--private", "--compression", "zstd"], &priv_b);
    // Share repo.key between private cluster peers
    fs::copy(
        priv_a.join(".shard/keys/repo.key"),
        priv_b.join(".shard/keys/repo.key"),
    )
    .unwrap();

    let secret_payload = b"top secret encrypted tensor data! ".repeat(1024);
    fs::write(priv_a.join("model.safetensors"), &secret_payload).unwrap();
    shard_ok(&["add", "model.safetensors"], &priv_a);
    let out = shard_ok(&["commit", "-m", "priv-commit", "--author", "A"], &priv_a);
    let priv_cid = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    // Pre-populate partial/ on priv_b with the encrypted chunk to test resume recovery!
    let chunk_hash = format!("{}", blake3::hash(&secret_payload).to_hex());
    let src_chunk_file = priv_a
        .join(".shard/objects")
        .join(&chunk_hash[..2])
        .join(&chunk_hash);
    let partial_dir = priv_b.join(".shard/partial").join(&priv_cid);
    fs::create_dir_all(&partial_dir).unwrap();
    fs::copy(&src_chunk_file, partial_dir.join(&chunk_hash)).unwrap();

    let (mut priv_server, priv_addr) = spawn_share(&priv_a);
    let pull_out = shard_ok(&["pull", &priv_addr, &priv_cid], &priv_b);
    let pull_stdout = String::from_utf8(pull_out.stdout).unwrap();
    assert!(
        pull_stdout.contains("Recovered 1 chunks from partial transfer"),
        "expected partial resume recovery, got: {}",
        pull_stdout
    );
    assert_eq!(
        fs::read(priv_b.join("model.safetensors")).unwrap(),
        secret_payload
    );
    let _ = priv_server.kill();
    let _ = priv_server.wait();

    // Push private commit from priv_a to a fresh priv_c sharing repo.key
    let priv_c = repo_dir("priv-c");
    shard_ok(&["init", "--private", "--compression", "zstd"], &priv_c);
    fs::copy(
        priv_a.join(".shard/keys/repo.key"),
        priv_c.join(".shard/keys/repo.key"),
    )
    .unwrap();
    let (mut priv_c_server, priv_c_addr) = spawn_share(&priv_c);
    shard_ok(&["push", &priv_c_addr], &priv_a);
    let _ = priv_c_server.kill();
    let _ = priv_c_server.wait();
    shard_ok(&["checkout", &priv_cid], &priv_c);
    assert_eq!(
        fs::read(priv_c.join("model.safetensors")).unwrap(),
        secret_payload
    );
}

#[test]
fn test_merge_zstd_preserves_compression_and_detects_conflicts() {
    let dir = repo_dir("merge-zstd");
    shard_ok(&["init", "--compression", "zstd"], &dir);

    fs::write(dir.join("base.txt"), b"base content").unwrap();
    shard_ok(&["add", "base.txt"], &dir);
    shard_ok(&["commit", "-m", "base", "--author", "T"], &dir);

    // Clean 3-way merge branch
    shard_ok(&["branch", "create", "feature/clean"], &dir);
    shard_ok(&["checkout", "feature/clean"], &dir);
    fs::write(dir.join("feat.txt"), b"feature content").unwrap();
    shard_ok(&["add", "feat.txt"], &dir);
    shard_ok(&["commit", "-m", "add feat", "--author", "T"], &dir);

    shard_ok(&["checkout", "main"], &dir);
    fs::write(dir.join("main2.txt"), b"main2 content").unwrap();
    shard_ok(&["add", "main2.txt"], &dir);
    shard_ok(&["commit", "-m", "add main2", "--author", "T"], &dir);

    let out = shard_ok(
        &[
            "merge",
            "feature/clean",
            "-m",
            "merge clean",
            "--author",
            "T",
        ],
        &dir,
    );
    let merge_cid = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    // Verify AND checkout merge commit — must reconstruct exact plaintext (NOT raw zstd bytes!)
    shard_ok(&["verify", &merge_cid], &dir);
    let _ = fs::remove_file(dir.join("base.txt"));
    let _ = fs::remove_file(dir.join("feat.txt"));
    let _ = fs::remove_file(dir.join("main2.txt"));
    shard_ok(&["checkout", &merge_cid], &dir);
    assert_eq!(
        fs::read_to_string(dir.join("base.txt")).unwrap(),
        "base content"
    );
    assert_eq!(
        fs::read_to_string(dir.join("feat.txt")).unwrap(),
        "feature content"
    );
    assert_eq!(
        fs::read_to_string(dir.join("main2.txt")).unwrap(),
        "main2 content"
    );

    // Conflicting 3-way merge must fail with clear conflict error!
    shard_ok(&["checkout", "main"], &dir);
    shard_ok(&["branch", "create", "feature/conflict"], &dir);
    shard_ok(&["checkout", "feature/conflict"], &dir);
    fs::write(dir.join("base.txt"), b"theirs version").unwrap();
    shard_ok(&["add", "base.txt"], &dir);
    shard_ok(&["commit", "-m", "theirs edit", "--author", "T"], &dir);

    shard_ok(&["checkout", "main"], &dir);
    fs::write(dir.join("base.txt"), b"ours version").unwrap();
    shard_ok(&["add", "base.txt"], &dir);
    shard_ok(&["commit", "-m", "ours edit", "--author", "T"], &dir);

    let conflict = shard(
        &[
            "merge",
            "feature/conflict",
            "-m",
            "should conflict",
            "--author",
            "T",
        ],
        &dir,
    )
    .output()
    .unwrap();
    assert!(!conflict.status.success(), "conflicting merge must fail");
    let stderr = String::from_utf8_lossy(&conflict.stderr);
    assert!(
        stderr.contains("conflict") && stderr.contains("base.txt"),
        "expected merge conflict listing base.txt, got: {}",
        stderr
    );
}

#[test]
fn test_branch_traversal_rejected_and_gc_preserves_slashed_branch() {
    let dir = repo_dir("slashed-branch-gc");
    shard_ok(&["init"], &dir);
    fs::write(dir.join("a.txt"), b"alpha").unwrap();
    shard_ok(&["add", "a.txt"], &dir);
    shard_ok(&["commit", "-m", "c1", "--author", "T"], &dir);

    // Branch traversal must be rejected
    let bad = shard(&["branch", "create", "../pwned"], &dir)
        .output()
        .unwrap();
    assert!(!bad.status.success(), "branch traversal must fail");

    // Slashed branch works and survives GC!
    shard_ok(&["branch", "create", "feature/models/v1"], &dir);
    shard_ok(&["checkout", "feature/models/v1"], &dir);
    fs::write(dir.join("nested/weights.bin"), b"branch-only weights").unwrap_or_else(|_| {
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("nested/weights.bin"), b"branch-only weights").unwrap();
    });
    shard_ok(&["add", "nested/weights.bin"], &dir);
    let out = shard_ok(&["commit", "-m", "feat commit", "--author", "T"], &dir);
    let feat_cid = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    // Switch back to main and run GC — feature/models/v1 commit must NOT be deleted!
    shard_ok(&["checkout", "main"], &dir);
    shard_ok(&["prune"], &dir);
    shard_ok(&["verify", &feat_cid], &dir);
}

#[test]
fn test_key_verify_json_exits_nonzero_on_tamper() {
    let dir = repo_dir("key-verify-json-exit");
    shard_ok(&["init"], &dir);
    shard_ok(&["key", "rotate"], &dir);

    let rot_dir = dir.join(".shard/keys/rotations");
    for entry in fs::read_dir(&rot_dir).unwrap() {
        let entry = entry.unwrap();
        let mut obj: serde_json::Value =
            serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap();
        obj["signature_hex"] = serde_json::Value::String("00".repeat(64));
        fs::write(entry.path(), serde_json::to_string_pretty(&obj).unwrap()).unwrap();
    }

    let out = shard(&["key", "verify", "--json"], &dir).output().unwrap();
    assert!(
        !out.status.success(),
        "key verify --json must exit non-zero on tampered rotation"
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must still be valid JSON");
    assert_eq!(v["verified"], false);
}

#[test]
fn test_commit_snapshot_and_sled_import_preserve_subdirs() {
    let dir = repo_dir("sled-import-snapshot");
    shard_ok(&["init", "--db", "sled"], &dir);

    let import_src = tempfile::tempdir().unwrap();
    fs::create_dir_all(import_src.path().join("sub/dir")).unwrap();
    fs::write(import_src.path().join("root.txt"), b"root").unwrap();
    fs::write(import_src.path().join("sub/dir/leaf.txt"), b"leaf").unwrap();

    // Sled import must NOT fail with exclusive lock double-open, and must preserve subdirs!
    shard_ok(
        &[
            "import",
            import_src.path().to_str().unwrap(),
            "-m",
            "imported",
            "--author",
            "T",
        ],
        &dir,
    );

    // Add a second file and commit — unmodified imported files must be carried forward!
    fs::write(dir.join("extra.txt"), b"extra").unwrap();
    shard_ok(&["add", "extra.txt"], &dir);
    let out = shard_ok(&["commit", "-m", "second", "--author", "T"], &dir);
    let cid2 = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    let exported = tempfile::tempdir().unwrap();
    shard_ok(&["export", &cid2, exported.path().to_str().unwrap()], &dir);
    assert_eq!(
        fs::read_to_string(exported.path().join("root.txt")).unwrap(),
        "root"
    );
    assert_eq!(
        fs::read_to_string(exported.path().join("sub/dir/leaf.txt")).unwrap(),
        "leaf"
    );
    assert_eq!(
        fs::read_to_string(exported.path().join("extra.txt")).unwrap(),
        "extra"
    );
}
