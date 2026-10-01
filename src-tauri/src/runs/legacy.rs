//! Projections from a terminal `RunSnapshot` onto the legacy command and CLI JSON shapes.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::db::paths::DataPaths;
use crate::error::{AppError, AppResult};
use crate::jobs::csv::{ExportResult, ImportResult};
use crate::jobs::posting_check::fetch::PostingFetcher;
use crate::runs::coordinator::{
    Clock, RunCoordinator, RunExecution, RunRejection, RunRequest, StageItems,
};
use crate::runs::model::RunStatus;
use crate::runs::progress::{RunEventSink, RunSnapshot};
use crate::runs::RunRegistry;

/// Convert a coordinator rejection to the stable legacy coded error string.
pub fn rejection_error(rejection: RunRejection) -> AppError {
    AppError::coded(
        rejection.code_category().0,
        rejection.code_category().1,
        rejection.message(),
    )
}

/// Drive a legacy Jobs_Cycle or Posting_Check_Run through the canonical
/// coordinator and project its terminal result onto the pre-run JSON shape.
pub async fn execute_legacy<F, S, K>(
    coordinator: &RunCoordinator<F, S, K>,
    request: RunRequest,
) -> AppResult<Value>
where
    F: PostingFetcher,
    S: RunEventSink,
    K: Clock,
{
    let accepted = coordinator.accept(request).map_err(rejection_error)?;
    let execution = coordinator.execute_run(accepted).await;
    project_execution(&execution)
}

/// Build a coordinator-backed legacy result. Kept generic so Tauri can use a
/// Tauri event sink while CLI/launchd use the log sink.
pub async fn execute_legacy_with<F, S, K>(
    paths: DataPaths,
    runner_lock: Arc<tokio::sync::Mutex<()>>,
    fetcher: Arc<F>,
    sink: Arc<S>,
    clock: K,
    registry: RunRegistry,
    request: RunRequest,
    csv_dirty_hook: Option<crate::runs::coordinator::CsvDirtyHook>,
) -> AppResult<Value>
where
    F: PostingFetcher,
    S: RunEventSink,
    K: Clock,
{
    let mut coordinator = RunCoordinator::new(paths, runner_lock, fetcher, sink, clock, registry);
    if let Some(hook) = csv_dirty_hook {
        coordinator = coordinator.with_csv_dirty_hook(hook);
    }
    execute_legacy(&coordinator, request).await
}

fn project_execution(execution: &RunExecution) -> AppResult<Value> {
    let snapshot = &execution.snapshot;
    match snapshot.run_type {
        crate::runs::model::RunType::JobsCycle => {
            jobs_cycle_summary(snapshot, &execution.stage_items)
        }
        crate::runs::model::RunType::PostingCheck => postings_summary(snapshot),
        crate::runs::model::RunType::CareerCheck => {
            career_check_summary(snapshot, &execution.stage_items)
        }
    }
}

/// Project a terminal CareerCheck onto the compact payload used by the
/// career-source command. CareerCheck runs only watches and careers; it does
/// not produce posting or CSV results.
pub fn career_check_summary(snapshot: &RunSnapshot, items: &StageItems) -> AppResult<Value> {
    match snapshot.run_status {
        RunStatus::Canceled => Err(AppError::coded(
            "run_canceled",
            snapshot.run_id.clone(),
            format!("Run {} was canceled", snapshot.run_id),
        )),
        RunStatus::Error => Err(AppError::coded(
            "run_failed",
            snapshot
                .error_reason
                .clone()
                .unwrap_or_else(|| "internal".into()),
            "Run failed",
        )),
        RunStatus::Completed | RunStatus::CompletedWithErrors => Ok(json!({
            "watches": items.watches.clone(),
            "careers": items.careers.clone(),
            "runId": snapshot.run_id.clone(),
            "runStatus": snapshot.run_status,
        })),
        status => Err(AppError::coded(
            "run_failed",
            "not_terminal",
            format!("Run ended in unexpected status {}", status.as_str()),
        )),
    }
}

/// Project a terminal Jobs_Cycle while retaining the historical fields.
pub fn jobs_cycle_summary(snapshot: &RunSnapshot, items: &StageItems) -> AppResult<Value> {
    match snapshot.run_status {
        RunStatus::Canceled => Err(AppError::coded(
            "run_canceled",
            snapshot.run_id.clone(),
            format!("Run {} was canceled", snapshot.run_id),
        )),
        RunStatus::Error => Err(AppError::coded(
            "run_failed",
            snapshot
                .error_reason
                .clone()
                .unwrap_or_else(|| "internal".into()),
            "Run failed",
        )),
        RunStatus::Completed | RunStatus::CompletedWithErrors => {
            let out = json!({
                "postings": snapshot.posting_counts.total(),
                "watches": items.watches.clone(),
                "careers": items.careers.clone(),
                "csv": {
                    "imported": items.csv_imported.clone(),
                    "exported": items.csv_exported.clone(),
                },
                "runId": snapshot.run_id.clone(),
                "runStatus": snapshot.run_status,
            });
            Ok(out)
        }
        status => Err(AppError::coded(
            "run_failed",
            "not_terminal",
            format!("Run ended in unexpected status {}", status.as_str()),
        )),
    }
}

/// Project a terminal Posting_Check_Run onto its historical compact payload.
pub fn postings_summary(snapshot: &RunSnapshot) -> AppResult<Value> {
    match snapshot.run_status {
        RunStatus::Canceled => Err(AppError::coded(
            "run_canceled",
            snapshot.run_id.clone(),
            format!("Run {} was canceled", snapshot.run_id),
        )),
        RunStatus::Error => Err(AppError::coded(
            "run_failed",
            snapshot
                .error_reason
                .clone()
                .unwrap_or_else(|| "internal".into()),
            "Run failed",
        )),
        RunStatus::Completed | RunStatus::CompletedWithErrors => Ok(json!({
            "postings": snapshot.posting_counts.total(),
            "runId": snapshot.run_id.clone(),
            "runStatus": snapshot.run_status,
        })),
        status => Err(AppError::coded(
            "run_failed",
            "not_terminal",
            format!("Run ended in unexpected status {}", status.as_str()),
        )),
    }
}

// Keep these aliases local to this adapter: they document the legacy payload
// components and make compile-time changes to stage result types obvious.
#[allow(dead_code)]
type _LegacyCsvImport = Option<ImportResult>;
#[allow(dead_code)]
type _LegacyCsvExport = ExportResult;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::progress::{BoundedCount, RunSnapshot};

    fn snapshot(run_type: &str, status: &str) -> RunSnapshot {
        serde_json::from_value(json!({
            "version": 1, "runId": "r", "runType": run_type, "runStatus": status,
            "seq": 2, "stage": "cycle", "message": "done", "current": 2, "total": 2,
            "done": true, "startedAt": "2026-01-01T00:00:00Z", "elapsedMs": 1,
            "postingCounts": {"queued": 0, "active": 0, "completed": 2, "error": 0, "canceled": 0},
            "postingTotal": 2, "postings": [], "live": false, "dismissed": false
        }))
        .unwrap()
    }

    #[test]
    fn rejection_preserves_runner_wire_code() {
        assert_eq!(
            rejection_error(RunRejection::InProgress).to_string(),
            "operation_in_progress:runner"
        );
    }

    #[test]
    fn legacy_projection_uses_terminal_posting_count() {
        let mut snapshot: RunSnapshot = serde_json::from_value(json!({
            "version": 1, "runId": "r", "runType": "postingCheck", "runStatus": "completed",
            "seq": 2, "stage": "postings", "message": "done", "current": 2, "total": 2,
            "done": true, "startedAt": "2026-01-01T00:00:00Z", "elapsedMs": 1,
            "postingCounts": {"queued": 0, "active": 0, "completed": 2, "error": 0, "canceled": 0},
            "postingTotal": 2, "postings": [], "live": false, "dismissed": false
        }))
        .unwrap();
        snapshot.posting_counts.completed = 2;
        let value = project_execution(&RunExecution {
            snapshot,
            stage_items: Default::default(),
        })
        .unwrap();
        assert_eq!(value["postings"], 2);
        assert!(value["runId"] == "r");
        let _ = BoundedCount::ZERO;
    }

    #[test]
    fn jobs_cycle_projection_preserves_legacy_keys_and_adds_run_metadata() {
        let value = jobs_cycle_summary(&snapshot("jobsCycle", "completed"), &StageItems::default())
            .unwrap();
        let object = value.as_object().unwrap();
        for key in [
            "postings",
            "watches",
            "careers",
            "csv",
            "runId",
            "runStatus",
        ] {
            assert!(object.contains_key(key), "missing legacy key {key}");
        }
        assert_eq!(object["runStatus"], "completed");
    }

    #[test]
    fn posting_projection_preserves_legacy_keys_and_adds_run_metadata() {
        let value = postings_summary(&snapshot("postingCheck", "completed_with_errors")).unwrap();
        let object = value.as_object().unwrap();
        for key in ["postings", "runId", "runStatus"] {
            assert!(object.contains_key(key), "missing posting key {key}");
        }
        assert_eq!(object["runStatus"], "completed_with_errors");
    }

    #[test]
    fn career_check_projection_contains_only_its_stage_results() {
        let value = career_check_summary(
            &snapshot("careerCheck", "completed"),
            &StageItems {
                watches: vec![json!({"watchId": "w1"})],
                careers: vec![json!({"companyId": "c1"})],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(value["watches"], json!([{"watchId": "w1"}]));
        assert_eq!(value["careers"], json!([{"companyId": "c1"}]));
        assert_eq!(value["runId"], "r");
        assert_eq!(value["runStatus"], "completed");
        assert!(value.get("postings").is_none());
        assert!(value.get("csv").is_none());
    }

    #[test]
    fn canceled_and_failed_runs_keep_stable_legacy_error_codes() {
        let canceled = postings_summary(&snapshot("postingCheck", "canceled")).unwrap_err();
        assert_eq!(canceled.to_string(), "run_canceled:r");
        let failed = postings_summary(&snapshot("postingCheck", "error")).unwrap_err();
        assert_eq!(failed.code_parts().code, "run_failed");
    }
}
