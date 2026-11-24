use thiserror::Error;

/// Structured domain error type for Shard core operations.
#[derive(Debug, Error)]
pub enum ShardError {
    #[error("not a shard repository (run `shard init` first)")]
    NotARepo,

    #[error("repository already initialized at {0} (run `shard status` to confirm)")]
    AlreadyInitialized(String),

    #[error("invalid object id '{0}': expected 64 lowercase hexadecimal characters")]
    InvalidObjectId(String),

    #[error("unsafe path '{0}': {1}")]
    PathTraversal(String, String),

    #[error("invalid branch name '{0}': {1}")]
    InvalidBranchName(String, String),

    #[error("object not found: {0}")]
    ObjectNotFound(String),

    #[error("hash mismatch for {kind} '{id}': expected {expected}, got {actual}")]
    HashMismatch {
        kind: &'static str,
        id: String,
        expected: String,
        actual: String,
    },

    #[error("decompressed size exceeds limit of {0} bytes")]
    DecompressionLimitExceeded(usize),

    #[error("commit '{0}' is unsigned (re-run with --allow-unsigned to bypass)")]
    UnsignedCommit(String),

    #[error(
        "untrusted signing key for commit '{0}': key is not in local keychain or authorized_keys"
    )]
    UntrustedKey(String),

    #[error("signature verification failed: {0}")]
    SignatureVerificationFailed(String),

    #[error("merge conflict in {count} file(s): {files}")]
    MergeConflict { count: usize, files: String },

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
