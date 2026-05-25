use anyhow::Result;
use pyo3::prelude::*;
use std::path::PathBuf;
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

    let output = cmd.output().map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(py_err_from_status(&output))
    }
}

#[pyfunction]
fn add(repo_path: Option<String>, file_path: String) -> PyResult<String> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let mut cmd = Command::new("shard");
    cmd.arg("add").arg(&file_path);
    cmd.current_dir(&path);

    let output = cmd.output().map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(py_err_from_status(&output))
    }
}

#[pyfunction]
fn commit(repo_path: Option<String>, message: String, author: Option<String>) -> PyResult<CommitResult> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let mut cmd = Command::new("shard");
    cmd.arg("commit").arg("-m").arg(&message);
    if let Some(a) = author {
        cmd.arg("--author").arg(a);
    }
    cmd.current_dir(&path);

    let output = cmd.output().map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if !output.status.success() {
        return Err(py_err_from_status(&output));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let commit_id = stdout.split_whitespace().nth(1).unwrap_or("unknown").to_string();

    Ok(CommitResult {
        commit_id,
        message,
        author: author.unwrap_or_else(|| "User <user@example.com>".to_string()),
        timestamp: chrono_now(),
    })
}

#[pyfunction]
fn log(repo_path: Option<String>, limit: Option<usize>) -> PyResult<Vec<String>> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let mut cmd = Command::new("shard");
    cmd.arg("log");
    if let Some(n) = limit {
        cmd.arg("--limit").arg(n.to_string());
    }
    cmd.current_dir(&path);

    let output = cmd.output().map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        let out = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(out.lines().map(|s| s.to_string()).collect())
    } else {
        Err(py_err_from_status(&output))
    }
}

#[pyfunction]
fn status(repo_path: Option<String>) -> PyResult<String> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let cmd = Command::new("shard").arg("status").current_dir(&path);

    let output = cmd.output().map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(py_err_from_status(&output))
    }
}

#[pyfunction]
fn checkout(repo_path: Option<String>, target: String) -> PyResult<String> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let cmd = Command::new("shard").arg("checkout").arg(&target).current_dir(&path);

    let output = cmd.output().map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(py_err_from_status(&output))
    }
}

#[pyfunction]
fn verify(repo_path: Option<String>, commit_id: String) -> PyResult<String> {
    let path = repo_path.unwrap_or_else(|| ".".to_string());
    let cmd = Command::new("shard").arg("verify").arg(&commit_id).current_dir(&path);

    let output = cmd.output().map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(py_err_from_status(&output))
    }
}

fn py_err_from_status(output: &std::process::Output) -> pyo3::exceptions::PyRuntimeError {
    pyo3::exceptions::PyRuntimeError::new_err(String::from_utf8_lossy(&output.stderr).to_string())
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    format!("{}", now.as_secs())
}

#[pymodule]
fn shard(_py: Python, m: &PyModule) -> PyResult<()> {
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