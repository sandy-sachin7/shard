use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Commit {
    pub commit_id: String,
    pub parents: Vec<String>,
    pub manifests: Vec<String>,
    pub author: String,
    pub message: String,
    pub timestamp: u64,
    pub public_key: Option<String>,
    pub signature: Option<String>,
    #[serde(default)]
    pub key_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_commit_serialization_roundtrip() {
        let c = Commit {
            commit_id: "abc".into(),
            parents: vec!["parent1".into()],
            manifests: vec!["man1".into()],
            author: "Test".into(),
            message: "test message".into(),
            timestamp: 1000,
            public_key: Some("pk".into()),
            signature: Some("sig".into()),
            key_id: Some("key1".into()),
        };
        let json = serde_json::to_vec(&c).unwrap();
        let c2: Commit = serde_json::from_slice(&json).unwrap();
        assert_eq!(c.commit_id, c2.commit_id);
        assert_eq!(c.author, c2.author);
        assert_eq!(c.message, c2.message);
        assert_eq!(c.timestamp, c2.timestamp);
        assert_eq!(c.key_id, c2.key_id);
    }

    #[test]
    fn test_commit_key_id_backward_compat() {
        let json = r#"{"commit_id":"x","parents":[],"manifests":[],"author":"A","message":"M","timestamp":0}"#;
        let c: Commit = serde_json::from_str(json).unwrap();
        assert!(c.key_id.is_none());
        assert_eq!(c.commit_id, "x");
    }

    #[test]
    fn test_commit_empty_parents() {
        let c = Commit {
            commit_id: "root".into(),
            parents: vec![],
            manifests: vec![],
            author: "Root".into(),
            message: "root commit".into(),
            timestamp: 0,
            public_key: None,
            signature: None,
            key_id: None,
        };
        assert!(c.parents.is_empty());
        assert!(c.public_key.is_none());
    }

    #[test]
    fn test_commit_with_all_fields() {
        let c = Commit {
            commit_id: "full".into(),
            parents: vec!["p1".into(), "p2".into()],
            manifests: vec!["m1".into(), "m2".into(), "m3".into()],
            author: "Author <a@b.com>".into(),
            message: "merge commit".into(),
            timestamp: 1234567890,
            public_key: Some("pkhex".into()),
            signature: Some("sighex".into()),
            key_id: Some("keyid".into()),
        };
        let json = serde_json::to_string_pretty(&c).unwrap();
        let c2: Commit = serde_json::from_str(&json).unwrap();
        assert_eq!(c2.parents.len(), 2);
        assert_eq!(c2.manifests.len(), 3);
    }
}
