use pyo3::prelude::*;
use std::process::Command;

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

#[pyfunction]
#[pyo3(signature = (repo_path=None, private=false, db=None))]
fn init(repo_path: Option<String>, private: bool, db: Option<String>) -> PyResult<String> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let mut cmd = Command::new("shard");
    cmd.arg("init");
    if private {
        cmd.arg("--private");
    }
    if let Some(db_str) = db {
        cmd.arg("--db").arg(db_str);
    }
    cmd.current_dir(&path);

    let output = cmd
        .output()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(py_err_from_status(&output))
    }
}

#[pyfunction]
#[pyo3(signature = (file_path, repo_path=None))]
fn add(file_path: String, repo_path: Option<String>) -> PyResult<String> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let mut cmd = Command::new("shard");
    cmd.arg("add").arg(&file_path);
    cmd.current_dir(&path);

    let output = cmd
        .output()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(py_err_from_status(&output))
    }
}

#[pyfunction]
#[pyo3(signature = (message, repo_path=None, author=None))]
fn commit(
    message: String,
    repo_path: Option<String>,
    author: Option<String>,
) -> PyResult<CommitResult> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let mut cmd = Command::new("shard");
    cmd.arg("commit").arg("-m").arg(&message);
    if let Some(ref a) = author {
        cmd.arg("--author").arg(a);
    }
    cmd.current_dir(&path);

    let output = cmd
        .output()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if !output.status.success() {
        return Err(py_err_from_status(&output));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let commit_id = stdout
        .split_whitespace()
        .nth(1)
        .unwrap_or("unknown")
        .to_string();

    Ok(CommitResult {
        commit_id,
        message,
        author: author.unwrap_or_else(|| "User <user@example.com>".to_string()),
        timestamp: chrono_now(),
    })
}

#[pyfunction]
#[pyo3(signature = (repo_path=None, limit=None))]
fn log(repo_path: Option<String>, limit: Option<usize>) -> PyResult<Vec<String>> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let mut cmd = Command::new("shard");
    cmd.arg("log");
    if let Some(n) = limit {
        cmd.arg("--limit").arg(n.to_string());
    }
    cmd.current_dir(&path);

    let output = cmd
        .output()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        let out = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(out.lines().map(|s| s.to_string()).collect())
    } else {
        Err(py_err_from_status(&output))
    }
}

#[pyfunction]
#[pyo3(signature = (repo_path=None))]
fn status(repo_path: Option<String>) -> PyResult<String> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let mut cmd = Command::new("shard");
    cmd.arg("status").current_dir(&path);

    let output = cmd
        .output()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(py_err_from_status(&output))
    }
}

#[pyfunction]
#[pyo3(signature = (target, repo_path=None))]
fn checkout(target: String, repo_path: Option<String>) -> PyResult<String> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let mut cmd = Command::new("shard");
    cmd.arg("checkout").arg(&target).current_dir(&path);

    let output = cmd
        .output()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(py_err_from_status(&output))
    }
}

#[pyfunction]
#[pyo3(signature = (commit_id, repo_path=None))]
fn verify(commit_id: String, repo_path: Option<String>) -> PyResult<String> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let mut cmd = Command::new("shard");
    cmd.arg("verify").arg(&commit_id).current_dir(&path);

    let output = cmd
        .output()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(py_err_from_status(&output))
    }
}

fn py_err_from_status(output: &std::process::Output) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(String::from_utf8_lossy(&output.stderr).to_string())
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("{}", now.as_secs())
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
