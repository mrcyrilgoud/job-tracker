use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub struct DataPaths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub documents_dir: PathBuf,
    pub jobs_csv_path: PathBuf,
    pub jobs_csv_sync_path: PathBuf,
    pub worker_log_path: PathBuf,
    pub runner_lock_path: PathBuf,
}

impl DataPaths {
    pub fn from_data_dir(data_dir: PathBuf) -> Self {
        let db_path = data_dir.join("job-tracker.db");
        let documents_dir = data_dir.join("documents");
        let jobs_csv_path = data_dir.join("jobs.csv");
        let jobs_csv_sync_path = data_dir.join("jobs.csv.sync.json");
        let worker_log_path = data_dir.join("jobs-worker.log");
        let runner_lock_path = data_dir.join("jobs-runner.lock");
        Self {
            data_dir,
            db_path,
            documents_dir,
            jobs_csv_path,
            jobs_csv_sync_path,
            worker_log_path,
            runner_lock_path,
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.data_dir)
            .with_context(|| format!("create {}", self.data_dir.display()))?;
        fs::create_dir_all(&self.documents_dir)
            .with_context(|| format!("create {}", self.documents_dir.display()))?;
        Ok(())
    }
}

/// Resolve data directory.
/// - `JOB_TRACKER_DATA_DIR` always wins
/// - In debug/dev: repo `data/` (shared with Next during migration)
/// - In release: `app_data_dir` (Application Support)
pub fn resolve_data_dir(app_data_dir: Option<PathBuf>) -> DataPaths {
    if let Ok(dir) = env::var("JOB_TRACKER_DATA_DIR") {
        return DataPaths::from_data_dir(PathBuf::from(dir));
    }

    if cfg!(debug_assertions) {
        if let Some(repo_data) = find_repo_data_dir() {
            return DataPaths::from_data_dir(repo_data);
        }
    }

    let dir = app_data_dir.unwrap_or_else(|| {
        dirs_fallback()
    });
    DataPaths::from_data_dir(dir)
}

fn find_repo_data_dir() -> Option<PathBuf> {
    let cwd = env::current_dir().ok()?;
    for candidate in [
        cwd.join("data"),
        cwd.join("../data"),
        cwd.join("../../data"),
    ] {
        if let Ok(canonical) = candidate.canonicalize() {
            return Some(canonical);
        }
        // Prefer creating beside Cargo.toml / package.json when present
        if cwd.join("package.json").exists() || cwd.join("src-tauri").exists() {
            return Some(cwd.join("data"));
        }
        if cwd.join("../package.json").exists() {
            return Some(cwd.join("../data"));
        }
    }
    Some(cwd.join("data"))
}

fn dirs_fallback() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/Application Support/com.jobtracker.local")
}
