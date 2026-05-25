use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::Serialize;

const CBOR_MARKER: u8 = 0x02;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MetadataFormat {
    Json,
    Cbor,
}

impl MetadataFormat {
    pub fn from_config(config: &std::collections::BTreeMap<String, String>) -> Self {
        match config.get("serialization_format").map(|s| s.as_str()) {
            Some("cbor") => MetadataFormat::Cbor,
            _ => MetadataFormat::Json,
        }
    }

    pub fn config_value(&self) -> &'static str {
        match self {
            MetadataFormat::Json => "json",
            MetadataFormat::Cbor => "cbor",
        }
    }
}

fn sort_json_keys(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            let mut sorted = serde_json::Map::with_capacity(keys.len());
            for key in keys {
                if let Some(val) = map.get(&key) {
                    sorted.insert(key, sort_json_keys(val.clone()));
                }
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(sort_json_keys).collect())
        }
        other => other,
    }
}

pub fn serialize<T: Serialize>(data: &T, format: &MetadataFormat) -> Vec<u8> {
    match format {
        MetadataFormat::Json => {
            let value = serde_json::to_value(data).expect("JSON serialization failed");
            let sorted = sort_json_keys(value);
            serde_json::to_vec(&sorted).expect("canonical JSON serialization failed")
        }
        MetadataFormat::Cbor => {
            let mut buf = vec![CBOR_MARKER];
            ciborium::into_writer(data, &mut buf).expect("CBOR serialization failed");
            buf
        }
    }
}

pub fn deserialize<T: DeserializeOwned>(data: &[u8]) -> Result<T> {
    if data.is_empty() {
        anyhow::bail!("empty metadata");
    }
    if data[0] == CBOR_MARKER {
        return Ok(ciborium::from_reader(&data[1..])?);
    }
    Ok(serde_json::from_slice(data)?)
}

pub fn serialize_for_signing<T: Serialize>(data: &T) -> Vec<u8> {
    let value = serde_json::to_value(data).expect("JSON serialization failed");
    let sorted = sort_json_keys(value);
    serde_json::to_vec(&sorted).expect("canonical JSON serialization failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestData {
        name: String,
        value: u64,
        items: Vec<String>,
    }

    fn test_obj() -> TestData {
        TestData {
            name: "test".into(),
            value: 42,
            items: vec!["a".into(), "b".into()],
        }
    }

    #[test]
    fn test_json_roundtrip() {
        let obj = test_obj();
        let bytes = serialize(&obj, &MetadataFormat::Json);
        // JSON has no marker byte
        assert!(!bytes.is_empty());
        let decoded: TestData = deserialize(&bytes).unwrap();
        assert_eq!(decoded, obj);
    }

    #[test]
    fn test_cbor_roundtrip() {
        let obj = test_obj();
        let bytes = serialize(&obj, &MetadataFormat::Cbor);
        assert_eq!(bytes[0], CBOR_MARKER);
        let decoded: TestData = deserialize(&bytes).unwrap();
        assert_eq!(decoded, obj);
    }

    #[test]
    fn test_cbor_backward_compat() {
        // CBOR-marker data should be readable
        let obj = test_obj();
        let cbor_bytes = serialize(&obj, &MetadataFormat::Cbor);
        let decoded: TestData = deserialize(&cbor_bytes).unwrap();
        assert_eq!(decoded, obj);
    }

    #[test]
    fn test_json_backward_compat_no_marker() {
        // Legacy JSON (no marker) must still be readable
        let obj = test_obj();
        let json_bytes = serde_json::to_vec(&obj).unwrap();
        let decoded: TestData = deserialize(&json_bytes).unwrap();
        assert_eq!(decoded, obj);
    }

    #[test]
    fn test_empty_data_fails() {
        let result: Result<TestData> = deserialize(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize_for_signing_is_json() {
        let obj = test_obj();
        let bytes = serialize_for_signing(&obj);
        // Should be parseable as JSON
        let decoded: TestData = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, obj);
    }

    #[test]
    fn test_format_from_config() {
        let mut config = std::collections::BTreeMap::new();
        assert_eq!(MetadataFormat::from_config(&config), MetadataFormat::Json);
        config.insert("serialization_format".into(), "json".into());
        assert_eq!(MetadataFormat::from_config(&config), MetadataFormat::Json);
        config.insert("serialization_format".into(), "cbor".into());
        assert_eq!(MetadataFormat::from_config(&config), MetadataFormat::Cbor);
        config.insert("serialization_format".into(), "invalid".into());
        assert_eq!(MetadataFormat::from_config(&config), MetadataFormat::Json);
    }

    #[test]
    fn test_cbor_compactness() {
        let obj = test_obj();
        let json_bytes = serialize(&obj, &MetadataFormat::Json);
        let cbor_bytes = serialize(&obj, &MetadataFormat::Cbor);
        // CBOR should be smaller than JSON for this struct
        assert!(cbor_bytes.len() < json_bytes.len());
    }

    #[test]
    fn test_cbor_marker_byte() {
        let obj = test_obj();
        let bytes = serialize(&obj, &MetadataFormat::Cbor);
        // First byte must be 0x02
        assert_eq!(bytes[0], CBOR_MARKER);
        // Must have content after marker
        assert!(bytes.len() > 1);
    }

    #[test]
    fn test_cbor_btreemap_roundtrip() {
        let mut map = std::collections::BTreeMap::new();
        map.insert("key1".to_string(), "value1".to_string());
        map.insert("key2".to_string(), "value2".to_string());
        let bytes = serialize(&map, &MetadataFormat::Cbor);
        assert_eq!(bytes[0], CBOR_MARKER);
        let decoded: std::collections::BTreeMap<String, String> = deserialize(&bytes).unwrap();
        assert_eq!(decoded["key1"], "value1");
        assert_eq!(decoded["key2"], "value2");
        assert_eq!(decoded.len(), 2);
    }
}
