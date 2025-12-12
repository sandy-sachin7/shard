use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct Commit {
    pub commit_id: String, // Self-reference? No, usually computed after.
    // But spec says "Commit node: JSON with commit_id...".
    // If it's inside the JSON, it changes the hash.
    // Usually commit_id is the hash of the content.
    // I'll omit commit_id from the struct for serialization, or it's a derived property.
    // The spec might mean the JSON *object* has a commit_id field when returned by API, but stored without it?
    // Or maybe it's like a block header?
    // "Commit node: JSON with commit_id (hash of canonical commit JSON)..."
    // This implies commit_id is NOT in the JSON that is hashed.

    pub parents: Vec<String>,
    pub manifests: Vec<String>, // Hash of manifest objects
    pub author: String,
    pub message: String,
    pub timestamp: u64,
    pub signature: Option<String>,
}
