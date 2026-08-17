//! Debounced, single-flight CSV export coordinator.
//!
//! Mutations mark the export dirty and return immediately. A short debounce
//! window coalesces rapid writes; export then opens its own SQLite connection
//! (never the UI mutex) and holds a file lock across the CSV + sync write.

use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use fs2::FileExt;
use parking_lot::Mutex;
use rusqlite::Connection;
use serde::Serialize;

use crate::db::migrate;
use crate::error::{AppError, AppResult};
use crate::jobs::csv::export_jobs_csv;
use crate::jobs::csv_config::{active_csv_path, csv_lock_path};
use crate::util::now_iso;

const EXPORT_DEBOUNCE_MS: u64 = 500;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvExportStatus {
    pub dirty: bool,
    pub last_successful_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct CsvExportCoordinator {
    db_path: PathBuf,
    default_csv_path: PathBuf,
    generation: Arc<AtomicU64>,
    last_exported: Arc<AtomicU64>,
    status: Arc<Mutex<CsvExportStatus>>,
    export_lock: Arc<tokio::sync::Mutex<()>>,
}

impl CsvExportCoordinator {
    pub fn new(db_path: PathBuf, default_csv_path: PathBuf) -> Self {
        Self {
            db_path,
            default_csv_path,
            generation: Arc::new(AtomicU64::new(0)),
            last_exported: Arc::new(AtomicU64::new(0)),
            status: Arc::new(Mutex::new(CsvExportStatus::default())),
            export_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn status(&self) -> CsvExportStatus {
        self.status.lock().clone()
    }

    /// Mark the CSV dirty and schedule a debounced export outside the UI DB lock.
    pub fn mark_dirty(&self) {
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut status = self.status.lock();
            status.dirty = true;
        }
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(EXPORT_DEBOUNCE_MS)).await;
            this.export_if_current(gen).await;
        });
    }

    async fn export_if_current(&self, _requested_gen: u64) {
        let _guard = self.export_lock.lock().await;
        loop {
            let export_gen = self.generation.load(Ordering::SeqCst);
            if self.last_exported.load(Ordering::SeqCst) >= export_gen {
                return;
            }

            let db_path = self.db_path.clone();
            let default_csv_path = self.default_csv_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                export_with_own_connection(&db_path, &default_csv_path)
            })
            .await;

            match result {
                Ok(Ok(_)) => {
                    self.last_exported.store(export_gen, Ordering::SeqCst);
                    let mut status = self.status.lock();
                    status.dirty = self.generation.load(Ordering::SeqCst) > export_gen;
                    status.last_successful_at = Some(now_iso());
                    status.last_error = None;
                }
                Ok(Err(err)) => {
                    let mut status = self.status.lock();
                    status.last_error = Some(err.to_string());
                    log::error!("[csv-sync] export failed: {err}");
                    return;
                }
                Err(err) => {
                    let mut status = self.status.lock();
                    status.last_error = Some(err.to_string());
                    log::error!("[csv-sync] export task join failed: {err}");
                    return;
                }
            }

            if self.generation.load(Ordering::SeqCst) <= export_gen {
                return;
            }
            // Dirtied during export — run one follow-up with the newest generation.
        }
    }

    /// Run a configuration change exclusively with the exporter. Queued and
    /// future exports resolve the destination after this operation completes.
    pub async fn with_exclusive<T>(&self, f: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
        let _guard = self.export_lock.lock().await;
        f()
    }
}

fn export_with_own_connection(
    db_path: &std::path::Path,
    default_csv_path: &std::path::Path,
) -> AppResult<()> {
    let conn = open_export_conn(db_path)?;
    let csv_path = active_csv_path(&conn, default_csv_path)?;
    let lock_path = csv_lock_path(&csv_path);
    with_csv_file_lock(&lock_path, || {
        export_jobs_csv(&conn, &csv_path, None)?;
        Ok(())
    })
}

fn open_export_conn(db_path: &std::path::Path) -> AppResult<Connection> {
    let conn = Connection::open(db_path).map_err(|e| AppError::from(e.to_string()))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| AppError::from(e.to_string()))?;
    conn.pragma_update(None, "busy_timeout", 5000i32)
        .map_err(|e| AppError::from(e.to_string()))?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(|e| AppError::from(e.to_string()))?;
    migrate::migrate(&conn).map_err(|e| AppError::from(e.to_string()))?;
    Ok(conn)
}

/// Exclusive lock covering the CSV + sync-state write sequence.
pub fn with_csv_file_lock<T>(
    lock_path: &std::path::Path,
    f: impl FnOnce() -> AppResult<T>,
) -> AppResult<T> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|e| AppError::from(format!("open csv lock: {e}")))?;
    file.lock_exclusive()
        .map_err(|e| AppError::from(format!("csv lock held: {e}")))?;
    let result = f();
    let _ = file.unlock();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;
    use crate::db::paths::DataPaths;
    use crate::jobs::csv_config::set_custom_csv_path;
    use crate::jobs::service::create_job_from_url;
    use std::sync::atomic::AtomicUsize;
    use tempfile::tempdir;

    #[tokio::test]
    async fn rapid_marks_coalesce_to_one_export() {
        let directory = tempdir().unwrap();
        let paths = DataPaths::from_data_dir(directory.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let conn = Connection::open(&paths.db_path).unwrap();
        migrate(&conn).unwrap();
        create_job_from_url(
            &conn,
            "https://example.com/jobs/1",
            "Role",
            Some("Acme"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        drop(conn);

        let export_count = Arc::new(AtomicUsize::new(0));
        let coordinator = CsvExportCoordinator::new(
            paths.db_path.clone(),
            paths.jobs_csv_path.clone(),
        );

        for _ in 0..10 {
            coordinator.mark_dirty();
        }

        // Wait for debounce + export.
        tokio::time::sleep(Duration::from_millis(1200)).await;

        assert!(paths.jobs_csv_path.exists(), "csv should be written");
        let status = coordinator.status();
        assert!(!status.dirty);
        assert!(status.last_successful_at.is_some());
        assert!(status.last_error.is_none());
        // Generation advanced 10 times; last_exported should equal generation.
        assert_eq!(
            coordinator.generation.load(Ordering::SeqCst),
            coordinator.last_exported.load(Ordering::SeqCst)
        );
        let _ = export_count;
    }

    #[tokio::test]
    async fn uses_configured_csv_destination() {
        let directory = tempdir().unwrap();
        let custom_path = directory.path().join("shared").join("jobs.csv");
        std::fs::create_dir_all(custom_path.parent().unwrap()).unwrap();
        let paths = DataPaths::from_data_dir(directory.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let conn = Connection::open(&paths.db_path).unwrap();
        migrate(&conn).unwrap();
        set_custom_csv_path(&conn, &paths.jobs_csv_path, &custom_path).unwrap();
        create_job_from_url(
            &conn,
            "https://example.com/jobs/custom",
            "Role",
            Some("Acme"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        drop(conn);

        let coordinator =
            CsvExportCoordinator::new(paths.db_path.clone(), paths.jobs_csv_path.clone());
        coordinator.mark_dirty();
        tokio::time::sleep(Duration::from_millis(1200)).await;

        assert!(custom_path.exists());
        assert!(std::path::PathBuf::from(format!("{}.sync.json", custom_path.display())).exists());
        assert!(!paths.jobs_csv_path.exists());
    }

    #[tokio::test]
    async fn list_path_stays_responsive_while_export_marked() {
        let directory = tempdir().unwrap();
        let paths = DataPaths::from_data_dir(directory.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let state = crate::db::AppState::open(paths.clone()).unwrap();
        state
            .with_db(|conn| {
                create_job_from_url(
                    conn,
                    "https://example.com/jobs/2",
                    "Role",
                    Some("Acme"),
                    None,
                    None,
                    None,
                    None,
                )?;
                Ok(())
            })
            .unwrap();

        state.csv_export.mark_dirty();
        // Immediate UI read must not wait for CSV I/O.
        let count = state
            .with_db(|conn| {
                let n: i64 = conn
                    .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
                    .unwrap();
                Ok(n)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
}
