//! Transactional application of a classified posting check.
//!
//! [`apply_classified_check`] is the only writer of a check result to the
//! `jobs` table. It runs inside the caller's transaction (the coordinator's
//! finalize-posting `BEGIN IMMEDIATE`, or `with_db_tx` for the single-posting
//! `check_job_posting` command) and wraps its own statements in a `SAVEPOINT`,
//! so it is atomic on its own and leaves nothing behind on error (Req 7.10).
//!
//! Per result it:
//! 1. Updates only the availability columns `posting_state`, `last_checked_at`
//!    (the attempt time), `last_check_result`, and `updated_at` (Req 6.17,
//!    8.7). Status, notes, favorite, and every other column are untouched.
//! 2. Inserts exactly one `posting_state_changed` job event when the persisted
//!    state differs from the job's current state, with the previous state, the
//!    new state, and the Classification_Reason (Req 7.8).
//! 3. Inserts the authoritative `posting_check_evidence` row (Req 7.1).
//!
//! The `run_postings` row is moved to Completed separately by
//! `runs::store::finalize_posting`, in the same caller transaction.

use rusqlite::{params, Connection, OptionalExtension};

use super::classify::Classification;
use super::evidence::{CheckEvidence, EVIDENCE_VERSION};
use crate::error::{map_sqlite, AppError, AppResult};
use crate::runs::model::PostingState;
use crate::runs::store::{self, EvidenceKind, EvidenceRecord};
use crate::util::{create_id, now_iso};

/// Error code returned when the job row no longer exists. The coordinator maps
/// it to `FailureCategory::Persistence` with detail `job_missing`.
pub const JOB_MISSING: &str = "job_missing";

/// Job event type for availability changes. Shared with the watch sync
/// (`ats::sync`) and the legacy check path, which write the same type.
pub const POSTING_STATE_CHANGED: &str = "posting_state_changed";

const SAVEPOINT: &str = "posting_check_persist";

/// Outcome of [`apply_classified_check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedCheck {
    /// `jobs.posting_state` before this result, as persisted text.
    pub previous_state: String,
    pub posting_state: PostingState,
    /// Value written to `jobs.last_check_result`.
    pub last_check_result: String,
    /// Value written to `jobs.last_checked_at` (the attempt time).
    pub last_checked_at: String,
    /// True when a `posting_state_changed` event was inserted.
    pub state_changed: bool,
    /// Id of the authoritative evidence row.
    pub evidence_id: String,
}

/// True when `err` is the [`JOB_MISSING`] error from [`apply_classified_check`].
pub fn is_job_missing(err: &AppError) -> bool {
    matches!(err, AppError::Message(m) if m == JOB_MISSING || m.starts_with("job_missing:"))
}

fn job_missing(job_id: &str) -> AppError {
    AppError::Message(format!("{JOB_MISSING}:{job_id}"))
}

/// Human-readable note for a `posting_state_changed` event.
pub fn state_change_note(previous: &str, new: PostingState, reason: &str) -> String {
    format!(
        "Posting state changed from {previous} to {} ({reason})",
        new.as_str()
    )
}

/// Apply one classified check to the job row, job history, and evidence
/// history. `run_id` is `None` for `check_job_posting`.
///
/// `evidence` is normalized before it is stored; `classification` should be
/// `classify(evidence)` (the caller classifies so the coordinator can publish
/// the same value it persists).
pub fn apply_classified_check(
    tx: &Connection,
    run_id: Option<&str>,
    job_id: &str,
    evidence: &CheckEvidence,
    classification: &Classification,
) -> AppResult<AppliedCheck> {
    let evidence = evidence.normalized();
    let evidence_json = evidence
        .to_json()
        .map_err(|e| AppError::Message(format!("evidence_serialize:{e}")))?;
    let checked_at = if evidence.attempted_at.trim().is_empty() {
        now_iso()
    } else {
        evidence.attempted_at.clone()
    };
    let updated_at = now_iso();
    let new_state = classification.state;
    let last_check_result = classification.last_check_result();

    in_savepoint(tx, |c| {
        let previous_state: String = c
            .query_row(
                "SELECT posting_state FROM jobs WHERE id = ?1",
                params![job_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sqlite)?
            .ok_or_else(|| job_missing(job_id))?;

        let updated = c
            .execute(
                "UPDATE jobs SET posting_state = ?1, last_checked_at = ?2, last_check_result = ?3,
                     updated_at = ?4
                 WHERE id = ?5",
                params![
                    new_state.as_str(),
                    checked_at,
                    last_check_result,
                    updated_at,
                    job_id
                ],
            )
            .map_err(map_sqlite)?;
        if updated == 0 {
            return Err(job_missing(job_id));
        }

        let state_changed = previous_state != new_state.as_str();
        if state_changed {
            c.execute(
                "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    create_id(),
                    job_id,
                    POSTING_STATE_CHANGED,
                    state_change_note(&previous_state, new_state, &classification.reason),
                    checked_at,
                ],
            )
            .map_err(map_sqlite)?;
        }

        let evidence_id = store::insert_evidence(
            c,
            run_id,
            job_id,
            EvidenceKind::Authoritative,
            &EvidenceRecord {
                attempted_at: checked_at.clone(),
                posting_state: new_state,
                reason_code: classification.reason_code.clone(),
                reason: classification.reason.clone(),
                evidence_version: EVIDENCE_VERSION,
                evidence_json,
                created_at: updated_at.clone(),
            },
        )?;

        Ok(AppliedCheck {
            previous_state,
            posting_state: new_state,
            last_check_result: last_check_result.clone(),
            last_checked_at: checked_at.clone(),
            state_changed,
            evidence_id,
        })
    })
}

/// Most recent conclusive (Active or Closed) Posting_State in the job's
/// authoritative evidence history, skipping Unknown results (Req 7.7). When the
/// latest check is Unknown, this is the conclusive state that preceded it.
/// `None` when the job has never had a conclusive result. Retention
/// (`store::prune_history`) always keeps the row this reads.
pub fn preceding_conclusive_state(
    conn: &Connection,
    job_id: &str,
) -> AppResult<Option<PostingState>> {
    store::latest_conclusive_evidence_state(conn, job_id)
}

/// Run `f` inside a named `SAVEPOINT`, rolling back to it on error. Nests
/// inside a caller transaction, or acts as its own transaction standalone.
fn in_savepoint<T>(conn: &Connection, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
    conn.execute_batch(&format!("SAVEPOINT {SAVEPOINT}"))
        .map_err(map_sqlite)?;
    let rollback = || {
        let _ = conn.execute_batch(&format!("ROLLBACK TO {SAVEPOINT}; RELEASE {SAVEPOINT}"));
    };
    match f(conn) {
        Ok(value) => match conn.execute_batch(&format!("RELEASE {SAVEPOINT}")) {
            Ok(()) => Ok(value),
            Err(err) => {
                rollback();
                Err(map_sqlite(err))
            }
        },
        Err(err) => {
            rollback();
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;
    use crate::jobs::posting_check::classify::classify;
    use crate::jobs::posting_check::evidence::FailureCategory;

    const URL: &str = "https://acme.com/jobs/1";
    const T1: &str = "2026-01-01T00:00:01.000Z";
    const T2: &str = "2026-01-01T00:00:02.000Z";
    const T3: &str = "2026-01-01T00:00:03.000Z";

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO companies (id, name, created_at, updated_at) VALUES ('c1', 'Acme', ?1, ?1)",
            params!["2025-01-01T00:00:00.000Z"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO jobs (id, company_id, title, url, canonical_url, status, applied_at,
                 posting_state, source, notes, description, location, is_favorite,
                 created_at, updated_at)
             VALUES ('j1', 'c1', 'Engineer', ?1, ?1, 'applied', '2025-06-01', 'active', 'manual',
                 'my notes', 'desc', 'Remote', 1, '2025-01-01T00:00:00.000Z',
                 '2025-01-01T00:00:00.000Z')",
            params![URL],
        )
        .unwrap();
        conn
    }

    /// Evidence classified as `state`.
    fn evidence(state: PostingState, at: &str) -> CheckEvidence {
        let mut ev = CheckEvidence::new(URL, at);
        match state {
            PostingState::Inactive => ev.http_status = Some(404),
            PostingState::Unknown => ev.failure = Some(FailureCategory::Timeout),
            PostingState::Active => {
                use crate::jobs::posting_check::evidence::ContentSignal::*;
                ev.http_status = Some(200);
                ev.content = [TitleMatch, CompanyMatch, ApplyEnabled]
                    .into_iter()
                    .collect();
            }
        }
        assert_eq!(classify(&ev).state, state);
        ev
    }

    fn apply(
        conn: &Connection,
        run_id: Option<&str>,
        state: PostingState,
        at: &str,
    ) -> AppliedCheck {
        let ev = evidence(state, at);
        apply_classified_check(conn, run_id, "j1", &ev, &classify(&ev)).unwrap()
    }

    /// Every jobs column except the availability columns.
    fn other_columns(conn: &Connection) -> Vec<Option<String>> {
        conn.query_row(
            "SELECT company_id, title, url, canonical_url, source_external_id, status, applied_at,
                    source, notes, description, location, CAST(is_new_from_watch AS TEXT),
                    watch_disposition, CAST(missing_from_sync_count AS TEXT),
                    CAST(is_favorite AS TEXT), created_at
             FROM jobs WHERE id = 'j1'",
            [],
            |r| (0..16).map(|i| r.get::<_, Option<String>>(i)).collect(),
        )
        .unwrap()
    }

    fn job_row(conn: &Connection) -> (String, Option<String>, Option<String>, String) {
        conn.query_row(
            "SELECT posting_state, last_checked_at, last_check_result, updated_at FROM jobs WHERE id = 'j1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
    }

    fn events(conn: &Connection) -> Vec<(String, String, String)> {
        let mut stmt = conn
            .prepare("SELECT type, note, occurred_at FROM job_events WHERE job_id = 'j1' ORDER BY occurred_at")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn evidence_rows(conn: &Connection) -> Vec<(Option<String>, String, String, String, String)> {
        let mut stmt = conn
            .prepare(
                "SELECT run_id, kind, posting_state, attempted_at, evidence_json
                 FROM posting_check_evidence WHERE job_id = 'j1' ORDER BY attempted_at",
            )
            .unwrap();
        stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    #[test]
    fn only_availability_columns_change() {
        let conn = db();
        let before = other_columns(&conn);
        let applied = apply(&conn, None, PostingState::Inactive, T1);

        assert_eq!(other_columns(&conn), before);
        let (state, checked, result, updated) = job_row(&conn);
        assert_eq!(state, "inactive");
        assert_eq!(checked.as_deref(), Some(T1));
        assert_eq!(
            result.as_deref(),
            Some("inactive: posting returned HTTP 404")
        );
        assert_ne!(updated, "2025-01-01T00:00:00.000Z");
        assert_eq!(applied.previous_state, "active");
        assert_eq!(applied.last_checked_at, T1);
        assert_eq!(
            applied.last_check_result,
            "inactive: posting returned HTTP 404"
        );
    }

    #[test]
    fn last_check_result_uses_state_prefix_format() {
        let conn = db();
        let ev = evidence(PostingState::Unknown, T1);
        let c = classify(&ev);
        let applied = apply_classified_check(&conn, None, "j1", &ev, &c).unwrap();
        assert!(
            applied.last_check_result.starts_with("unknown: "),
            "{}",
            applied.last_check_result
        );
        assert!(!applied.last_check_result.contains("Unknown:"));
        assert_eq!(
            job_row(&conn).2.as_deref(),
            Some(applied.last_check_result.as_str())
        );
        assert_eq!(applied.last_check_result, c.last_check_result());
    }

    #[test]
    fn state_change_inserts_exactly_one_event_and_unchanged_state_none() {
        let conn = db();
        // active -> active: no event.
        let a = apply(&conn, None, PostingState::Active, T1);
        assert!(!a.state_changed);
        assert!(events(&conn).is_empty());

        // active -> inactive: one event.
        let b = apply(&conn, None, PostingState::Inactive, T2);
        assert!(b.state_changed);
        let ev = events(&conn);
        assert_eq!(ev.len(), 1);
        let (kind, note, occurred) = &ev[0];
        assert_eq!(kind, POSTING_STATE_CHANGED);
        assert_eq!(occurred, T2);
        assert!(note.contains("from active to inactive"), "{note}");
        assert!(note.contains("Closed: posting returned HTTP 404"), "{note}");

        // inactive -> inactive: still one event.
        apply(&conn, None, PostingState::Inactive, T3);
        assert_eq!(events(&conn).len(), 1);
    }

    #[test]
    fn missing_job_returns_job_missing_and_writes_nothing() {
        let conn = db();
        let ev = evidence(PostingState::Inactive, T1);
        let err = apply_classified_check(&conn, None, "nope", &ev, &classify(&ev)).unwrap_err();
        assert!(is_job_missing(&err), "{err}");
        assert!(err.to_string().starts_with("job_missing"));
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM posting_check_evidence", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn evidence_row_written_with_and_without_run_id() {
        let conn = db();
        apply(&conn, None, PostingState::Active, T1);
        apply(&conn, Some("run-1"), PostingState::Inactive, T2);

        let rows = evidence_rows(&conn);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, None);
        assert_eq!(rows[1].0.as_deref(), Some("run-1"));
        for (_, kind, _, _, _) in &rows {
            assert_eq!(kind, "authoritative");
        }
        assert_eq!((rows[0].2.as_str(), rows[0].3.as_str()), ("active", T1));
        assert_eq!((rows[1].2.as_str(), rows[1].3.as_str()), ("inactive", T2));

        let stored = store::load_authoritative_evidence(&conn, "run-1", "j1")
            .unwrap()
            .unwrap();
        let ev = evidence(PostingState::Inactive, T2);
        assert_eq!(
            CheckEvidence::from_json(&stored.record.evidence_json).unwrap(),
            ev.normalized()
        );
        assert_eq!(stored.record.reason, classify(&ev).reason);
        assert_eq!(stored.record.evidence_version, EVIDENCE_VERSION);
    }

    #[test]
    fn nested_in_caller_transaction_rolls_back_with_it() {
        let conn = db();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        apply(&conn, None, PostingState::Inactive, T1);
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(job_row(&conn).0, "active");
        assert!(events(&conn).is_empty());
        assert!(evidence_rows(&conn).is_empty());
    }

    #[test]
    fn preceding_conclusive_state_skips_unknown() {
        let conn = db();
        assert_eq!(preceding_conclusive_state(&conn, "j1").unwrap(), None);
        apply(&conn, None, PostingState::Unknown, T1);
        assert_eq!(preceding_conclusive_state(&conn, "j1").unwrap(), None);
        apply(&conn, None, PostingState::Inactive, T2);
        apply(&conn, None, PostingState::Unknown, T3);
        assert_eq!(job_row(&conn).0, "unknown");
        assert_eq!(
            preceding_conclusive_state(&conn, "j1").unwrap(),
            Some(PostingState::Inactive)
        );
    }
}
