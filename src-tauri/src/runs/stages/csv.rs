//! CSV mirror synchronization stage (moved from `runner.rs`, unchanged semantics).
//!
//! Stage 4 of a Jobs_Cycle (design.md, Component 6). A single unit: resolve the
//! active CSV path from the coordinator-owned connection, then run
//! `sync_jobs_csv_with_disk` under the CSV file lock, exactly as the original
//! runner did.
//!
//! ## Stage-level result
//! This stage has one item. Any error from resolving the path, taking the file
//! lock, or `sync_jobs_csv_with_disk` is an apply/DB failure and surfaces as a
//! stage-level `Err` (Req 2.2). There is no per-item fetch to keep item-level.
//!
//! ## Scope boundary (task 8.1 vs 8.2)
//! Exposes [`run_csv_stage`]. It calls a supplied progress callback with a
//! [`StageProgress`] at start (`InProgress`, 0/1) and at completion
//! (`Succeeded`, 1/1). The coordinator (task 8.2) owns sequencing, event
//! publishing, and the CSV mark-dirty rules for canceled/posting-check runs.

use std::path::Path;

use rusqlite::Connection;

use crate::error::AppResult;
use crate::jobs::csv::sync_jobs_csv_with_disk;
use crate::jobs::csv::{ExportResult, ImportResult};
use crate::jobs::csv_config::{active_csv_path, csv_lock_path};
use crate::jobs::csv_export::with_csv_file_lock;
use crate::runs::model::{StageName, StageOutcome};
use crate::runs::progress::{BoundedCount, StageProgress};

/// Outcome of the CSV stage.
#[derive(Debug)]
pub struct CsvStageResult {
    /// Import count for the legacy JSON projection (`csv.imported`), from the
    /// `Option<ImportResult>` returned by `sync_jobs_csv_with_disk`.
    pub imported: Option<ImportResult>,
    /// Export result for the legacy JSON projection (`csv.exported`).
    pub exported: ExportResult,
    /// `Succeeded`; a failure short-circuits with a stage-level `Err`.
    pub outcome: StageOutcome,
    /// Final `StageProgress` (also delivered through the progress callback).
    pub progress: StageProgress,
}

/// Run the CSV mirror sync against the caller's connection.
///
/// `conn` resolves the active CSV path (custom or the `default_csv_path`).
/// `default_csv_path` is the runner's `DataPaths::jobs_csv_path`, and `db_path`
/// is `DataPaths::db_path`, matching the original `run_jobs_cycle`.
///
/// `progress` is called with `InProgress` (0/1) at start and `Succeeded` (1/1)
/// at completion. A resolve/lock/sync failure returns a stage-level `Err`.
pub async fn run_csv_stage(
    conn: &mut Connection,
    db_path: &Path,
    default_csv_path: &Path,
    mut progress: impl FnMut(StageProgress),
) -> AppResult<CsvStageResult> {
    progress(stage_progress(StageOutcome::InProgress, 0));

    let csv_path = active_csv_path(conn, default_csv_path)?;
    let (imported, exported) = with_csv_file_lock(&csv_lock_path(&csv_path), || {
        sync_jobs_csv_with_disk(db_path, &csv_path)
    })?;

    let progress_final = stage_progress(StageOutcome::Succeeded, 1);
    progress(progress_final.clone());
    Ok(CsvStageResult {
        imported,
        exported,
        outcome: StageOutcome::Succeeded,
        progress: progress_final,
    })
}

fn stage_progress(outcome: StageOutcome, current: usize) -> StageProgress {
    StageProgress {
        name: StageName::Csv,
        outcome,
        current: BoundedCount::from_usize(current),
        total: BoundedCount::from_usize(1),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;
    use tempfile::tempdir;

    #[tokio::test]
    async fn exports_csv_and_reports_succeeded() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("job-tracker.db");
        let csv_path = dir.path().join("jobs.csv");
        {
            let conn = Connection::open(&db_path).unwrap();
            migrate(&conn).unwrap();
        }
        let mut conn = Connection::open(&db_path).unwrap();

        let mut snapshots = Vec::new();
        let result = run_csv_stage(&mut conn, &db_path, &csv_path, |p| snapshots.push(p))
            .await
            .unwrap();

        assert_eq!(result.outcome, StageOutcome::Succeeded);
        // A fresh DB has no CSV on disk yet, so the mirror is exported, not imported.
        assert!(result.imported.is_none());
        assert!(csv_path.exists(), "CSV mirror should be written");
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].outcome, StageOutcome::InProgress);
        assert_eq!(snapshots[1].outcome, StageOutcome::Succeeded);
        assert_eq!(result.progress.current, BoundedCount::from_usize(1));
        assert_eq!(result.progress.total, BoundedCount::from_usize(1));
    }
}
