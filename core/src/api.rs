use crate::branch;
use crate::index::Index;
use crate::metadata::{self, MetadataFormat};
use crate::store::Store;
use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

#[derive(Clone)]
struct AppState {
    pub repo_path: PathBuf,
    pub shard_dir: PathBuf,
}

#[derive(Deserialize)]
struct InitRequest {
    backend: Option<String>,
    compression: Option<String>,
    chunker: Option<String>,
    chunk_size: Option<u64>,
    is_private: Option<bool>,
    passphrase: Option<String>,
}

#[derive(Deserialize)]
struct AddRequest {
    path: String,
}

#[derive(Deserialize)]
struct CommitRequest {
    message: String,
    author: Option<String>,
}

#[derive(Deserialize)]
struct PullRequest {
    peer: String,
    commit_id: String,
}

#[derive(Deserialize)]
struct PushRequest {
    peer: String,
}

#[derive(Deserialize)]
struct BranchCreateRequest {
    name: String,
    commit_id: Option<String>,
}

fn err_json(e: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"status": "error", "error": e.to_string()})),
    )
}

fn ok_json(v: serde_json::Value) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::OK, Json(v))
}

pub async fn serve(path: &Path, addr: &str, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    let state = AppState {
        repo_path: path.to_path_buf(),
        shard_dir,
    };

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/api/v1/init", post(init_handler))
        .route("/api/v1/add", post(add_handler))
        .route("/api/v1/commit", post(commit_handler))
        .route("/api/v1/log", get(log_handler))
        .route("/api/v1/status", get(status_handler))
        .route("/api/v1/pull", post(pull_handler))
        .route("/api/v1/push", post(push_handler))
        .route("/api/v1/branch", get(branch_list_handler))
        .route("/api/v1/branch/create", post(branch_create_handler))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(Arc::new(Mutex::new(state)));

    let listener = tokio::net::TcpListener::bind(addr).await?;
    if json {
        info!(
            "{}",
            serde_json::json!({"event": "api_start", "addr": addr})
        );
    } else {
        info!("API server listening on {}", addr);
    }
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

async fn init_handler(
    State(state): State<Arc<Mutex<AppState>>>,
    Json(req): Json<InitRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let s = state.lock().await;
    let pass = req.passphrase.as_deref().unwrap_or("");
    match crate::init_with_passphrase(
        &s.repo_path,
        req.backend.as_deref().unwrap_or("flat"),
        req.compression.as_deref().unwrap_or("zstd"),
        req.chunker.as_deref().unwrap_or("fixed"),
        req.chunk_size,
        req.is_private.unwrap_or(false),
        false,
        pass,
    ) {
        Ok(()) => ok_json(serde_json::json!({"status": "ok", "message": "initialized"})),
        Err(e) => err_json(e),
    }
}

async fn add_handler(
    State(state): State<Arc<Mutex<AppState>>>,
    Json(req): Json<AddRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let s = state.lock().await;
    match crate::add(&s.repo_path, &PathBuf::from(&req.path), false) {
        Ok(()) => ok_json(serde_json::json!({"status": "ok", "message": "added"})),
        Err(e) => err_json(e),
    }
}

async fn commit_handler(
    State(state): State<Arc<Mutex<AppState>>>,
    Json(req): Json<CommitRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let s = state.lock().await;
    let author = req.author.as_deref().unwrap_or("User <user@example.com>");
    match crate::commit(&s.repo_path, &req.message, author, false) {
        Ok(commit_id) => ok_json(serde_json::json!({"status": "ok", "commit_id": commit_id})),
        Err(e) => err_json(e),
    }
}

async fn log_handler(
    State(state): State<Arc<Mutex<AppState>>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let s = state.lock().await;
    let shard_dir = &s.shard_dir;
    if !shard_dir.exists() {
        return err_json("not a shard repository");
    }
    let store = match Store::open(shard_dir) {
        Ok(s) => s,
        Err(e) => return err_json(e),
    };
    let head = branch::resolve_head(shard_dir).ok().and_then(|(_, h)| h);

    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut stack = head.clone().into_iter().collect::<Vec<_>>();
    let mut seen = std::collections::HashSet::new();
    while let Some(cid) = stack.pop() {
        if !seen.insert(cid.clone()) {
            continue;
        }
        if let Ok(data) = store.get_chunk(&cid) {
            if let Ok(commit) = metadata::deserialize::<crate::commit::Commit>(&data) {
                entries.push(serde_json::json!({
                    "commit_id": cid,
                    "message": commit.message,
                    "author": commit.author,
                    "timestamp": commit.timestamp.to_string(),
                }));
                for p in &commit.parents {
                    stack.push(p.clone());
                }
            }
        }
    }
    ok_json(serde_json::json!({"status": "ok", "entries": entries}))
}

async fn status_handler(
    State(state): State<Arc<Mutex<AppState>>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let s = state.lock().await;
    let shard_dir = &s.shard_dir;
    if !shard_dir.exists() {
        return err_json("not a shard repository");
    }
    let (current_branch, head) = branch::resolve_head(shard_dir).unwrap_or((None, None));
    let index = Index::load(shard_dir, &MetadataFormat::Json).ok();
    let staged: Vec<String> = index
        .as_ref()
        .map(|i| i.files.keys().cloned().collect())
        .unwrap_or_default();
    ok_json(serde_json::json!({
        "status": "ok",
        "current_branch": current_branch,
        "head": head,
        "staged": staged,
    }))
}

async fn pull_handler(
    State(state): State<Arc<Mutex<AppState>>>,
    Json(req): Json<PullRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let s = state.lock().await;
    match crate::pull(&s.repo_path, &req.peer, &req.commit_id, false).await {
        Ok(()) => ok_json(serde_json::json!({"status": "ok", "message": "pulled"})),
        Err(e) => err_json(e),
    }
}

async fn push_handler(
    State(state): State<Arc<Mutex<AppState>>>,
    Json(req): Json<PushRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let s = state.lock().await;
    match crate::push(&s.repo_path, &req.peer, false).await {
        Ok(()) => ok_json(serde_json::json!({"status": "ok", "message": "pushed"})),
        Err(e) => err_json(e),
    }
}

async fn branch_list_handler(
    State(state): State<Arc<Mutex<AppState>>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let s = state.lock().await;
    let shard_dir = &s.shard_dir;
    if !shard_dir.exists() {
        return err_json("not a shard repository");
    }
    let (current, branches) = branch::list_branches(shard_dir).unwrap_or((None, Vec::new()));
    let branch_list: Vec<serde_json::Value> = branches
        .into_iter()
        .map(|(name, tip)| serde_json::json!({"name": name, "tip": tip}))
        .collect();
    ok_json(serde_json::json!({"status": "ok", "current": current, "branches": branch_list}))
}

async fn branch_create_handler(
    State(state): State<Arc<Mutex<AppState>>>,
    Json(req): Json<BranchCreateRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let s = state.lock().await;
    match crate::branch_create(&s.repo_path, &req.name, req.commit_id.as_deref()) {
        Ok(()) => ok_json(serde_json::json!({"status": "ok", "message": "branch_created"})),
        Err(e) => err_json(e),
    }
}
