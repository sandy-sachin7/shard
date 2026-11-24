use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use std::path::PathBuf;

#[pyclass]
struct CommitResult {
    #[pyo3(get)]
    commit_id: String,
    #[pyo3(get)]
    message: String,
    #[pyo3(get)]
    author: String,
    #[pyo3(get)]
    timestamp: String,
}

fn to_py_err(e: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

#[pyfunction]
#[pyo3(signature = (repo_path=None, private=false, db=None, compression=None, chunker=None, passphrase=None))]
fn init(
    py: Python<'_>,
    repo_path: Option<String>,
    private: bool,
    db: Option<String>,
    compression: Option<String>,
    chunker: Option<String>,
    passphrase: Option<String>,
) -> PyResult<String> {
    let path = PathBuf::from(repo_path.unwrap_or_else(|| ".".to_string()));
    let backend = db.unwrap_or_else(|| "flat".to_string());
    let comp = compression.unwrap_or_else(|| "zstd".to_string());
    let chunker_mode = chunker.unwrap_or_else(|| "fixed".to_string());
    let pass = passphrase.unwrap_or_default();

    py.allow_threads(move || {
        shard_core::init_with_passphrase(
            &path,
            &backend,
            &comp,
            &chunker_mode,
            None,
            private,
            true,
            &pass,
        )
    })
    .map_err(to_py_err)?;

    Ok("initialized".to_string())
}

#[pyfunction]
#[pyo3(signature = (file_path, repo_path=None))]
fn add(py: Python<'_>, file_path: String, repo_path: Option<String>) -> PyResult<String> {
    let repo = PathBuf::from(repo_path.unwrap_or_else(|| ".".to_string()));
    let target = PathBuf::from(file_path);

    py.allow_threads(move || shard_core::add(&repo, &target, true))
        .map_err(to_py_err)?;

    Ok("added".to_string())
}

#[pyfunction]
#[pyo3(signature = (message, repo_path=None, author=None))]
fn commit(
    py: Python<'_>,
    message: String,
    repo_path: Option<String>,
    author: Option<String>,
) -> PyResult<CommitResult> {
    let repo = PathBuf::from(repo_path.unwrap_or_else(|| ".".to_string()));
    let author_str = author.unwrap_or_else(|| "User <user@example.com>".to_string());
    let msg_clone = message.clone();
    let author_clone = author_str.clone();

    let commit_id =
        py.allow_threads(move || shard_core::commit(&repo, &msg_clone, &author_clone, true))
            .map_err(to_py_err)?;

    Ok(CommitResult {
        commit_id,
        message,
        author: author_str,
        timestamp: now_unix_secs(),
    })
}

#[pyfunction]
#[pyo3(signature = (repo_path=None, limit=None))]
fn log(py: Python<'_>, repo_path: Option<String>, limit: Option<usize>) -> PyResult<Vec<String>> {
    let repo = PathBuf::from(repo_path.unwrap_or_else(|| ".".to_string()));
    let entries = py
        .allow_threads(move || shard_core::log_entries(&repo, limit))
        .map_err(to_py_err)?;

    Ok(entries
        .into_iter()
        .map(|e| format!("{} {} ({})", e.commit_id, e.message, e.author))
        .collect())
}

#[pyfunction]
#[pyo3(signature = (repo_path=None))]
fn status(py: Python<'_>, repo_path: Option<String>) -> PyResult<String> {
    let repo = PathBuf::from(repo_path.unwrap_or_else(|| ".".to_string()));
    let shard_dir = repo.join(".shard");

    py.allow_threads(move || -> anyhow::Result<String> {
        if !shard_dir.exists() {
            anyhow::bail!("not a shard repository");
        }
        let (branch, head) = shard_core::branch::resolve_head(&shard_dir)?;
        let index = shard_core::index::Index::load(
            &shard_dir.join("index"),
            &shard_core::metadata::MetadataFormat::Json,
        )?;
        let mut staged: Vec<String> = index.files.keys().cloned().collect();
        staged.sort();
        Ok(serde_json::to_string(&serde_json::json!({
            "branch": branch,
            "head": head,
            "staged": staged,
        }))?)
    })
    .map_err(to_py_err)
}

#[pyfunction]
#[pyo3(signature = (target, repo_path=None))]
fn checkout(py: Python<'_>, target: String, repo_path: Option<String>) -> PyResult<String> {
    let repo = PathBuf::from(repo_path.unwrap_or_else(|| ".".to_string()));
    let target_clone = target.clone();
    py.allow_threads(move || shard_core::checkout(&repo, &target_clone, true))
        .map_err(to_py_err)?;
    Ok(format!("checked out {}", target))
}

#[pyfunction]
#[pyo3(signature = (commit_id, repo_path=None, allow_unsigned=false))]
fn verify(
    py: Python<'_>,
    commit_id: String,
    repo_path: Option<String>,
    allow_unsigned: bool,
) -> PyResult<String> {
    let repo = PathBuf::from(repo_path.unwrap_or_else(|| ".".to_string()));
    py.allow_threads(move || {
        shard_core::verify_with_options(&repo, &commit_id, true, allow_unsigned)
    })
    .map_err(to_py_err)?;
    Ok("verified".to_string())
}

fn now_unix_secs() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

#[pymodule]
fn shard(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(init, m)?)?;
    m.add_function(wrap_pyfunction!(add, m)?)?;
    m.add_function(wrap_pyfunction!(commit, m)?)?;
    m.add_function(wrap_pyfunction!(log, m)?)?;
    m.add_function(wrap_pyfunction!(status, m)?)?;
    m.add_function(wrap_pyfunction!(checkout, m)?)?;
    m.add_function(wrap_pyfunction!(verify, m)?)?;
    m.add_class::<CommitResult>()?;
    Ok(())
}
