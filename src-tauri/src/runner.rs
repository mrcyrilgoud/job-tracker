use fs2::FileExt;
use rusqlite::Connection;
use std::fs::OpenOptions;
use tauri::AppHandle;

use crate::db::configure_app_connection;
use crate::db::paths::DataPaths;
use crate::error::{AppError, AppResult};
use crate::runs::coordinator::{RunRequest, SystemClock};
use crate::runs::legacy::execute_legacy_with;
use crate::runs::model::Trigger;
use crate::runs::progress::{LogSink, TauriSink};
use crate::runs::RunRegistry;

pub(crate) fn open_runner_conn(paths: &DataPaths) -> AppResult<Connection> {
    paths.ensure_dirs()?;
    let conn = Connection::open(&paths.db_path)?;
    configure_app_connection(&conn)?;
    Ok(conn)
}

/// Open a short-lived-stage status connection without running migrations.
/// Polling must never perform schema work; startup owns migrations.
pub(crate) fn open_runner_status_conn(paths: &DataPaths) -> AppResult<Connection> {
    paths.ensure_dirs()?;
    let conn = Connection::open(&paths.db_path)?;
    conn.pragma_update(None, "busy_timeout", 5000i32)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(conn)
}

pub fn try_lock_runner(paths: &DataPaths) -> AppResult<std::fs::File> {
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&paths.runner_lock_path)?;
    if lock_file.try_lock_exclusive().is_err() {
        return Err(AppError::from("operation_in_progress:runner"));
    }
    Ok(lock_file)
}

/// Batch HTTP posting checks for every job, with exclusive runner flock.
/// Coordinator-backed entry point shared by GUI compatibility calls and the
/// headless LaunchAgent/CLI worker. The old function name remains unchanged.
pub async fn run_jobs_cycle_trigger(
    paths: &DataPaths,
    app: Option<AppHandle>,
    trigger: Trigger,
) -> AppResult<serde_json::Value> {
    let request = RunRequest::JobsCycle { trigger };
    let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    match app {
        Some(app) => {
            execute_legacy_with(
                paths.clone(),
                lock,
                std::sync::Arc::new(crate::jobs::posting_check::fetch::HttpPostingFetcher),
                std::sync::Arc::new(TauriSink::new(app)),
                SystemClock,
                RunRegistry::new(),
                request,
                None,
            )
            .await
        }
        None => {
            execute_legacy_with(
                paths.clone(),
                lock,
                std::sync::Arc::new(crate::jobs::posting_check::fetch::HttpPostingFetcher),
                std::sync::Arc::new(LogSink),
                SystemClock,
                RunRegistry::new(),
                request,
                None,
            )
            .await
        }
    }
}

/// Headless CLI entry for LaunchAgent: `job-tracker --run-jobs`
pub async fn run_jobs_cli(data_dir: Option<std::path::PathBuf>) -> AppResult<()> {
    let paths = if let Some(dir) = data_dir {
        DataPaths::from_data_dir(dir)
    } else {
        crate::db::paths::resolve_data_dir(None)
    };
    let result = run_jobs_cycle_trigger(&paths, None, Trigger::Launchd).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );
    Ok(())
}
