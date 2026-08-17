use std::fs::OpenOptions;
use std::io::Write;
use std::time::Duration;

use fs2::FileExt;
use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::ats::careers::{apply_careers_check, fetch_careers_hash};
use crate::ats::sync::{apply_watch_sync, fetch_remote_jobs};
use crate::db::migrate;
use crate::db::paths::DataPaths;
use crate::error::{AppError, AppResult};
use crate::gmail::{is_gmail_connected, poll_gmail_matches};
use crate::jobs::check_active::{apply_posting_check, fetch_posting_state};
use crate::jobs::csv::sync_jobs_csv_with_disk;
use crate::jobs::csv_config::{active_csv_path, csv_lock_path};
use crate::jobs::csv_export::with_csv_file_lock;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerProgress {
    pub stage: String,
    pub message: String,
    pub current: usize,
    pub total: usize,
    pub done: bool,
}

fn emit_progress(app: Option<&AppHandle>, progress: RunnerProgress) {
    if let Some(app) = app {
        let _ = app.emit("jobs-runner-progress", &progress);
    }
    log::info!(
        "[jobs-runner] {} ({}/{}) {}",
        progress.stage,
        progress.current,
        progress.total,
        progress.message
    );
}

fn open_runner_conn(paths: &DataPaths) -> AppResult<Connection> {
    paths.ensure_dirs()?;
    let conn = Connection::open(&paths.db_path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000i32)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate::migrate(&conn)?;
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

/// Shared posting-check loop (no flock). Callers that already hold the runner
/// lock use this; `check_all_postings` acquires the lock then delegates here.
///
/// Takes `&mut Connection` (Send) rather than `&Connection` (!Send) so the
/// future stays Send across HTTP awaits.
pub async fn check_all_postings_with_conn(
    conn: &mut Connection,
    app: Option<&AppHandle>,
) -> AppResult<Vec<serde_json::Value>> {
    let jobs: Vec<(String, String, String)> = {
        let mut stmt = conn.prepare("SELECT id, url, posting_state FROM jobs")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let job_ids: Vec<String> = jobs.iter().map(|(id, _, _)| id.clone()).collect();
    emit_progress(
        app,
        RunnerProgress {
            stage: "postings".into(),
            message: "Checking job postings".into(),
            current: 0,
            total: job_ids.len(),
            done: false,
        },
    );
    let semaphore = std::sync::Arc::new(Semaphore::new(4));
    let mut fetches = JoinSet::new();
    for (job_id, url, previous) in jobs {
        let semaphore = semaphore.clone();
        fetches.spawn(async move {
            let _permit = semaphore.acquire_owned().await.expect("posting semaphore");
            let fetched =
                tokio::time::timeout(Duration::from_secs(30), fetch_posting_state(&url)).await;
            let (state, result) = fetched.unwrap_or_else(|_| {
                (
                    "unknown".to_string(),
                    "error: request timed out after 30s".to_string(),
                )
            });
            (job_id, previous, state, result)
        });
    }
    let mut posting_results = Vec::new();
    let mut completed = 0;
    while let Some(result) = fetches.join_next().await {
        let (job_id, previous, state, last_check_result) =
            result.map_err(|e| AppError::from(e.to_string()))?;
        let applied = apply_posting_check(conn, &job_id, &previous, &state, &last_check_result)?;
        posting_results.push(serde_json::json!({ "jobId": job_id, "result": applied }));
        completed += 1;
        emit_progress(
            app,
            RunnerProgress {
                stage: "postings".into(),
                message: format!("Checked {job_id}"),
                current: completed,
                total: job_ids.len(),
                done: false,
            },
        );
    }
    emit_progress(
        app,
        RunnerProgress {
            stage: "postings".into(),
            message: "Posting checks complete".into(),
            current: job_ids.len(),
            total: job_ids.len(),
            done: true,
        },
    );
    Ok(posting_results)
}

/// Batch HTTP posting checks for every job, with exclusive runner flock.
pub async fn check_all_postings(
    paths: &DataPaths,
    app: Option<AppHandle>,
) -> AppResult<serde_json::Value> {
    let lock_file = try_lock_runner(paths)?;
    let mut conn = open_runner_conn(paths)?;
    let posting_results = check_all_postings_with_conn(&mut conn, app.as_ref()).await?;
    let _ = lock_file.unlock();
    Ok(serde_json::json!({
        "postings": posting_results.len(),
    }))
}

/// Single-instance flock around the full jobs cycle.
pub async fn run_jobs_cycle(
    paths: &DataPaths,
    app: Option<AppHandle>,
) -> AppResult<serde_json::Value> {
    let lock_file = try_lock_runner(paths)?;

    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.worker_log_path)?;
    writeln!(log, "--- jobs cycle start {} ---", crate::util::now_iso())?;

    let mut conn = open_runner_conn(paths)?;

    // 1) Check postings (shared helper; cycle already holds the flock)
    let posting_results = check_all_postings_with_conn(&mut conn, app.as_ref()).await?;

    // 2) Sync watches
    let watch_ids: Vec<(String, String, String)> = {
        let mut stmt = conn.prepare("SELECT id, provider, board_slug FROM company_watches")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    emit_progress(
        app.as_ref(),
        RunnerProgress {
            stage: "watches".into(),
            message: "Syncing ATS watches".into(),
            current: 0,
            total: watch_ids.len(),
            done: false,
        },
    );
    let semaphore = std::sync::Arc::new(Semaphore::new(2));
    let mut fetches = JoinSet::new();
    for (watch_id, provider, board_slug) in watch_ids.clone() {
        let semaphore = semaphore.clone();
        fetches.spawn(async move {
            let _permit = semaphore.acquire_owned().await.expect("watch semaphore");
            let remote = tokio::time::timeout(
                Duration::from_secs(30),
                fetch_remote_jobs(&provider, &board_slug),
            )
            .await
            .map_err(|_| "request timed out after 30s".to_string())
            .and_then(|result| result.map_err(|e| e.to_string()));
            (watch_id, remote)
        });
    }
    let mut watch_results = Vec::new();
    let mut completed = 0;
    while let Some(result) = fetches.join_next().await {
        let (watch_id, remote) = result.map_err(|e| AppError::from(e.to_string()))?;
        let result = apply_watch_sync(&conn, &watch_id, remote)?;
        watch_results.push(serde_json::json!({ "watchId": watch_id, "result": result }));
        completed += 1;
        emit_progress(
            app.as_ref(),
            RunnerProgress {
                stage: "watches".into(),
                message: format!("Synced {watch_id}"),
                current: completed,
                total: watch_ids.len(),
                done: false,
            },
        );
    }
    emit_progress(
        app.as_ref(),
        RunnerProgress {
            stage: "watches".into(),
            message: "ATS watch sync complete".into(),
            current: watch_ids.len(),
            total: watch_ids.len(),
            done: true,
        },
    );

    // 3) Careers pages
    let companies: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, name, careers_url FROM companies WHERE careers_url IS NOT NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    emit_progress(
        app.as_ref(),
        RunnerProgress {
            stage: "careers".into(),
            message: "Checking careers pages".into(),
            current: 0,
            total: companies.len(),
            done: false,
        },
    );
    let semaphore = std::sync::Arc::new(Semaphore::new(4));
    let mut fetches = JoinSet::new();
    for (company_id, name, url) in companies.clone() {
        let semaphore = semaphore.clone();
        fetches.spawn(async move {
            let _permit = semaphore.acquire_owned().await.expect("careers semaphore");
            let fetched = tokio::time::timeout(Duration::from_secs(30), fetch_careers_hash(&url))
                .await
                .map_err(|_| AppError::from("request timed out after 30s"))
                .and_then(|result| result);
            (company_id, name, fetched)
        });
    }
    let mut careers_results = Vec::new();
    let mut completed = 0;
    while let Some(result) = fetches.join_next().await {
        let (company_id, name, fetched) = result.map_err(|e| AppError::from(e.to_string()))?;
        match fetched {
            Ok((hash, text)) => {
                let result = apply_careers_check(&conn, &company_id, &name, &hash, &text)?;
                careers_results
                    .push(serde_json::json!({ "companyId": company_id, "result": result }));
            }
            Err(e) => {
                careers_results.push(serde_json::json!({
                    "companyId": company_id,
                    "result": { "changed": false, "reason": e.to_string() }
                }));
            }
        }
        completed += 1;
        emit_progress(
            app.as_ref(),
            RunnerProgress {
                stage: "careers".into(),
                message: format!("Checked {name}"),
                current: completed,
                total: companies.len(),
                done: false,
            },
        );
    }
    emit_progress(
        app.as_ref(),
        RunnerProgress {
            stage: "careers".into(),
            message: "Careers checks complete".into(),
            current: companies.len(),
            total: companies.len(),
            done: true,
        },
    );

    // 4) Gmail
    let gmail = if is_gmail_connected().unwrap_or(false) {
        emit_progress(
            app.as_ref(),
            RunnerProgress {
                stage: "gmail".into(),
                message: "Polling Gmail".into(),
                current: 0,
                total: 1,
                done: false,
            },
        );
        // Dedicated connection avoids holding shared UI mutex; this conn is runner-owned.
        match poll_gmail_matches(&paths.db_path, &paths.gmail_poll_lock_path).await {
            Ok(v) => v,
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        }
    } else {
        serde_json::json!({ "skipped": true })
    };

    // 5) CSV sync
    emit_progress(
        app.as_ref(),
        RunnerProgress {
            stage: "csv".into(),
            message: "Syncing jobs.csv".into(),
            current: 0,
            total: 1,
            done: false,
        },
    );
    let csv_path = active_csv_path(&conn, &paths.jobs_csv_path)?;
    let csv = with_csv_file_lock(&csv_lock_path(&csv_path), || {
        sync_jobs_csv_with_disk(&paths.db_path, &csv_path)
    })?;

    let summary = serde_json::json!({
        "postings": posting_results.len(),
        "watches": watch_results,
        "careers": careers_results,
        "gmail": gmail,
        "csv": {
            "imported": csv.0,
            "exported": csv.1
        }
    });

    emit_progress(
        app.as_ref(),
        RunnerProgress {
            stage: "cycle".into(),
            message: "Jobs cycle complete".into(),
            current: 1,
            total: 1,
            done: true,
        },
    );
    writeln!(log, "--- jobs cycle end {} ---", crate::util::now_iso())?;
    let _ = lock_file.unlock();
    Ok(summary)
}

/// Headless CLI entry for LaunchAgent: `job-tracker --run-jobs`
pub async fn run_jobs_cli(data_dir: Option<std::path::PathBuf>) -> AppResult<()> {
    let paths = if let Some(dir) = data_dir {
        DataPaths::from_data_dir(dir)
    } else {
        crate::db::paths::resolve_data_dir(None)
    };
    let result = run_jobs_cycle(&paths, None).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );
    Ok(())
}
