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
    shard_core::commit(&repo, "test", "Test <test@test.com>", false).unwrap();

    let (_, head) = shard_core::branch::resolve_head(&repo.join(".shard")).unwrap();
    assert!(head.is_some());
    shard_core::verify(&repo, &head.unwrap(), false).unwrap();
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
    shard_core::commit(&repo, "multi", "Test <test@test.com>", false).unwrap();

    let (_, head) = shard_core::branch::resolve_head(&repo.join(".shard")).unwrap();
    assert!(head.is_some());
    shard_core::verify(&repo, &head.unwrap(), false).unwrap();
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
    shard_core::status(&repo, false).unwrap();
}

#[test]
fn workspace_checkout_after_commit() {
    let (_dir, repo) = init_repo();
    fs::write(repo.join("checkme.txt"), b"checkout data").unwrap();
    shard_core::add(&repo, &repo.join("checkme.txt"), false).unwrap();
    shard_core::commit(&repo, "checkout test", "Test <test@test.com>", false).unwrap();

    let (_, head) = shard_core::branch::resolve_head(&repo.join(".shard")).unwrap();
    let head = head.unwrap();

    fs::remove_file(repo.join("checkme.txt")).unwrap();
    shard_core::checkout(&repo, &head, false).unwrap();
    assert_eq!(fs::read_to_string(repo.join("checkme.txt")).unwrap(), "checkout data");
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
}

#[test]
fn workspace_branch_create_switch_list() {
    let (_dir, repo) = init_repo();
    fs::write(repo.join("main.txt"), b"main").unwrap();
    shard_core::add(&repo, &repo.join("main.txt"), false).unwrap();
    shard_core::commit(&repo, "first", "Test <test@test.com>", false).unwrap();
    shard_core::branch_create(&repo, "feature", None).unwrap();
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
    shard_core::commit(&repo, "tag test", "Test <test@test.com>", false).unwrap();
    let (_, head) = shard_core::branch::resolve_head(&repo.join(".shard")).unwrap();
    shard_core::tag_add(&repo, "v1.0", &head.unwrap()).unwrap();
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
    shard_core::commit(repo, "private", "Test <test@test.com>", false).unwrap();

    let (_, head) = shard_core::branch::resolve_head(&repo.join(".shard")).unwrap();
    shard_core::verify(repo, &head.unwrap(), false).unwrap();
}
