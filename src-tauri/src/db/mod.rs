use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use parking_lot::Mutex;
use rusqlite::Connection;

use crate::error::{AppError, AppResult};

pub mod migrate;
pub mod paths;

pub use paths::{resolve_data_dir, DataPaths};

/// Shared app state. Never hold the mutex across `.await`.
#[derive(Clone)]
pub struct AppState {
    pub paths: DataPaths,
    pub db: Arc<Mutex<Connection>>,
    pub runner_lock: Arc<Mutex<()>>,
}

impl AppState {
    pub fn open(paths: DataPaths) -> AppResult<Self> {
        paths.ensure_dirs()?;
        let conn = Connection::open(&paths.db_path)
            .with_context(|| format!("open db {}", paths.db_path.display()))
            .map_err(AppError::from)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000i32)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        migrate::migrate(&conn)?;
        Ok(Self {
            paths,
            db: Arc::new(Mutex::new(conn)),
            runner_lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn with_db<T>(&self, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
        let guard = self.db.lock();
        f(&guard)
    }
}

/// WAL-safe copy of a SQLite database (+ wal/shm) into `dest_dir`.
pub fn migrate_legacy_data(legacy_dir: &Path, dest: &DataPaths) -> AppResult<bool> {
    let legacy_db = legacy_dir.join("job-tracker.db");
    if !legacy_db.exists() {
        return Ok(false);
    }
    if dest.db_path.exists() {
        return Ok(false);
    }

    dest.ensure_dirs()?;

    // Checkpoint if we can open the legacy DB exclusively enough.
    if let Ok(conn) = Connection::open(&legacy_db) {
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        drop(conn);
    }

    copy_if_exists(&legacy_db, &dest.db_path)?;
    copy_if_exists(
        &legacy_dir.join("job-tracker.db-wal"),
        &PathBuf::from(format!("{}-wal", dest.db_path.display())),
    )?;
    copy_if_exists(
        &legacy_dir.join("job-tracker.db-shm"),
        &PathBuf::from(format!("{}-shm", dest.db_path.display())),
    )?;

    let legacy_docs = legacy_dir.join("documents");
    if legacy_docs.is_dir() {
        copy_dir_recursive(&legacy_docs, &dest.documents_dir)?;
    }

    for name in ["jobs.csv", "jobs.csv.sync.json"] {
        let src = legacy_dir.join(name);
        let dst = dest.data_dir.join(name);
        copy_if_exists(&src, &dst)?;
    }

    Ok(true)
}

fn copy_if_exists(src: &Path, dst: &Path) -> AppResult<()> {
    if src.exists() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)
            .with_context(|| format!("copy {} -> {}", src.display(), dst.display()))
            .map_err(AppError::from)?;
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> AppResult<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
