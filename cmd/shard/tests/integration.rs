use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn shard_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_shard"))
}

fn repo_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("shard-tests")
        .join(name);
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
    assert!(output.status.success(), "init failed: {}", String::from_utf8_lossy(&output.stderr));
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
    assert!(stderr.contains("already initialized"), "wrong error: {stderr}");
}

#[test]
fn test_add_commit_verify_roundtrip() {
    let dir = repo_dir("add-commit-verify");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("hello.txt"), b"Hello, Shard!").unwrap();
    let output = shard(&["add", "hello.txt"], &dir).output().unwrap();
    assert!(output.status.success(), "add failed: {}", String::from_utf8_lossy(&output.stderr));

    let output = shard(&["commit", "-m", "first", "--author", "Test"], &dir).output().unwrap();
    assert!(output.status.success(), "commit failed: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).expect("no commit id in output");

    let output = shard(&["verify", commit_id], &dir).output().unwrap();
    assert!(output.status.success(), "verify failed: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Signature verified"), "no signature verification: {stdout}");
    assert!(stdout.contains("Verification successful"), "verify not successful: {stdout}");
}

#[test]
fn test_verify_fails_on_tampered_chunk() {
    let dir = repo_dir("tamper-test");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("data.bin"), b"important data").unwrap();
    shard(&["add", "data.bin"], &dir).output().unwrap();
    let output = shard(&["commit", "-m", "tamper-me", "--author", "Test"], &dir).output().unwrap();
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
    assert!(!output.status.success(), "verify should have failed after tampering");
}

#[test]
fn test_empty_commit_fails() {
    let dir = repo_dir("empty-commit");
    shard(&["init"], &dir).output().unwrap();
    let output = shard(&["commit", "-m", "empty", "--author", "Test"], &dir).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Nothing to commit"), "wrong error: {stderr}");
}

#[test]
fn test_verify_nonexistent_commit_fails() {
    let dir = repo_dir("bad-verify");
    shard(&["init"], &dir).output().unwrap();
    let output = shard(&["verify", "0000000000000000000000000000000000000000000000000000000000000000"], &dir).output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_multiple_adds_and_commits() {
    let dir = repo_dir("multi-commit");
    shard(&["init"], &dir).output().unwrap();

    fs::write(dir.join("a.txt"), b"file a").unwrap();
    shard(&["add", "a.txt"], &dir).output().unwrap();
    let out1 = shard(&["commit", "-m", "commit a", "--author", "T"], &dir).output().unwrap();
    let id1 = String::from_utf8(out1.stdout).unwrap().split_whitespace().nth(1).unwrap().to_string();

    fs::write(dir.join("b.txt"), b"file b").unwrap();
    shard(&["add", "b.txt"], &dir).output().unwrap();
    let out2 = shard(&["commit", "-m", "commit b", "--author", "T"], &dir).output().unwrap();
    let id2 = String::from_utf8(out2.stdout).unwrap().split_whitespace().nth(1).unwrap().to_string();

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
    let output = shard(&["commit", "-m", "large", "--author", "T"], &dir).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let commit_id = stdout.split_whitespace().nth(1).unwrap();

    shard(&["verify", commit_id], &dir).output().unwrap();
}
