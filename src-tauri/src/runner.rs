use std::fs::OpenOptions;
use std::io::Write;

use fs2::FileExt;
use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::ats::careers::{apply_careers_check, fetch_careers_hash};
use crate::ats::sync::{apply_watch_sync, fetch_remote_jobs};
use crate::db::migrate;
use crate::db::paths::DataPaths;
use crate::error::{AppError, AppResult};
use crate::gmail::{is_gmail_connected, poll_gmail_matches};
use crate::jobs::check_active::{apply_posting_check, fetch_posting_state, load_job_check_context};
use crate::jobs::csv::sync_jobs_csv_with_disk;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerProgress {
    pub stage: String,
    pub message: String,
    pub current: usize,
    pub total: usize,
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

/// Single-instance flock around the full jobs cycle.
pub async fn run_jobs_cycle(
    paths: &DataPaths,
    app: Option<AppHandle>,
) -> AppResult<serde_json::Value> {
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&paths.runner_lock_path)?;
    if lock_file.try_lock_exclusive().is_err() {
        return Err(AppError::from(
            "Another jobs runner is already active (lock held)",
        ));
    }

    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.worker_log_path)?;
    writeln!(log, "--- jobs cycle start {} ---", crate::util::now_iso())?;

    let conn = open_runner_conn(paths)?;

    // 1) Check postings
    let job_ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM jobs")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    emit_progress(
        app.as_ref(),
        RunnerProgress {
            stage: "postings".into(),
            message: "Checking job postings".into(),
            current: 0,
            total: job_ids.len(),
        },
    );
    let mut posting_results = Vec::new();
    for (i, job_id) in job_ids.iter().enumerate() {
        let (url, previous) = load_job_check_context(&conn, job_id)?;
        let (state, result) = fetch_posting_state(&url).await;
        let applied = apply_posting_check(&conn, job_id, &previous, &state, &result)?;
        posting_results.push(serde_json::json!({ "jobId": job_id, "result": applied }));
        emit_progress(
            app.as_ref(),
            RunnerProgress {
                stage: "postings".into(),
                message: format!("Checked {job_id}"),
                current: i + 1,
                total: job_ids.len(),
            },
        );
    }

    // 2) Sync watches
    let watch_ids: Vec<(String, String, String)> = {
        let mut stmt = conn.prepare("SELECT id, provider, board_slug FROM company_watches")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    emit_progress(
        app.as_ref(),
        RunnerProgress {
            stage: "watches".into(),
            message: "Syncing ATS watches".into(),
            current: 0,
            total: watch_ids.len(),
        },
    );
    let mut watch_results = Vec::new();
    for (i, (watch_id, provider, board_slug)) in watch_ids.iter().enumerate() {
        let remote = fetch_remote_jobs(provider, board_slug)
            .await
            .map_err(|e| e.to_string());
        let result = apply_watch_sync(&conn, watch_id, remote)?;
        watch_results.push(serde_json::json!({ "watchId": watch_id, "result": result }));
        emit_progress(
            app.as_ref(),
            RunnerProgress {
                stage: "watches".into(),
                message: format!("Synced {watch_id}"),
                current: i + 1,
                total: watch_ids.len(),
            },
        );
    }

    // 3) Careers pages
    let companies: Vec<(String, String, String)> = {
        let mut stmt =
            conn.prepare("SELECT id, name, careers_url FROM companies WHERE careers_url IS NOT NULL")?;
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
        },
    );
    let mut careers_results = Vec::new();
    for (i, (company_id, name, url)) in companies.iter().enumerate() {
        match fetch_careers_hash(url).await {
            Ok((hash, text)) => {
                let result = apply_careers_check(&conn, company_id, name, &hash, &text)?;
                careers_results.push(serde_json::json!({ "companyId": company_id, "result": result }));
            }
            Err(e) => {
                careers_results.push(serde_json::json!({
                    "companyId": company_id,
                    "result": { "changed": false, "reason": e.to_string() }
                }));
            }
        }
        emit_progress(
            app.as_ref(),
            RunnerProgress {
                stage: "careers".into(),
                message: format!("Checked {name}"),
                current: i + 1,
                total: companies.len(),
            },
        );
    }

    // 4) Gmail
    let gmail = if is_gmail_connected().unwrap_or(false) {
        emit_progress(
            app.as_ref(),
            RunnerProgress {
                stage: "gmail".into(),
                message: "Polling Gmail".into(),
                current: 0,
                total: 1,
            },
        );
        // Dedicated connection avoids holding shared UI mutex; this conn is runner-owned.
        match poll_gmail_matches(&paths.db_path).await {
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
        },
    );
    let csv = sync_jobs_csv_with_disk(&paths.db_path, &paths.jobs_csv_path)?;

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
    println!("{}", serde_json::to_string_pretty(&result).unwrap_or_default());
    Ok(())
}
