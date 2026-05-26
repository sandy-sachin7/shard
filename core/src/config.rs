use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path;

const ENV_PREFIX: &str = "SHARD_";

pub fn env_to_config_key(env_name: &str) -> Option<String> {
    let rest = env_name.strip_prefix(ENV_PREFIX)?;
    Some(rest.to_lowercase().replace('_', "."))
}

pub fn load_env_overrides() -> BTreeMap<String, String> {
    let mut overrides = BTreeMap::new();
    for (key, value) in std::env::vars() {
        if let Some(config_key) = env_to_config_key(&key) {
            overrides.insert(config_key, value);
        }
    }
    overrides
}

pub fn load_config_env(shard_dir: &Path) -> Result<()> {
    let config_path = shard_dir.join("config.json");
    let mut config: BTreeMap<String, String> = if config_path.exists() {
        let data = std::fs::read(&config_path)?;
        serde_json::from_slice(&data)?
    } else {
        BTreeMap::new()
    };
    let overrides = load_env_overrides();
    let mut changed = false;
    for (k, v) in overrides {
        if config.get(&k).map(|s| s.as_str()) != Some(v.as_str()) {
            config.insert(k, v);
            changed = true;
        }
    }
    if changed {
        let data = serde_json::to_vec_pretty(&config)?;
        std::fs::write(&config_path, data)?;
    }
    Ok(())
}

pub fn validate_config(config: &BTreeMap<String, String>) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(backend) = config.get("storage_backend") {
        match backend.as_str() {
            "flat" | "sled" | "sqlite" => {}
            _ => errors.push(format!("Unknown storage_backend: {}", backend)),
        }
    }
    if let Some(compression) = config.get("compression") {
        match compression.as_str() {
            "none" | "zstd" | "gzip" => {}
            s if s.starts_with("zstd(") => {}
            _ => errors.push(format!("Unknown compression: {}", compression)),
        }
    }
    if let Some(chunker) = config.get("chunker_mode") {
        match chunker.as_str() {
            "fixed" | "rabin" => {}
            _ => errors.push(format!("Unknown chunker_mode: {}", chunker)),
        }
    }
    if let Some(cs) = config.get("chunk_size") {
        if cs.parse::<u64>().is_err() || cs.parse::<u64>().unwrap_or(0) == 0 {
            errors.push(format!("Invalid chunk_size: {}", cs));
        }
    }
    if let Some(rate) = config.get("rate_limit_max_requests") {
        if rate.parse::<u32>().is_err() {
            errors.push(format!("Invalid rate_limit_max_requests: {}", rate));
        }
    }
    if let Some(interval) = config.get("rate_limit_window_secs") {
        if interval.parse::<u64>().is_err() {
            errors.push(format!("Invalid rate_limit_window_secs: {}", interval));
        }
    }
    if let Some(gc) = config.get("gc_interval_secs") {
        if gc.parse::<u64>().is_err() {
            errors.push(format!("Invalid gc_interval_secs: {}", gc));
        }
    }
    errors
}

pub fn config_get_rate_limit_max(config: &BTreeMap<String, String>) -> u32 {
    config
        .get("rate_limit_max_requests")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
}

pub fn config_get_rate_limit_window(config: &BTreeMap<String, String>) -> u64 {
    config
        .get("rate_limit_window_secs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(60)
}

pub fn config_get_gc_interval(config: &BTreeMap<String, String>) -> u64 {
    config
        .get("gc_interval_secs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600)
}

pub fn config_get_gc_enabled(config: &BTreeMap<String, String>) -> bool {
    config
        .get("gc_enabled")
        .map(|s| s == "true")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_to_config_key() {
        assert_eq!(
            env_to_config_key("SHARD_STORAGE_BACKEND"),
            Some("storage.backend".to_string())
        );
        assert_eq!(
            env_to_config_key("SHARD_COMPRESSION"),
            Some("compression".to_string())
        );
        assert_eq!(env_to_config_key("NOT_SHARD"), None);
    }

    #[test]
    fn test_validate_config_valid() {
        let mut config = BTreeMap::new();
        config.insert("storage_backend".to_string(), "flat".to_string());
        config.insert("compression".to_string(), "zstd".to_string());
        config.insert("chunker_mode".to_string(), "fixed".to_string());
        config.insert("chunk_size".to_string(), "4194304".to_string());
        assert!(validate_config(&config).is_empty());
    }

    #[test]
    fn test_validate_config_invalid() {
        let mut config = BTreeMap::new();
        config.insert("storage_backend".to_string(), "unknown".to_string());
        config.insert("compression".to_string(), "bad".to_string());
        config.insert("chunk_size".to_string(), "not_a_number".to_string());
        let errors = validate_config(&config);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("unknown")));
    }

    #[test]
    fn test_config_defaults() {
        let config = BTreeMap::new();
        assert_eq!(config_get_rate_limit_max(&config), 50);
        assert_eq!(config_get_rate_limit_window(&config), 60);
        assert_eq!(config_get_gc_interval(&config), 3600);
        assert!(!config_get_gc_enabled(&config));
    }

    #[test]
    fn test_config_custom() {
        let mut config = BTreeMap::new();
        config.insert("rate_limit_max_requests".to_string(), "100".to_string());
        config.insert("rate_limit_window_secs".to_string(), "30".to_string());
        config.insert("gc_enabled".to_string(), "true".to_string());
        assert_eq!(config_get_rate_limit_max(&config), 100);
        assert_eq!(config_get_rate_limit_window(&config), 30);
        assert!(config_get_gc_enabled(&config));
    }
}
