use crate::branch;
use crate::fs_util::safe_repo_join;
use crate::index::Index;
use crate::metadata::{self, MetadataFormat};
use crate::store::Store;
use anyhow::Result;
use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rand::RngCore;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing::info;

#[derive(Clone)]
struct AppState {
    pub repo_path: PathBuf,
    pub shard_dir: PathBuf,
    pub api_token: String,
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

fn err_json(status: StatusCode, e: impl ToString) -> (StatusCode, Json<serde_json::Value>) {
    (
        status,
        Json(serde_json::json!({"status": "error", "error": e.to_string()})),
    )
}

fn ok_json(v: serde_json::Value) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::OK, Json(v))
}

fn generate_random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

async fn bearer_auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let provided = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let authorized = match provided {
        Some(token) => {
            let a = token.as_bytes();
            let b = state.api_token.as_bytes();
            a.len() == b.len() && bool::from(a.ct_eq(b))
        }
        None => false,
    };

    if !authorized {
        return err_json(StatusCode::UNAUTHORIZED, "missing or invalid bearer token")
            .into_response();
    }
    next.run(req).await
}

/// Build the hardened Axum router with constant-time bearer auth on `/api/v1/*`
/// and no permissive CORS by default (optional allowlist via `SHARD_API_CORS_ORIGINS`).
pub fn build_router(repo_path: PathBuf, api_token: String) -> Router {
    let shard_dir = repo_path.join(".shard");
    let state = Arc::new(AppState {
        repo_path,
        shard_dir,
        api_token,
    });

    let api_routes = Router::new()
        .route("/api/v1/init", post(init_handler))
        .route("/api/v1/add", post(add_handler))
        .route("/api/v1/commit", post(commit_handler))
        .route("/api/v1/log", get(log_handler))
        .route("/api/v1/status", get(status_handler))
        .route("/api/v1/pull", post(pull_handler))
        .route("/api/v1/push", post(push_handler))
        .route("/api/v1/branch", get(branch_list_handler))
        .route("/api/v1/branch/create", post(branch_create_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            bearer_auth_middleware,
        ));

    let mut router = Router::new()
        .route("/health", get(health_handler))
        .merge(api_routes)
        .with_state(state);

    // Optional explicit CORS allowlist; by default cross-origin browser access is denied.
    if let Ok(origins_str) = std::env::var("SHARD_API_CORS_ORIGINS") {
        let origins: Vec<HeaderValue> = origins_str
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|s| HeaderValue::from_str(s).ok())
            .collect();
        if !origins.is_empty() {
            let cors = CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
            router = router.layer(cors);
        }
    }

    router
}

pub async fn serve(path: &Path, addr: &str, json: bool) -> Result<()> {
    let (token, auto_generated) = match std::env::var("SHARD_API_TOKEN") {
        Ok(t) if !t.is_empty() => (t, false),
        _ => (generate_random_token(), true),
    };

    let app = build_router(path.to_path_buf(), token.clone());
    let listener = tokio::net::TcpListener::bind(addr).await?;

    if json {
        let mut payload = serde_json::json!({"event": "api_start", "addr": addr});
        if auto_generated {
            payload["token"] = serde_json::Value::String(token);
        }
        println!("{}", payload);
    } else {
        info!("API server listening on {}", addr);
        if auto_generated {
            eprintln!("Generated ephemeral API bearer token: {}", token);
        }
    }
    let gc_task = crate::gc::GcTaskHandle::spawn_if_enabled(path);
    let serve_res = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    gc_task.shutdown().await;
    serve_res?;
    Ok(())
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

async fn init_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InitRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let repo_path = state.repo_path.clone();
    let res = tokio::task::spawn_blocking(move || {
        let pass = req.passphrase.as_deref().unwrap_or("");
        crate::init_with_passphrase(
            &repo_path,
            req.backend.as_deref().unwrap_or("flat"),
            req.compression.as_deref().unwrap_or("zstd"),
            req.chunker.as_deref().unwrap_or("fixed"),
            req.chunk_size,
            req.is_private.unwrap_or(false),
            true,
            pass,
        )
    })
    .await;

    match res {
        Ok(Ok(())) => ok_json(serde_json::json!({"status": "ok", "message": "initialized"})),
        Ok(Err(e)) => err_json(StatusCode::BAD_REQUEST, e),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn add_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AddRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Strictly confine `/api/v1/add` path inside `repo_path`!
    let safe_path = match safe_repo_join(&state.repo_path, &req.path) {
        Ok(p) => p,
        Err(e) => return err_json(StatusCode::BAD_REQUEST, e),
    };
    let repo_path = state.repo_path.clone();
    let res = tokio::task::spawn_blocking(move || crate::add(&repo_path, &safe_path, true)).await;

    match res {
        Ok(Ok(())) => ok_json(serde_json::json!({"status": "ok", "message": "added"})),
        Ok(Err(e)) => err_json(StatusCode::BAD_REQUEST, e),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn commit_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CommitRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let repo_path = state.repo_path.clone();
    let res = tokio::task::spawn_blocking(move || {
        let author = req.author.as_deref().unwrap_or("User <user@example.com>");
        crate::commit(&repo_path, &req.message, author, true)
    })
    .await;

    match res {
        Ok(Ok(commit_id)) => ok_json(serde_json::json!({"status": "ok", "commit_id": commit_id})),
        Ok(Err(e)) => err_json(StatusCode::BAD_REQUEST, e),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn log_handler(State(state): State<Arc<AppState>>) -> (StatusCode, Json<serde_json::Value>) {
    let repo_path = state.repo_path.clone();
    let shard_dir = state.shard_dir.clone();
    let res = tokio::task::spawn_blocking(move || -> Result<Vec<serde_json::Value>> {
        crate::with_read_op(&repo_path, "api_log", || {
            if !shard_dir.exists() {
                anyhow::bail!("not a shard repository");
            }
            let store = Store::open(&shard_dir)?;
            let head = branch::resolve_head(&shard_dir).ok().and_then(|(_, h)| h);

            let mut entries = Vec::new();
            let mut stack = head.into_iter().collect::<Vec<_>>();
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
            Ok(entries)
        })
    })
    .await;

    match res {
        Ok(Ok(entries)) => ok_json(serde_json::json!({"status": "ok", "entries": entries})),
        Ok(Err(e)) => err_json(StatusCode::BAD_REQUEST, e),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn status_handler(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let repo_path = state.repo_path.clone();
    let shard_dir = state.shard_dir.clone();
    let res = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        crate::with_read_op(&repo_path, "api_status", || {
            if !shard_dir.exists() {
                anyhow::bail!("not a shard repository");
            }
            let config = crate::load_config(&shard_dir)?;
            let fmt = MetadataFormat::from_config(&config);
            let (current_branch, head) = branch::resolve_head(&shard_dir).unwrap_or((None, None));
            let index = Index::load(&shard_dir.join("index"), &fmt)?;
            let mut staged: Vec<String> = index.files.keys().cloned().collect();
            staged.sort();
            Ok(serde_json::json!({
                "status": "ok",
                "current_branch": current_branch,
                "head": head,
                "staged": staged,
            }))
        })
    })
    .await;

    match res {
        Ok(Ok(v)) => ok_json(v),
        Ok(Err(e)) => err_json(StatusCode::BAD_REQUEST, e),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn pull_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PullRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match crate::pull(&state.repo_path, &req.peer, &req.commit_id, true).await {
        Ok(()) => ok_json(serde_json::json!({"status": "ok", "message": "pulled"})),
        Err(e) => err_json(StatusCode::BAD_REQUEST, e),
    }
}

async fn push_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PushRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match crate::push(&state.repo_path, &req.peer, true).await {
        Ok(()) => ok_json(serde_json::json!({"status": "ok", "message": "pushed"})),
        Err(e) => err_json(StatusCode::BAD_REQUEST, e),
    }
}

async fn branch_list_handler(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let repo_path = state.repo_path.clone();
    let shard_dir = state.shard_dir.clone();
    let res = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
        crate::with_read_op(&repo_path, "api_branch_list", || {
            if !shard_dir.exists() {
                anyhow::bail!("not a shard repository");
            }
            let (current, branches) = branch::list_branches(&shard_dir)?;
            let branch_list: Vec<serde_json::Value> = branches
                .into_iter()
                .map(|(name, tip)| serde_json::json!({"name": name, "tip": tip}))
                .collect();
            Ok(serde_json::json!({"status": "ok", "current": current, "branches": branch_list}))
        })
    })
    .await;

    match res {
        Ok(Ok(v)) => ok_json(v),
        Ok(Err(e)) => err_json(StatusCode::BAD_REQUEST, e),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn branch_create_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BranchCreateRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let repo_path = state.repo_path.clone();
    let res = tokio::task::spawn_blocking(move || {
        crate::branch_create(&repo_path, &req.name, req.commit_id.as_deref())
    })
    .await;

    match res {
        Ok(Ok(())) => ok_json(serde_json::json!({"status": "ok", "message": "branch_created"})),
        Ok(Err(e)) => err_json(StatusCode::BAD_REQUEST, e),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tempfile::tempdir;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_api_requires_bearer_token_and_confines_paths() {
        let dir = tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        crate::init(&repo, "flat", "zstd", "fixed", None, false, false).unwrap();
        std::fs::write(repo.join("model.txt"), b"api weights").unwrap();

        let token = "test-secret-token-123".to_string();
        let app = build_router(repo.clone(), token.clone());

        // 1. /health is accessible
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().get("access-control-allow-origin").is_none());

        // 2. Unauthenticated request to /api/v1/status must be 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // 3. Wrong bearer token must be 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // 4. Path traversal in /api/v1/add must be rejected with 400
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/add")
                    .header("authorization", format!("Bearer {}", token))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path":"/etc/passwd"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // 5. Valid /api/v1/add succeeds and shows up in /api/v1/status!
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/add")
                    .header("authorization", format!("Bearer {}", token))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path":"model.txt"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .unwrap();
        let status_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(status_json["staged"], serde_json::json!(["model.txt"]));

        // 6. /api/v1/commit returns the real 64-char hex commit_id (NOT null!)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commit")
                    .header("authorization", format!("Bearer {}", token))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"message":"api commit","author":"API"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .unwrap();
        let commit_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let cid = commit_json["commit_id"].as_str().unwrap();
        assert_eq!(cid.len(), 64);
    }
}
