use shard_core::store::Store;
use std::collections::BTreeMap;
use std::sync::Mutex;
use tempfile::tempdir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn test_store_open_honors_shard_storage_backend_env_override() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved = std::env::var("SHARD_STORAGE_BACKEND").ok();

    let dir = tempdir().unwrap();
    let shard_dir = dir.path().join(".shard");
    std::fs::create_dir_all(&shard_dir).unwrap();

    // Config file specifies flat, but env overrides must take precedence.
    let mut cfg = BTreeMap::new();
    cfg.insert("storage_backend".to_string(), "flat".to_string());
    std::fs::write(
        shard_dir.join("config.json"),
        serde_json::to_vec(&cfg).unwrap(),
    )
    .unwrap();

    // 1. Override to sqlite
    std::env::set_var("SHARD_STORAGE_BACKEND", "sqlite");
    let sqlite_dir = tempdir().unwrap();
    std::fs::write(
        sqlite_dir.path().join("config.json"),
        serde_json::to_vec(&cfg).unwrap(),
    )
    .unwrap();
    let store = Store::open(sqlite_dir.path()).unwrap();
    assert!(matches!(store, Store::Sqlite(_)));
    drop(store);

    // 2. Override to sled
    std::env::set_var("SHARD_STORAGE_BACKEND", "sled");
    let sled_dir = tempdir().unwrap();
    std::fs::write(
        sled_dir.path().join("config.json"),
        serde_json::to_vec(&cfg).unwrap(),
    )
    .unwrap();
    let store = Store::open(sled_dir.path()).unwrap();
    assert!(matches!(store, Store::Sled(_)));
    drop(store);

    // 3. Override to flat even when config.json says sqlite
    let mut sqlite_cfg = BTreeMap::new();
    sqlite_cfg.insert("storage_backend".to_string(), "sqlite".to_string());
    std::fs::write(
        shard_dir.join("config.json"),
        serde_json::to_vec(&sqlite_cfg).unwrap(),
    )
    .unwrap();
    std::env::set_var("SHARD_STORAGE_BACKEND", "flat");
    let store = Store::open(&shard_dir).unwrap();
    assert!(matches!(store, Store::Flat(_)));
    drop(store);

    // Restore original env
    if let Some(val) = saved {
        std::env::set_var("SHARD_STORAGE_BACKEND", val);
    } else {
        std::env::remove_var("SHARD_STORAGE_BACKEND");
    }
}
