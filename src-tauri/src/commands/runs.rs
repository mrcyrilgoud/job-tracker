//! Tauri commands for the persisted run monitor contract.

use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::db::AppState;
use crate::error::{AppError, AppResult};
use crate::jobs::posting_check::fetch::HttpPostingFetcher;
use crate::runs::coordinator::{RunCoordinator, RunLockGuard, RunRequest};
use crate::runs::legacy::rejection_error;
use crate::runs::model::{RunType, Trigger};
use crate::runs::progress::{LogSink, RunAccepted, RunSnapshot, TauriSink};
use crate::runs::store::{self, DismissOutcome};
use crate::runs::RunRegistry;

static REGISTRY: OnceLock<RunRegistry> = OnceLock::new();

fn registry() -> RunRegistry {
    REGISTRY.get_or_init(RunRegistry::new).clone()
}

fn parse_run_type(value: &str) -> AppResult<RunType> {
    match value {
        "jobsCycle" | "jobs_cycle" => Ok(RunType::JobsCycle),
        "postingCheck" | "posting_check" => Ok(RunType::PostingCheck),
        "careerCheck" | "career_check" => Ok(RunType::CareerCheck),
        _ => Err(AppError::coded(
            "invalid_run_type",
            "run",
            "runType must be jobsCycle, postingCheck, or careerCheck",
        )),
    }
}

fn coordinator(state: &AppState, app: AppHandle) -> RunCoordinator<HttpPostingFetcher, TauriSink> {
    let hook = {
        let csv = state.csv_export.clone();
        Arc::new(move || csv.mark_dirty())
    };
    RunCoordinator::new(
        state.paths.clone(),
        state.runner_lock.clone(),
        Arc::new(HttpPostingFetcher),
        Arc::new(TauriSink::new(app)),
        crate::runs::coordinator::SystemClock,
        registry(),
    )
    .with_csv_dirty_hook(hook)
}

fn coordinator_for_reads(state: &AppState) -> RunCoordinator<HttpPostingFetcher, LogSink> {
    RunCoordinator::new(
        state.paths.clone(),
        state.runner_lock.clone(),
        Arc::new(HttpPostingFetcher),
        Arc::new(LogSink),
        crate::runs::coordinator::SystemClock,
        registry(),
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRunInput {
    pub run_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryRunInput {
    pub source_run_id: String,
    pub job_ids: Vec<String>,
}

#[tauri::command]
pub async fn start_run_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
    input: StartRunInput,
) -> AppResult<RunAccepted> {
    let run_type = parse_run_type(&input.run_type)?;
    let coordinator = Arc::new(coordinator(&state, app));
    let request = match run_type {
        RunType::JobsCycle => RunRequest::JobsCycle {
            trigger: Trigger::Desktop,
        },
        RunType::PostingCheck => RunRequest::PostingCheck {
            trigger: Trigger::Desktop,
        },
        RunType::CareerCheck => RunRequest::CareerCheck {
            trigger: Trigger::Desktop,
        },
    };
    let accepted = coordinator.accept(request).map_err(rejection_error)?;
    let response = accepted.accepted();
    tauri::async_runtime::spawn(async move {
        let _ = coordinator.execute(accepted).await;
    });
    Ok(response)
}

#[tauri::command]
pub async fn retry_run_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
    input: RetryRunInput,
) -> AppResult<RunAccepted> {
    let coordinator = Arc::new(coordinator(&state, app));
    let accepted = coordinator
        .accept(RunRequest::Retry {
            source_run_id: input.source_run_id,
            job_ids: input.job_ids,
            trigger: Trigger::Retry,
        })
        .map_err(rejection_error)?;
    let response = accepted.accepted();
    tauri::async_runtime::spawn(async move {
        let _ = coordinator.execute(accepted).await;
    });
    Ok(response)
}

#[tauri::command]
pub async fn cancel_run_cmd(state: State<'_, AppState>, run_id: String) -> AppResult<RunSnapshot> {
    coordinator_for_reads(&state)
        .cancel(&run_id)
        .map_err(rejection_error)
}

#[tauri::command]
pub async fn get_run_cmd(state: State<'_, AppState>, run_id: String) -> AppResult<RunSnapshot> {
    state
        .with_db(|conn| {
            store::load_snapshot(
                conn,
                &run_id,
                registry().is_live(&run_id),
                &crate::util::now_iso(),
            )
        })?
        .ok_or_else(|| {
            AppError::coded(
                "run_not_found",
                "run",
                format!("Run {run_id} was not found"),
            )
        })
}

#[tauri::command]
pub async fn get_current_run_cmd(state: State<'_, AppState>) -> AppResult<Option<RunSnapshot>> {
    // A GUI reconciliation is also an opportunity to close runs abandoned by
    // a crashed process. Recovery is best-effort and only runs while both
    // exclusivity mechanisms are held; an active owner remains untouched.
    if let Ok(guard) = RunLockGuard::acquire(&state.runner_lock, &state.paths) {
        if let Ok(conn) = crate::runner::open_runner_conn(&state.paths) {
            let _ = store::recover_orphaned_runs(&conn, &crate::util::now_iso());
        }
        drop(guard);
    }
    state.with_db(|conn| {
        store::load_current(conn, |id| registry().is_live(id), &crate::util::now_iso())
    })
}

#[tauri::command]
pub async fn dismiss_run_cmd(
    state: State<'_, AppState>,
    run_id: String,
) -> AppResult<serde_json::Value> {
    let outcome =
        state.with_db_tx(|conn| store::dismiss_run(conn, &run_id, &crate::util::now_iso()))?;
    match outcome {
        DismissOutcome::Dismissed => Ok(serde_json::json!({ "ok": true })),
        DismissOutcome::NotFound => Err(AppError::coded(
            "run_not_found",
            "run",
            format!("Run {run_id} was not found"),
        )),
        DismissOutcome::NotAllowed(status) => Err(AppError::coded(
            "dismiss_not_allowed",
            status.as_str(),
            format!("Run cannot be dismissed while {}", status.as_str()),
        )),
    }
}
