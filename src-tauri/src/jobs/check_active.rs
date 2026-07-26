use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::safe_fetch::{looks_like_closed_posting, safe_fetch};
use crate::jobs::service::get_job_by_id;
use crate::util::{create_id, now_iso};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckPostingResult {
    pub posting_state: String,
    pub last_check_result: String,
    pub last_checked_at: String,
}

/// Network-only portion. Do not hold a DB mutex across this call.
pub async fn fetch_posting_state(url: &str) -> (String, String) {
    let result = safe_fetch(url, Some("GET"), None).await;
    if let Some(err) = &result.error {
        ("unknown".to_string(), format!("error: {err}"))
    } else if looks_like_closed_posting(&result.body_text, result.status) {
        (
            "inactive".to_string(),
            format!("inactive: HTTP {}", result.status),
        )
    } else if result.ok {
        (
            "active".to_string(),
            format!("active: HTTP {}", result.status),
        )
    } else {
        (
            "unknown".to_string(),
            format!("unknown: HTTP {}", result.status),
        )
    }
}

pub fn load_job_check_context(conn: &Connection, job_id: &str) -> AppResult<(String, String)> {
    let job = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
    Ok((job.url, job.posting_state))
}

pub fn apply_posting_check(
    conn: &Connection,
    job_id: &str,
    previous_state: &str,
    posting_state: &str,
    last_check_result: &str,
) -> AppResult<CheckPostingResult> {
    let checked_at = now_iso();
    conn.execute(
        "UPDATE jobs SET posting_state=?1, last_checked_at=?2, last_check_result=?3, updated_at=?2 WHERE id=?4",
        params![posting_state, checked_at, last_check_result, job_id],
    )
    .map_err(map_sqlite)?;

    if previous_state != posting_state {
        conn.execute(
            "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1,?2,'posting_state_changed',?3,?4)",
            params![
                create_id(),
                job_id,
                format!(
                    "Posting state changed from {previous_state} to {posting_state} ({last_check_result})"
                ),
                checked_at
            ],
        )
        .map_err(map_sqlite)?;
    }

    Ok(CheckPostingResult {
        posting_state: posting_state.to_string(),
        last_check_result: last_check_result.to_string(),
        last_checked_at: checked_at,
    })
}
