//! SQLite persistence for `runs`, `run_postings`, and `posting_check_evidence`.
//!
//! Rules (design.md, "RunStore" and "Key design decisions"):
//! - The database is the source of truth. Every lifecycle and per-posting
//!   transition is a conditional `UPDATE … WHERE status IN (…)`, so a
//!   transition that lost a race (for example a posting start after a
//!   committed cancel, Req 5.4) affects zero rows instead of overwriting state.
//! - The store never reads a clock. Every timestamp (`at`, `now`,
//!   `finished_at`, …) is an RFC 3339 string supplied by the caller.
//! - Every function takes `&Connection`. Multi-statement functions run inside
//!   an internal `SAVEPOINT`, so they are atomic on their own and nest inside a
//!   caller's `BEGIN IMMEDIATE` transaction (for example the finalize-posting
//!   transaction, which also runs `apply_classified_check`).
//! - Split of the finalize-posting transaction: `finalize_posting` only moves
//!   the `run_postings` row to Completed. The `jobs` availability update, the
//!   `posting_state_changed` job event, and the authoritative evidence insert
//!   belong to `jobs::posting_check::persist::apply_classified_check` (task
//!   5.12), which may use [`insert_evidence`] for the evidence row.
//! - Evidence is accepted pre-serialized ([`EvidenceRecord::evidence_json`] +
//!   `evidence_version`), because the `CheckEvidence` type lives in
//!   `jobs::posting_check` and is not a dependency of the store.

use std::collections::HashMap;

use chrono::DateTime;
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::model::{
    JobIdentity, PostingCounts, PostingState, PostingStatus, RunId, RunStatus, RunType, StageName,
    StageOutcome, Trigger,
};
use super::progress::{
    bounded, BoundedCount, ContractBounds, LegacyStage, PostingOutcomes, PostingProgress,
    RunSnapshot, RunSummary, StageProgress, MAX_MESSAGE_BYTES, PROGRESS_CONTRACT_VERSION,
};
use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::posting_check::evidence::CheckEvidence;

/// Terminal runs kept by [`prune_history`] (design "Retention").
pub const DEFAULT_KEEP_RUNS: usize = 50;
/// Authoritative evidence rows kept per job, plus the latest conclusive row.
pub const DEFAULT_KEEP_EVIDENCE_PER_JOB: usize = 20;
/// Supplementary evidence older than this is deleted.
pub const SUPPLEMENTARY_RETENTION_DAYS: i64 = 30;

/// `error_reason` for runs closed out by [`recover_orphaned_runs`].
pub const RUNNER_INTERRUPTED: &str = "runner_interrupted";
/// Failure category for postings that were Active when their run stopped.
pub const RUN_ABORTED_CATEGORY: &str = "run_aborted";

/// SQL `IN` lists built from the enum `as_str` values (checked by a unit test).
const OPEN_RUN_STATUSES: &str = "('queued','active','canceling')";
const TERMINAL_RUN_STATUSES: &str = "('canceled','completed','completed_with_errors','error')";

// ---------------------------------------------------------------------------
// Inputs and outcomes
// ---------------------------------------------------------------------------

/// A run at its accept point (design "Accept algorithm", step 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRun {
    pub id: RunId,
    pub run_type: RunType,
    pub trigger: Trigger,
    /// Set for a Retry_Run; must reference an existing run.
    pub source_run_id: Option<String>,
    pub owner_pid: u32,
    /// Recorded as both `started_at` and `updated_at` (Req 1.2).
    pub accepted_at: String,
    /// Initial stage list (all `not_started`); persisted as `stages_json`.
    pub stages: Vec<StageProgress>,
}

/// One posting of an accepted run: frozen Job_Identity (Req 3.10) plus the
/// job's `posting_state` at accept time, used for the summary's state-change count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobIdentityWithState {
    pub identity: JobIdentity,
    pub state_at_start: String,
}

/// Authoritative result for an Active posting (design "Finalize posting", step 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedPosting {
    pub job_id: String,
    pub posting_state: PostingState,
    pub reason_code: String,
    /// Classification_Reason; must be non-empty.
    pub reason: String,
    /// Set for Completed results that still carry a failure, for example `timeout`.
    pub failure_category: Option<String>,
    pub attempted_at: String,
    pub finished_at: String,
}

/// Result of [`request_cancel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The run moved to Canceling from `previous` (Queued or Active).
    Accepted {
        previous: RunStatus,
    },
    /// The run exists but is not Queued or Active; nothing was modified (Req 5.11).
    NotAllowed(RunStatus),
    NotFound,
}

/// Result of [`dismiss_run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DismissOutcome {
    /// Dismissed now, or already dismissed (the first `dismissed_at` is kept).
    Dismissed,
    /// The run is not terminal; nothing was modified.
    NotAllowed(RunStatus),
    NotFound,
}

/// Terminal record written by [`finalize_run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRecord {
    /// Must be terminal (derived by `lifecycle::next_status`).
    pub status: RunStatus,
    pub finished_at: String,
    /// Required and non-empty iff `status == Error`; ignored otherwise.
    pub error_reason: Option<String>,
    pub stages: Vec<StageProgress>,
    /// `started_at`, `finished_at`, `duration_ms`, and `status` are overwritten
    /// with the persisted values so the summary never disagrees with the row.
    pub summary: Option<RunSummary>,
    /// Seq of the terminal event; `last_seq` only moves forward.
    pub seq: u64,
}

/// Postings closed out by [`abort_open_postings`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AbortedPostings {
    /// Previously Queued, now Canceled.
    pub canceled: Vec<String>,
    /// Previously Active, now Error (`run_aborted`).
    pub errored: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind {
    Authoritative,
    Supplementary,
}

impl EvidenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Authoritative => "authoritative",
            Self::Supplementary => "supplementary",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::Authoritative, Self::Supplementary]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

/// One `posting_check_evidence` row's payload. `evidence_json` is the
/// sanitized, normalized `CheckEvidence` serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub attempted_at: String,
    pub posting_state: PostingState,
    pub reason_code: String,
    pub reason: String,
    pub evidence_version: u32,
    pub evidence_json: String,
    pub created_at: String,
}

/// A loaded `posting_check_evidence` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvidence {
    pub id: String,
    pub run_id: Option<String>,
    pub job_id: String,
    pub kind: EvidenceKind,
    pub record: EvidenceRecord,
}

/// Rows deleted by [`prune_history`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PruneReport {
    pub runs: usize,
    pub run_postings: usize,
    pub authoritative_evidence: usize,
    pub supplementary_evidence: usize,
}

/// Legacy `stage` / `message` / `current` / `total` fields (Req 11.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyProgress {
    pub stage: LegacyStage,
    pub message: String,
    pub current: BoundedCount,
    pub total: BoundedCount,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run `f` inside `SAVEPOINT`. Rolls back to the savepoint on error. Works both
/// standalone (the savepoint is its own transaction) and nested in a caller's
/// transaction.
fn in_savepoint<T>(conn: &Connection, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
    conn.execute_batch("SAVEPOINT runs_store")
        .map_err(map_sqlite)?;
    let rollback = || {
        let _ = conn.execute_batch("ROLLBACK TO runs_store; RELEASE runs_store");
    };
    match f(conn) {
        Ok(value) => match conn.execute_batch("RELEASE runs_store") {
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

fn status_sql_list(statuses: &[RunStatus]) -> String {
    // Values come from `RunStatus::as_str` ('static enum text), never from input.
    let items: Vec<String> = statuses
        .iter()
        .map(|s| format!("'{}'", s.as_str()))
        .collect();
    format!("({})", items.join(","))
}

fn corrupt(what: &str, id: &str, value: &str) -> AppError {
    AppError::Message(format!("corrupt run data: {what} {value:?} for {id}"))
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

fn is_blank(s: &str) -> bool {
    s.trim().is_empty()
}

/// Milliseconds from `start` to `end` (RFC 3339), saturating at 0 when `end`
/// precedes `start`. The coordinator uses the same function for `RunTiming`,
/// so the persisted `duration_ms` equals `finished_at − started_at` (Req 1.8).
pub fn duration_ms_between(start: &str, end: &str) -> AppResult<u64> {
    let parse = |s: &str| {
        DateTime::parse_from_rfc3339(s)
            .map_err(|e| AppError::Message(format!("invalid timestamp {s:?}: {e}")))
    };
    let ms = (parse(end)? - parse(start)?).num_milliseconds();
    Ok(to_u64(ms))
}

fn stages_to_json(stages: &[StageProgress]) -> AppResult<String> {
    serde_json::to_string(stages).map_err(|e| AppError::Message(format!("stages_json: {e}")))
}

/// Terminal stage rule (Req 2.2, 9.4): `not_started` → `skipped`, and
/// `in_progress` → `failed` with `interrupted_error` (or `skipped` when `None`).
pub fn settle_stages_for_terminal(stages: &mut [StageProgress], interrupted_error: Option<&str>) {
    for stage in stages {
        match stage.outcome {
            StageOutcome::NotStarted => stage.outcome = StageOutcome::Skipped,
            StageOutcome::InProgress => match interrupted_error {
                Some(err) => {
                    stage.outcome = StageOutcome::Failed;
                    stage.error.get_or_insert_with(|| err.to_string());
                }
                None => stage.outcome = StageOutcome::Skipped,
            },
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

const RUN_COLUMNS: &str = "id, run_type, status, source_run_id, started_at, finished_at, \
     duration_ms, error_reason, stages_json, summary_json, last_seq, dismissed_at";

struct RawRunRow {
    id: String,
    run_type: String,
    status: String,
    source_run_id: Option<String>,
    started_at: String,
    finished_at: Option<String>,
    duration_ms: Option<i64>,
    error_reason: Option<String>,
    stages_json: String,
    summary_json: Option<String>,
    last_seq: i64,
    dismissed_at: Option<String>,
}

fn map_raw_run(row: &Row<'_>) -> rusqlite::Result<RawRunRow> {
    Ok(RawRunRow {
        id: row.get(0)?,
        run_type: row.get(1)?,
        status: row.get(2)?,
        source_run_id: row.get(3)?,
        started_at: row.get(4)?,
        finished_at: row.get(5)?,
        duration_ms: row.get(6)?,
        error_reason: row.get(7)?,
        stages_json: row.get(8)?,
        summary_json: row.get(9)?,
        last_seq: row.get(10)?,
        dismissed_at: row.get(11)?,
    })
}

struct RunRow {
    id: String,
    run_type: RunType,
    status: RunStatus,
    source_run_id: Option<String>,
    started_at: String,
    finished_at: Option<String>,
    duration_ms: Option<u64>,
    error_reason: Option<String>,
    stages: Vec<StageProgress>,
    summary_json: Option<String>,
    last_seq: u64,
    dismissed: bool,
}

impl TryFrom<RawRunRow> for RunRow {
    type Error = AppError;
    fn try_from(r: RawRunRow) -> AppResult<Self> {
        let run_type =
            RunType::parse(&r.run_type).ok_or_else(|| corrupt("run_type", &r.id, &r.run_type))?;
        let status =
            RunStatus::parse(&r.status).ok_or_else(|| corrupt("status", &r.id, &r.status))?;
        let stages: Vec<StageProgress> = serde_json::from_str(&r.stages_json)
            .map_err(|_| corrupt("stages_json", &r.id, &r.stages_json))?;
        Ok(Self {
            id: r.id,
            run_type,
            status,
            source_run_id: r.source_run_id,
            started_at: r.started_at,
            finished_at: r.finished_at,
            duration_ms: r.duration_ms.map(to_u64),
            error_reason: r.error_reason,
            stages,
            summary_json: r.summary_json,
            last_seq: to_u64(r.last_seq),
            dismissed: r.dismissed_at.is_some(),
        })
    }
}

fn load_run_row(conn: &Connection, run_id: &str) -> AppResult<Option<RunRow>> {
    let raw = conn
        .query_row(
            &format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?1"),
            params![run_id],
            map_raw_run,
        )
        .optional()
        .map_err(map_sqlite)?;
    raw.map(RunRow::try_from).transpose()
}

struct PostingRow {
    job_id: String,
    title: String,
    company_name: String,
    posting_url: String,
    state_at_start: String,
    status: PostingStatus,
    posting_state: Option<PostingState>,
    reason_code: Option<String>,
    reason: Option<String>,
    failure_category: Option<String>,
}

impl PostingRow {
    fn to_progress(
        &self,
        evidence: Option<crate::runs::progress::EvidenceView>,
    ) -> PostingProgress {
        let completed = self.status == PostingStatus::Completed;
        let has_reason = completed || self.status == PostingStatus::Error;
        let mut p = PostingProgress {
            job_id: self.job_id.clone(),
            title: self.title.clone(),
            company_name: self.company_name.clone(),
            posting_url: self.posting_url.clone(),
            status: self.status,
            posting_state: if completed { self.posting_state } else { None },
            reason_code: self.reason_code.clone(),
            reason: if has_reason {
                self.reason.clone()
            } else {
                None
            },
            failure_category: self.failure_category.clone(),
            evidence: if completed { evidence } else { None },
        };
        p.enforce_bounds();
        p
    }
}

fn load_posting_rows(conn: &Connection, run_id: &str) -> AppResult<Vec<PostingRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT job_id, job_title, company_name, posting_url, state_at_start, status,
                    posting_state, reason_code, reason, failure_category
             FROM run_postings WHERE run_id = ?1 ORDER BY ordinal, job_id",
        )
        .map_err(map_sqlite)?;
    let raw = stmt
        .query_map(params![run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    raw.into_iter()
        .map(
            |(
                job_id,
                title,
                company_name,
                posting_url,
                state_at_start,
                status,
                state,
                reason_code,
                reason,
                failure_category,
            )| {
                let status = PostingStatus::parse(&status)
                    .ok_or_else(|| corrupt("posting status", &job_id, &status))?;
                let posting_state = match state {
                    Some(s) => Some(
                        PostingState::parse(&s)
                            .ok_or_else(|| corrupt("posting_state", &job_id, &s))?,
                    ),
                    None => None,
                };
                Ok(PostingRow {
                    job_id,
                    title,
                    company_name,
                    posting_url,
                    state_at_start,
                    status,
                    posting_state,
                    reason_code,
                    reason,
                    failure_category,
                })
            },
        )
        .collect()
}

fn counts_of(rows: &[PostingRow]) -> PostingCounts {
    let mut c = PostingCounts::default();
    for r in rows {
        match r.status {
            PostingStatus::Queued => c.queued += 1,
            PostingStatus::Active => c.active += 1,
            PostingStatus::Completed => c.completed += 1,
            PostingStatus::Error => c.error += 1,
            PostingStatus::Canceled => c.canceled += 1,
        }
    }
    c
}

/// Run_Summary rebuilt from rows, used when `summary_json` is absent or
/// unreadable (for example a run closed out by orphan recovery).
fn summary_from_rows(
    run: &RunRow,
    status: RunStatus,
    rows: &[PostingRow],
    stages: &[StageProgress],
    finished_at: &str,
    duration_ms: u64,
) -> RunSummary {
    let mut outcomes = PostingOutcomes::default();
    let mut state_changes = 0u64;
    for r in rows {
        match r.status {
            PostingStatus::Queued | PostingStatus::Active => {}
            PostingStatus::Completed => {
                let state = r.posting_state.unwrap_or(PostingState::Unknown);
                match state {
                    PostingState::Active => outcomes.active += 1,
                    PostingState::Inactive => outcomes.closed += 1,
                    PostingState::Unknown => outcomes.unknown += 1,
                }
                if state.as_str() != r.state_at_start {
                    state_changes += 1;
                }
            }
            PostingStatus::Error => outcomes.error += 1,
            PostingStatus::Canceled => outcomes.canceled += 1,
        }
    }
    let mut summary = RunSummary {
        run_id: run.id.clone(),
        run_type: run.run_type,
        status,
        started_at: run.started_at.clone(),
        finished_at: finished_at.to_string(),
        duration_ms: BoundedCount::new(duration_ms),
        posting_outcomes: outcomes,
        state_changes: BoundedCount::new(state_changes),
        stages: (run.run_type == RunType::JobsCycle).then(|| stages.to_vec()),
        source_run_id: run.source_run_id.clone(),
    };
    summary.enforce_bounds();
    summary
}

// ---------------------------------------------------------------------------
// Legacy progress derivation (pure)
// ---------------------------------------------------------------------------

fn run_label(run_type: RunType) -> &'static str {
    match run_type {
        RunType::JobsCycle => "Jobs cycle",
        RunType::PostingCheck => "Posting check",
    }
}

/// Derive the legacy fields for a run state. Pure; shared by snapshots and
/// (later) the coordinator's events so both report the same values.
///
/// - Posting_Check_Run: `stage = postings`, `current/total` =
///   `completed + error + canceled` / posting total.
/// - Jobs_Cycle, terminal: `stage = cycle`, `current = total = 1`.
/// - Jobs_Cycle, non-terminal: the `in_progress` stage, else the last stage
///   that finished, else `postings`. The postings stage reports posting counts;
///   other stages report their own `current/total`.
pub fn legacy_progress(
    run_type: RunType,
    status: RunStatus,
    stages: &[StageProgress],
    counts: &PostingCounts,
    error_reason: Option<&str>,
) -> LegacyProgress {
    let settled = counts.completed + counts.error + counts.canceled;
    let posting_total = counts.total();
    let label = run_label(run_type);

    let (stage, current, total) = if run_type == RunType::JobsCycle && status.is_terminal() {
        (LegacyStage::Cycle, 1, 1)
    } else if run_type == RunType::PostingCheck {
        (LegacyStage::Postings, settled, posting_total)
    } else {
        let current_stage = stages
            .iter()
            .find(|s| s.outcome == StageOutcome::InProgress)
            .or_else(|| {
                stages
                    .iter()
                    .rev()
                    .find(|s| matches!(s.outcome, StageOutcome::Succeeded | StageOutcome::Failed))
            });
        match current_stage {
            Some(s) if s.name != StageName::Postings => {
                (s.name.into(), s.current.get(), s.total.get())
            }
            _ => (LegacyStage::Postings, settled, posting_total),
        }
    };

    let progress_text = || match stage {
        LegacyStage::Postings => format!("Checked {current} of {total} postings"),
        LegacyStage::Watches => format!("Syncing company watches ({current}/{total})"),
        LegacyStage::Careers => format!("Checking careers pages ({current}/{total})"),
        LegacyStage::Csv => "Syncing CSV mirror".to_string(),
        LegacyStage::Cycle => format!("{label} finished"),
    };
    let message = match status {
        RunStatus::Queued => format!("{label} queued"),
        RunStatus::Active => progress_text(),
        RunStatus::Canceling => format!("Canceling: {}", progress_text()),
        RunStatus::Canceled => format!("{label} canceled"),
        RunStatus::Completed => format!("{label} completed"),
        RunStatus::CompletedWithErrors => format!("{label} completed with errors"),
        RunStatus::Error => match error_reason.filter(|r| !is_blank(r)) {
            Some(reason) => format!("{label} failed: {reason}"),
            None => format!("{label} failed"),
        },
    };

    let total = BoundedCount::new(total);
    LegacyProgress {
        stage,
        message: bounded(message, MAX_MESSAGE_BYTES),
        current: BoundedCount::new(current).min(total),
        total,
    }
}

// ---------------------------------------------------------------------------
// Runs: accept, status CAS, progress, cancel, finalize
// ---------------------------------------------------------------------------

/// Insert the accepted run (status `queued`, `owns_runner_lock = 1`,
/// `last_seq = 1`, `started_at = accepted_at`) and all its postings as `queued`
/// in ordinal order. Atomic: on any error nothing is written. A second
/// lock-owning run is rejected by `runs_single_lock_owner_uidx` (Req 4.8), and
/// a duplicate job id by the `run_postings` primary key.
pub fn insert_accepted_run(
    conn: &Connection,
    run: &NewRun,
    postings: &[JobIdentityWithState],
) -> AppResult<()> {
    let stages_json = stages_to_json(&run.stages)?;
    in_savepoint(conn, |c| {
        c.execute(
            "INSERT INTO runs (id, run_type, status, trigger, source_run_id, owner_pid,
                               owns_runner_lock, started_at, stages_json, last_seq, updated_at)
             VALUES (?1, ?2, 'queued', ?3, ?4, ?5, 1, ?6, ?7, 1, ?6)",
            params![
                run.id.as_str(),
                run.run_type.as_str(),
                run.trigger.as_str(),
                run.source_run_id,
                i64::from(run.owner_pid),
                run.accepted_at,
                stages_json,
            ],
        )
        .map_err(map_sqlite)?;
        let mut stmt = c
            .prepare(
                "INSERT INTO run_postings (run_id, job_id, ordinal, job_title, company_name,
                                           posting_url, state_at_start, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued')",
            )
            .map_err(map_sqlite)?;
        for (ordinal, p) in postings.iter().enumerate() {
            stmt.execute(params![
                run.id.as_str(),
                p.identity.job_id,
                to_i64(ordinal as u64),
                p.identity.title,
                p.identity.company_name,
                p.identity.posting_url,
                p.state_at_start,
            ])
            .map_err(map_sqlite)?;
        }
        Ok(())
    })
}

/// Compare-and-set the run status: `from` → `to`, only if the current status
/// is in `from`. Returns whether the row changed. Moving to Active records
/// `activated_at` once; moving to Canceling records `cancel_requested_at` once.
/// Terminal targets are rejected: use [`finalize_run`], which also releases
/// `owns_runner_lock` and records timing.
pub fn cas_run_status(
    conn: &Connection,
    run_id: &str,
    from: &[RunStatus],
    to: RunStatus,
    at: &str,
) -> AppResult<bool> {
    if to.is_terminal() {
        return Err(AppError::Message(format!(
            "cas_run_status cannot set terminal status {}; use finalize_run",
            to.as_str()
        )));
    }
    if from.is_empty() {
        return Ok(false);
    }
    let sql = format!(
        "UPDATE runs SET
           status = ?2,
           updated_at = ?3,
           activated_at = CASE WHEN ?2 = 'active' THEN COALESCE(activated_at, ?3) ELSE activated_at END,
           cancel_requested_at = CASE WHEN ?2 = 'canceling' THEN COALESCE(cancel_requested_at, ?3)
                                      ELSE cancel_requested_at END
         WHERE id = ?1 AND status IN {}",
        status_sql_list(from)
    );
    let changed = conn
        .execute(&sql, params![run_id, to.as_str(), at])
        .map_err(map_sqlite)?;
    Ok(changed == 1)
}

/// Current persisted status (for the coordinator's cross-process cancel poll).
pub fn run_status(conn: &Connection, run_id: &str) -> AppResult<Option<RunStatus>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT status FROM runs WHERE id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    text.map(|s| RunStatus::parse(&s).ok_or_else(|| corrupt("status", run_id, &s)))
        .transpose()
}

/// Record the latest published `seq` (monotonic: `last_seq` never decreases)
/// and, when given, the current stage list. Only non-terminal runs change.
/// Returns whether the row changed.
pub fn update_run_progress(
    conn: &Connection,
    run_id: &str,
    seq: u64,
    stages: Option<&[StageProgress]>,
    at: &str,
) -> AppResult<bool> {
    let stages_json = stages.map(stages_to_json).transpose()?;
    let changed = conn
        .execute(
            &format!(
                "UPDATE runs SET last_seq = MAX(last_seq, ?2),
                                 stages_json = COALESCE(?3, stages_json),
                                 updated_at = ?4
                 WHERE id = ?1 AND status IN {OPEN_RUN_STATUSES}"
            ),
            params![run_id, to_i64(seq), stages_json, at],
        )
        .map_err(map_sqlite)?;
    Ok(changed == 1)
}

/// Cancellation_Request (Req 5.3, 5.11): Queued/Active → Canceling via a
/// conditional UPDATE. Any other status, or a missing run, modifies nothing.
pub fn request_cancel(conn: &Connection, run_id: &str, at: &str) -> AppResult<CancelOutcome> {
    in_savepoint(conn, |c| {
        let Some(previous) = run_status(c, run_id)? else {
            return Ok(CancelOutcome::NotFound);
        };
        if !matches!(previous, RunStatus::Queued | RunStatus::Active) {
            return Ok(CancelOutcome::NotAllowed(previous));
        }
        let changed = c
            .execute(
                "UPDATE runs SET status = 'canceling',
                                 cancel_requested_at = COALESCE(cancel_requested_at, ?2),
                                 updated_at = ?2
                 WHERE id = ?1 AND status IN ('queued','active')",
                params![run_id, at],
            )
            .map_err(map_sqlite)?;
        if changed == 1 {
            Ok(CancelOutcome::Accepted { previous })
        } else {
            // Lost a race with another writer: report what is there now.
            Ok(match run_status(c, run_id)? {
                Some(s) => CancelOutcome::NotAllowed(s),
                None => CancelOutcome::NotFound,
            })
        }
    })
}

/// Write the terminal state (Req 1.4, 1.8, 4.9): status, `finished_at`,
/// `duration_ms = finished_at − started_at`, `error_reason` (Error only),
/// `stages_json`, `summary_json`, and `owns_runner_lock = 0`. Only a
/// non-terminal run with zero Queued and zero Active postings (Req 3.12) can be
/// finalized. Returns the recorded `duration_ms`.
pub fn finalize_run(conn: &Connection, run_id: &str, terminal: &TerminalRecord) -> AppResult<u64> {
    if !terminal.status.is_terminal() {
        return Err(AppError::Message(format!(
            "finalize_run requires a terminal status, got {}",
            terminal.status.as_str()
        )));
    }
    let error_reason = if terminal.status == RunStatus::Error {
        match terminal.error_reason.as_deref().filter(|r| !is_blank(r)) {
            Some(r) => Some(r.to_string()),
            None => {
                return Err(AppError::from(
                    "finalize_run: an Error run requires a non-empty reason",
                ))
            }
        }
    } else {
        None
    };

    in_savepoint(conn, |c| {
        let run = load_run_row(c, run_id)?
            .ok_or_else(|| AppError::Message(format!("run_not_found:{run_id}")))?;
        if run.status.is_terminal() {
            return Err(AppError::Message(format!(
                "run_already_terminal:{}",
                run.status.as_str()
            )));
        }
        let open: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM run_postings
                 WHERE run_id = ?1 AND status IN ('queued','active')",
                params![run_id],
                |r| r.get(0),
            )
            .map_err(map_sqlite)?;
        if open > 0 {
            return Err(AppError::Message(format!("run_postings_unsettled:{open}")));
        }

        let duration_ms = duration_ms_between(&run.started_at, &terminal.finished_at)?;
        let summary_json = terminal
            .summary
            .clone()
            .map(|mut s| {
                s.status = terminal.status;
                s.started_at = run.started_at.clone();
                s.finished_at = terminal.finished_at.clone();
                s.duration_ms = BoundedCount::new(duration_ms);
                serde_json::to_string(&s)
            })
            .transpose()
            .map_err(|e| AppError::Message(format!("summary_json: {e}")))?;

        let changed = c
            .execute(
                &format!(
                    "UPDATE runs SET status = ?2, finished_at = ?3, duration_ms = ?4,
                                     error_reason = ?5, stages_json = ?6, summary_json = ?7,
                                     owns_runner_lock = 0, last_seq = MAX(last_seq, ?8),
                                     updated_at = ?3
                     WHERE id = ?1 AND status IN {OPEN_RUN_STATUSES}"
                ),
                params![
                    run_id,
                    terminal.status.as_str(),
                    terminal.finished_at,
                    to_i64(duration_ms),
                    error_reason,
                    stages_to_json(&terminal.stages)?,
                    summary_json,
                    to_i64(terminal.seq),
                ],
            )
            .map_err(map_sqlite)?;
        if changed != 1 {
            return Err(AppError::Message(format!("run_not_open:{run_id}")));
        }
        Ok(duration_ms)
    })
}

/// Mark a terminal run dismissed. Non-terminal runs cannot be dismissed.
pub fn dismiss_run(conn: &Connection, run_id: &str, at: &str) -> AppResult<DismissOutcome> {
    in_savepoint(conn, |c| {
        let Some(status) = run_status(c, run_id)? else {
            return Ok(DismissOutcome::NotFound);
        };
        if !status.is_terminal() {
            return Ok(DismissOutcome::NotAllowed(status));
        }
        c.execute(
            &format!(
                "UPDATE runs SET dismissed_at = COALESCE(dismissed_at, ?2)
                 WHERE id = ?1 AND status IN {TERMINAL_RUN_STATUSES}"
            ),
            params![run_id, at],
        )
        .map_err(map_sqlite)?;
        Ok(DismissOutcome::Dismissed)
    })
}

// ---------------------------------------------------------------------------
// Postings
// ---------------------------------------------------------------------------

/// Posting start CAS (design decision 3, Req 5.4): Queued → Active only while
/// the run itself is Queued or Active. Once Canceling is committed (by any
/// process) this returns `false` and nothing changes.
pub fn try_mark_posting_active(
    conn: &Connection,
    run_id: &str,
    job_id: &str,
    at: &str,
) -> AppResult<bool> {
    let changed = conn
        .execute(
            "UPDATE run_postings SET status = 'active', attempted_at = ?3
             WHERE run_id = ?1 AND job_id = ?2 AND status = 'queued'
               AND (SELECT status FROM runs WHERE id = ?1) IN ('queued','active')",
            params![run_id, job_id, at],
        )
        .map_err(map_sqlite)?;
    Ok(changed == 1)
}

/// Active → Completed with the authoritative Posting_State and reason
/// (design "Finalize posting", step 5). Call inside the finalize transaction
/// after `apply_classified_check`. Errors, so the caller rolls back, when the
/// posting is not Active or the reason is empty.
pub fn finalize_posting(
    conn: &Connection,
    run_id: &str,
    result: &FinalizedPosting,
) -> AppResult<()> {
    if is_blank(&result.reason) {
        return Err(AppError::from("finalize_posting: reason must be non-empty"));
    }
    let changed = conn
        .execute(
            "UPDATE run_postings SET status = 'completed', posting_state = ?3, reason_code = ?4,
                                     reason = ?5, failure_category = ?6, attempted_at = ?7,
                                     finished_at = ?8
             WHERE run_id = ?1 AND job_id = ?2 AND status = 'active'",
            params![
                run_id,
                result.job_id,
                result.posting_state.as_str(),
                result.reason_code,
                result.reason,
                result.failure_category,
                result.attempted_at,
                result.finished_at,
            ],
        )
        .map_err(map_sqlite)?;
    if changed != 1 {
        return Err(AppError::Message(format!(
            "posting_not_active:{}",
            result.job_id
        )));
    }
    Ok(())
}

/// Queued/Active → Error with a failure category and non-empty reason
/// (Req 3.4, 7.10). Errors when the posting is already terminal, so an Error
/// never overwrites a Completed result and vice versa.
pub fn mark_posting_error(
    conn: &Connection,
    run_id: &str,
    job_id: &str,
    category: &str,
    reason: &str,
    at: &str,
) -> AppResult<()> {
    if is_blank(reason) {
        return Err(AppError::from(
            "mark_posting_error: reason must be non-empty",
        ));
    }
    let changed = conn
        .execute(
            "UPDATE run_postings SET status = 'error', posting_state = NULL,
                                     failure_category = ?3, reason = ?4, finished_at = ?5
             WHERE run_id = ?1 AND job_id = ?2 AND status IN ('queued','active')",
            params![run_id, job_id, category, reason, at],
        )
        .map_err(map_sqlite)?;
    if changed != 1 {
        return Err(AppError::Message(format!("posting_not_open:{job_id}")));
    }
    Ok(())
}

/// Cancel drain (Req 5.5): every remaining Queued posting → Canceled. Returns
/// the affected job ids in ordinal order so the coordinator can mirror them
/// into its ledger.
pub fn cancel_remaining_queued(
    conn: &Connection,
    run_id: &str,
    at: &str,
) -> AppResult<Vec<String>> {
    in_savepoint(conn, |c| {
        let ids = select_job_ids(c, run_id, "queued")?;
        c.execute(
            "UPDATE run_postings SET status = 'canceled', finished_at = ?2
             WHERE run_id = ?1 AND status = 'queued'",
            params![run_id, at],
        )
        .map_err(map_sqlite)?;
        Ok(ids)
    })
}

/// Run-level failure close-out (Req 1.7): Queued → Canceled and Active →
/// Error (`run_aborted`, `reason`). Finalized rows are untouched.
pub fn abort_open_postings(
    conn: &Connection,
    run_id: &str,
    reason: &str,
    at: &str,
) -> AppResult<AbortedPostings> {
    let reason = if is_blank(reason) {
        "Run stopped before this check finished"
    } else {
        reason
    };
    in_savepoint(conn, |c| {
        let errored = select_job_ids(c, run_id, "active")?;
        c.execute(
            "UPDATE run_postings SET status = 'error', posting_state = NULL,
                                     failure_category = ?2, reason = ?3, finished_at = ?4
             WHERE run_id = ?1 AND status = 'active'",
            params![run_id, RUN_ABORTED_CATEGORY, reason, at],
        )
        .map_err(map_sqlite)?;
        let canceled = cancel_remaining_queued(c, run_id, at)?;
        Ok(AbortedPostings { canceled, errored })
    })
}

fn select_job_ids(conn: &Connection, run_id: &str, status: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT job_id FROM run_postings WHERE run_id = ?1 AND status = ?2 ORDER BY ordinal, job_id")
        .map_err(map_sqlite)?;
    let ids = stmt
        .query_map(params![run_id, status], |r| r.get(0))
        .map_err(map_sqlite)?
        .collect::<Result<Vec<String>, _>>()
        .map_err(map_sqlite)?;
    Ok(ids)
}

/// Close out a single run that was interrupted (e.g., due to panic/abort).
/// This is a variant of [`abort_open_postings`] that also updates the run
/// record to Error with `runner_interrupted`.
pub fn abort_run(conn: &Connection, run_id: &str, at: &str) -> AppResult<()> {
    abort_run_with_reason(conn, run_id, RUNNER_INTERRUPTED, at)
}

/// Close one run as Error, but only if it is still open. This is deliberately
/// one savepoint: a failure while rebuilding the summary cannot leave some
/// postings closed and the run open. Terminal runs, including their posting
/// rows and evidence, are immutable and are treated as already settled.
pub fn abort_run_with_reason(
    conn: &Connection,
    run_id: &str,
    reason: &str,
    at: &str,
) -> AppResult<()> {
    let reason = if is_blank(reason) {
        "run stopped before completion"
    } else {
        reason
    };
    in_savepoint(conn, |c| {
        let Some(run) = load_run_row(c, run_id)? else {
            return Err(AppError::Message(format!("run not found: {run_id}")));
        };
        if run.status.is_terminal() {
            return Ok(());
        }

        abort_open_postings(c, run_id, reason, at)?;
        let run = load_run_row(c, run_id)?
            .ok_or_else(|| AppError::Message(format!("run not found: {run_id}")))?;
        let mut stages = run.stages.clone();
        settle_stages_for_terminal(&mut stages, Some(reason));
        let duration_ms = duration_ms_between(&run.started_at, at).unwrap_or(0);
        let rows = load_posting_rows(c, run_id)?;
        let summary = summary_from_rows(&run, RunStatus::Error, &rows, &stages, at, duration_ms);
        let summary_json = serde_json::to_string(&summary)
            .map_err(|e| AppError::Message(format!("summary_json: {e}")))?;
        let changed = c
            .execute(
                &format!(
                    "UPDATE runs SET status = 'error', error_reason = ?2, finished_at = ?3,
                                     duration_ms = ?4, stages_json = ?5, summary_json = ?6,
                                     owns_runner_lock = 0, updated_at = ?3
                     WHERE id = ?1 AND status IN {OPEN_RUN_STATUSES}"
                ),
                params![
                    run_id,
                    reason,
                    at,
                    to_i64(duration_ms),
                    stages_to_json(&stages)?,
                    summary_json
                ],
            )
            .map_err(map_sqlite)?;
        if changed != 1 {
            return Err(AppError::Message(format!("run_not_open:{run_id}")));
        }
        Ok(())
    })
}
// ---------------------------------------------------------------------------

/// Close out every non-terminal run as Error `runner_interrupted`. Only call
/// while this process holds the runner flock, which proves no live process
/// owns those runs. Postings are closed out as in [`abort_open_postings`],
/// unstarted stages become `skipped`, an in-progress stage becomes `failed`,
/// a summary is rebuilt from rows, and `owns_runner_lock` is cleared (also on
/// any terminal row that still claims it). Returns the number of runs recovered.
pub fn recover_orphaned_runs(conn: &Connection, at: &str) -> AppResult<usize> {
    in_savepoint(conn, |c| {
        let ids: Vec<String> = {
            let mut stmt = c
                .prepare(&format!("SELECT id FROM runs WHERE status IN {OPEN_RUN_STATUSES} ORDER BY started_at, id"))
                .map_err(map_sqlite)?;
            let rows = stmt
                .query_map([], |r| r.get(0))
                .map_err(map_sqlite)?
                .collect::<Result<Vec<String>, _>>()
                .map_err(map_sqlite)?;
            rows
        };

        for id in &ids {
            abort_open_postings(
                c,
                id,
                "Runner was interrupted before this check finished",
                at,
            )?;
            let Some(run) = load_run_row(c, id)? else {
                continue;
            };
            let mut stages = run.stages.clone();
            settle_stages_for_terminal(&mut stages, Some(RUNNER_INTERRUPTED));
            // Corrupt timestamps must not block recovery.
            let duration_ms = duration_ms_between(&run.started_at, at).unwrap_or(0);
            let rows = load_posting_rows(c, id)?;
            let summary =
                summary_from_rows(&run, RunStatus::Error, &rows, &stages, at, duration_ms);
            let summary_json = serde_json::to_string(&summary)
                .map_err(|e| AppError::Message(format!("summary_json: {e}")))?;
            c.execute(
                &format!(
                    "UPDATE runs SET status = 'error', error_reason = ?2, finished_at = ?3,
                                     duration_ms = ?4, stages_json = ?5, summary_json = ?6,
                                     owns_runner_lock = 0, updated_at = ?3
                     WHERE id = ?1 AND status IN {OPEN_RUN_STATUSES}"
                ),
                params![
                    id,
                    RUNNER_INTERRUPTED,
                    at,
                    to_i64(duration_ms),
                    stages_to_json(&stages)?,
                    summary_json
                ],
            )
            .map_err(map_sqlite)?;
        }

        c.execute(
            &format!("UPDATE runs SET owns_runner_lock = 0 WHERE owns_runner_lock = 1 AND status IN {TERMINAL_RUN_STATUSES}"),
            [],
        )
        .map_err(map_sqlite)?;
        Ok(ids.len())
    })
}

// ---------------------------------------------------------------------------
// Snapshots
// ---------------------------------------------------------------------------

/// Rebuild the full [`RunSnapshot`] from SQLite (design decision 1).
///
/// - `live` cannot be known from the DB: pass `true` when this process owns
///   the run and will publish events for it (it is registered in `RunRegistry`).
/// - `now` (RFC 3339) is used for `elapsed_ms` of non-terminal runs; terminal
///   runs report the persisted `duration_ms`.
/// - Counters and the posting list come from `run_postings`; `stages` (Jobs_Cycle
///   only) from `stages_json`; `summary` (terminal only) from `summary_json`,
///   rebuilt from rows when absent; legacy fields from [`legacy_progress`].
/// - authoritative evidence is loaded from `posting_check_evidence` for completed postings.
pub fn load_snapshot(
    conn: &Connection,
    run_id: &str,
    live: bool,
    now: &str,
) -> AppResult<Option<RunSnapshot>> {
    let Some(run) = load_run_row(conn, run_id)? else {
        return Ok(None);
    };
    let rows = load_posting_rows(conn, run_id)?;
    let postings = rows
        .iter()
        .map(|row| {
            let evidence = (row.status == PostingStatus::Completed)
                .then(|| load_authoritative_evidence(conn, run_id, &row.job_id))
                .transpose()?
                .flatten()
                .map(|stored| {
                    CheckEvidence::from_json(&stored.record.evidence_json)
                        .map(crate::runs::progress::EvidenceView::from)
                        .map_err(|error| AppError::from(error.to_string()))
                })
                .transpose()?;
            Ok(row.to_progress(evidence))
        })
        .collect::<AppResult<Vec<_>>>()?;
    Ok(Some(build_snapshot(run, &rows, postings, live, now)))
}

fn build_snapshot(
    run: RunRow,
    rows: &[PostingRow],
    postings: Vec<PostingProgress>,
    live: bool,
    now: &str,
) -> RunSnapshot {
    let counts = counts_of(rows);
    let done = run.status.is_terminal();
    let error_reason = if run.status == RunStatus::Error {
        Some(
            run.error_reason
                .clone()
                .filter(|r| !is_blank(r))
                .unwrap_or_else(|| "unknown_error".into()),
        )
    } else {
        None
    };
    let legacy = legacy_progress(
        run.run_type,
        run.status,
        &run.stages,
        &counts,
        error_reason.as_deref(),
    );

    let elapsed_ms = if done {
        run.duration_ms.unwrap_or_else(|| {
            run.finished_at
                .as_deref()
                .and_then(|f| duration_ms_between(&run.started_at, f).ok())
                .unwrap_or(0)
        })
    } else {
        duration_ms_between(&run.started_at, now).unwrap_or(0)
    };

    let summary = done.then(|| {
        run.summary_json
            .as_deref()
            .and_then(|json| match serde_json::from_str::<RunSummary>(json) {
                Ok(s) => Some(s),
                Err(err) => {
                    log::warn!("[runs] unreadable summary_json for {}: {err}", run.id);
                    None
                }
            })
            .unwrap_or_else(|| {
                let finished_at = run
                    .finished_at
                    .clone()
                    .unwrap_or_else(|| run.started_at.clone());
                summary_from_rows(
                    &run,
                    run.status,
                    rows,
                    &run.stages,
                    &finished_at,
                    elapsed_ms,
                )
            })
    });

    let mut snapshot = RunSnapshot {
        version: PROGRESS_CONTRACT_VERSION,
        run_id: run.id.clone(),
        run_type: run.run_type,
        run_status: run.status,
        seq: BoundedCount::new(run.last_seq),
        stage: legacy.stage,
        message: legacy.message,
        current: legacy.current,
        total: legacy.total,
        done,
        started_at: run.started_at.clone(),
        elapsed_ms: BoundedCount::new(elapsed_ms),
        stages: (run.run_type == RunType::JobsCycle).then(|| run.stages.clone()),
        posting_counts: counts,
        posting_total: BoundedCount::new(counts.total()),
        error_reason,
        summary,
        postings,
        live,
        source_run_id: run.source_run_id.clone(),
        dismissed: run.dismissed,
    };
    snapshot.enforce_bounds();
    snapshot
}

/// The run the Run_Monitor should display (Req 2.3, 2.5): the most recently
/// started non-terminal run, else the most recently finished terminal run that
/// has not been dismissed. `is_live(run_id)` supplies [`RunSnapshot::live`].
pub fn load_current(
    conn: &Connection,
    is_live: impl Fn(&str) -> bool,
    now: &str,
) -> AppResult<Option<RunSnapshot>> {
    let open: Option<String> = conn
        .query_row(
            &format!("SELECT id FROM runs WHERE status IN {OPEN_RUN_STATUSES} ORDER BY started_at DESC, rowid DESC LIMIT 1"),
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    let id = match open {
        Some(id) => Some(id),
        None => conn
            .query_row(
                &format!(
                    "SELECT id FROM runs WHERE status IN {TERMINAL_RUN_STATUSES} AND dismissed_at IS NULL
                     ORDER BY COALESCE(finished_at, started_at) DESC, started_at DESC, rowid DESC LIMIT 1"
                ),
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sqlite)?,
    };
    match id {
        Some(id) => load_snapshot(conn, &id, is_live(&id), now),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

/// Insert one `posting_check_evidence` row and return its uuid. `run_id` is
/// `None` for `check_job_posting`. At most one authoritative row per
/// `(run_id, job_id)` is enforced by `pce_one_authoritative_uidx`.
pub fn insert_evidence(
    conn: &Connection,
    run_id: Option<&str>,
    job_id: &str,
    kind: EvidenceKind,
    record: &EvidenceRecord,
) -> AppResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO posting_check_evidence (id, run_id, job_id, kind, attempted_at, posting_state,
             reason_code, reason, evidence_version, evidence_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id,
            run_id,
            job_id,
            kind.as_str(),
            record.attempted_at,
            record.posting_state.as_str(),
            record.reason_code,
            record.reason,
            i64::from(record.evidence_version),
            record.evidence_json,
            record.created_at,
        ],
    )
    .map_err(map_sqlite)?;
    Ok(id)
}

/// Store a late network result as Supplementary_Check_Evidence (Req 8.8).
/// Touches only `posting_check_evidence`; the authoritative row, the posting
/// row, and counters are never modified (Req 8.9).
pub fn insert_supplementary_evidence(
    conn: &Connection,
    run_id: &str,
    job_id: &str,
    record: &EvidenceRecord,
) -> AppResult<String> {
    insert_evidence(
        conn,
        Some(run_id),
        job_id,
        EvidenceKind::Supplementary,
        record,
    )
}

/// The authoritative evidence row for one posting of a run, if any.
pub fn load_authoritative_evidence(
    conn: &Connection,
    run_id: &str,
    job_id: &str,
) -> AppResult<Option<StoredEvidence>> {
    let raw = conn
        .query_row(
            "SELECT id, run_id, job_id, kind, attempted_at, posting_state, reason_code, reason,
                    evidence_version, evidence_json, created_at
             FROM posting_check_evidence
             WHERE run_id = ?1 AND job_id = ?2 AND kind = 'authoritative'",
            params![run_id, job_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, i64>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, String>(10)?,
                ))
            },
        )
        .optional()
        .map_err(map_sqlite)?;
    let Some((
        id,
        run_id,
        job_id,
        kind,
        attempted_at,
        state,
        reason_code,
        reason,
        version,
        json,
        created_at,
    )) = raw
    else {
        return Ok(None);
    };
    let kind = EvidenceKind::parse(&kind).ok_or_else(|| corrupt("evidence kind", &id, &kind))?;
    let posting_state = PostingState::parse(&state)
        .ok_or_else(|| corrupt("evidence posting_state", &id, &state))?;
    Ok(Some(StoredEvidence {
        id,
        run_id,
        job_id,
        kind,
        record: EvidenceRecord {
            attempted_at,
            posting_state,
            reason_code,
            reason,
            evidence_version: u32::try_from(version).unwrap_or(0),
            evidence_json: json,
            created_at,
        },
    }))
}

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

/// Best-effort retention (design "Retention", Req 7.7):
/// - Keep the `keep_runs` most recently finished terminal runs; older terminal
///   runs are deleted with their `run_postings`. Non-terminal runs are never
///   pruned. Because `runs.source_run_id` and `run_postings.run_id` reference
///   `runs(id)`, references to pruned runs are cleared and their postings are
///   deleted before the run rows. Evidence rows have no FK and follow their
///   own per-job retention below.
/// - Per job, keep the `keep_evidence_per_job` newest authoritative rows plus
///   the latest conclusive (non-`unknown`) authoritative row, using the same
///   ordering as the "preceding conclusive state" query.
/// - Delete supplementary rows created more than 30 days before `now`.
pub fn prune_history(
    conn: &Connection,
    keep_runs: usize,
    keep_evidence_per_job: usize,
    now: &str,
) -> AppResult<PruneReport> {
    DateTime::parse_from_rfc3339(now)
        .map_err(|e| AppError::Message(format!("invalid timestamp {now:?}: {e}")))?;
    in_savepoint(conn, |c| {
        let mut report = PruneReport::default();

        let pruned: Vec<String> = {
            let mut stmt = c
                .prepare(&format!(
                    "SELECT id FROM runs WHERE status IN {TERMINAL_RUN_STATUSES}
                     ORDER BY COALESCE(finished_at, started_at) DESC, started_at DESC, rowid DESC
                     LIMIT -1 OFFSET ?1"
                ))
                .map_err(map_sqlite)?;
            let rows = stmt
                .query_map(params![to_i64(keep_runs as u64)], |r| r.get(0))
                .map_err(map_sqlite)?
                .collect::<Result<Vec<String>, _>>()
                .map_err(map_sqlite)?;
            rows
        };
        for id in &pruned {
            c.execute(
                "UPDATE runs SET source_run_id = NULL WHERE source_run_id = ?1",
                params![id],
            )
            .map_err(map_sqlite)?;
        }
        for id in &pruned {
            report.run_postings += c
                .execute("DELETE FROM run_postings WHERE run_id = ?1", params![id])
                .map_err(map_sqlite)?;
            report.runs += c
                .execute("DELETE FROM runs WHERE id = ?1", params![id])
                .map_err(map_sqlite)?;
        }

        report.authoritative_evidence = c
            .execute(
                "DELETE FROM posting_check_evidence WHERE id IN (
                   SELECT id FROM (
                     SELECT id, posting_state,
                       ROW_NUMBER() OVER (PARTITION BY job_id
                         ORDER BY attempted_at DESC, created_at DESC, id DESC) AS rn,
                       ROW_NUMBER() OVER (PARTITION BY job_id, posting_state != 'unknown'
                         ORDER BY attempted_at DESC, created_at DESC, id DESC) AS conclusive_rn
                     FROM posting_check_evidence WHERE kind = 'authoritative'
                   )
                   WHERE rn > ?1 AND NOT (posting_state != 'unknown' AND conclusive_rn = 1)
                 )",
                params![to_i64(keep_evidence_per_job as u64)],
            )
            .map_err(map_sqlite)?;

        report.supplementary_evidence = c
            .execute(
                "DELETE FROM posting_check_evidence
                 WHERE kind = 'supplementary' AND julianday(created_at) < julianday(?1) - ?2",
                params![now, SUPPLEMENTARY_RETENTION_DAYS],
            )
            .map_err(map_sqlite)?;

        Ok(report)
    })
}

/// Preceding conclusive Posting_State for a job (Req 7.7), used by tests here
/// and by `jobs::posting_check::persist::preceding_conclusive_state`.
pub fn latest_conclusive_evidence_state(
    conn: &Connection,
    job_id: &str,
) -> AppResult<Option<PostingState>> {
    let state: Option<String> = conn
        .query_row(
            "SELECT posting_state FROM posting_check_evidence
             WHERE job_id = ?1 AND kind = 'authoritative' AND posting_state != 'unknown'
             ORDER BY attempted_at DESC, created_at DESC, id DESC LIMIT 1",
            params![job_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    state
        .map(|s| {
            PostingState::parse(&s).ok_or_else(|| corrupt("evidence posting_state", job_id, &s))
        })
        .transpose()
}

/// Map job id → `state_at_start` for a run (input to `build_run_summary`).
pub fn states_at_start(conn: &Connection, run_id: &str) -> AppResult<HashMap<String, String>> {
    let mut stmt = conn
        .prepare("SELECT job_id, state_at_start FROM run_postings WHERE run_id = ?1")
        .map_err(map_sqlite)?;
    let map = stmt
        .query_map(params![run_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(map_sqlite)?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(map_sqlite)?;
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;

    const T0: &str = "2026-01-01T00:00:00.000Z";
    const T1: &str = "2026-01-01T00:00:01.000Z";
    const T2: &str = "2026-01-01T00:00:02.500Z";
    const T3: &str = "2026-01-01T00:00:04.000Z";

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn posting(id: &str, state: &str) -> JobIdentityWithState {
        JobIdentityWithState {
            identity: JobIdentity {
                job_id: id.into(),
                title: format!("Engineer {id}"),
                company_name: "Acme".into(),
                posting_url: format!("https://example.com/jobs/{id}"),
            },
            state_at_start: state.into(),
        }
    }

    fn new_run(id: &str, run_type: RunType, at: &str) -> NewRun {
        let stages = match run_type {
            RunType::JobsCycle => StageName::ORDER
                .into_iter()
                .map(StageProgress::not_started)
                .collect(),
            RunType::PostingCheck => vec![],
        };
        NewRun {
            id: RunId::from_existing(id),
            run_type,
            trigger: Trigger::Desktop,
            source_run_id: None,
            owner_pid: 42,
            accepted_at: at.into(),
            stages,
        }
    }

    fn accept(conn: &Connection, id: &str, jobs: &[&str], at: &str) {
        let postings: Vec<_> = jobs.iter().map(|j| posting(j, "unknown")).collect();
        insert_accepted_run(conn, &new_run(id, RunType::PostingCheck, at), &postings).unwrap();
    }

    fn terminal(status: RunStatus, at: &str) -> TerminalRecord {
        TerminalRecord {
            status,
            finished_at: at.into(),
            error_reason: (status == RunStatus::Error).then(|| "boom".to_string()),
            stages: vec![],
            summary: None,
            seq: 0,
        }
    }

    /// Cancel remaining queued postings and finalize.
    fn finish(conn: &Connection, id: &str, status: RunStatus, at: &str) {
        cancel_remaining_queued(conn, id, at).unwrap();
        finalize_run(conn, id, &terminal(status, at)).unwrap();
    }

    fn completed(job_id: &str, state: PostingState) -> FinalizedPosting {
        FinalizedPosting {
            job_id: job_id.into(),
            posting_state: state,
            reason_code: "listed_open".into(),
            reason: "Open: listed on Greenhouse board".into(),
            failure_category: None,
            attempted_at: T1.into(),
            finished_at: T2.into(),
        }
    }

    fn posting_status(conn: &Connection, run_id: &str, job_id: &str) -> String {
        conn.query_row(
            "SELECT status FROM run_postings WHERE run_id = ?1 AND job_id = ?2",
            params![run_id, job_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn run_row_digest(conn: &Connection, run_id: &str) -> String {
        conn.query_row(
            "SELECT status || '|' || updated_at || '|' || IFNULL(cancel_requested_at, '-') || '|' ||
                    owns_runner_lock || '|' || IFNULL(finished_at, '-') FROM runs WHERE id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn evidence(state: PostingState, attempted_at: &str) -> EvidenceRecord {
        EvidenceRecord {
            attempted_at: attempted_at.into(),
            posting_state: state,
            reason_code: "code".into(),
            reason: "reason".into(),
            evidence_version: 1,
            evidence_json: format!(
                r#"{{"attemptedAt":"{attempted_at}","requestedUrl":"https://example.com/jobs/a"}}"#
            ),
            created_at: attempted_at.into(),
        }
    }

    #[test]
    fn sql_status_lists_match_enum_terminality() {
        let open: Vec<_> = RunStatus::ALL
            .into_iter()
            .filter(|s| !s.is_terminal())
            .collect();
        let term: Vec<_> = RunStatus::ALL
            .into_iter()
            .filter(|s| s.is_terminal())
            .collect();
        assert_eq!(status_sql_list(&open), OPEN_RUN_STATUSES);
        assert_eq!(status_sql_list(&term), TERMINAL_RUN_STATUSES);
    }

    #[test]
    fn snapshot_round_trip_through_lifecycle() {
        let conn = db();
        accept(&conn, "r1", &["a", "b", "c"], T0);

        let s = load_snapshot(&conn, "r1", true, T1).unwrap().unwrap();
        assert_eq!(s.run_status, RunStatus::Queued);
        assert_eq!(s.seq.get(), 1);
        assert_eq!(s.posting_total.get(), 3);
        assert_eq!(
            s.posting_counts,
            PostingCounts {
                queued: 3,
                ..Default::default()
            }
        );
        assert_eq!(
            s.postings
                .iter()
                .map(|p| p.job_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(s.postings[1].title, "Engineer b");
        assert_eq!(s.elapsed_ms.get(), 1000);
        assert!(s.live && !s.done && !s.dismissed && s.summary.is_none() && s.stages.is_none());

        assert!(cas_run_status(&conn, "r1", &[RunStatus::Queued], RunStatus::Active, T1).unwrap());
        assert!(!cas_run_status(&conn, "r1", &[RunStatus::Queued], RunStatus::Active, T1).unwrap());
        assert!(
            cas_run_status(&conn, "r1", &[RunStatus::Active], RunStatus::Completed, T1).is_err()
        );

        assert!(try_mark_posting_active(&conn, "r1", "a", T1).unwrap());
        assert!(
            !try_mark_posting_active(&conn, "r1", "a", T1).unwrap(),
            "no double start"
        );
        assert!(
            finalize_posting(&conn, "r1", &completed("b", PostingState::Active)).is_err(),
            "b is queued"
        );
        finalize_posting(&conn, "r1", &completed("a", PostingState::Active)).unwrap();
        assert!(
            mark_posting_error(&conn, "r1", "a", "internal", "late", T2).is_err(),
            "no Error after Completed"
        );
        assert!(try_mark_posting_active(&conn, "r1", "b", T1).unwrap());
        mark_posting_error(&conn, "r1", "b", "persistence", "db locked", T2).unwrap();
        assert!(update_run_progress(&conn, "r1", 6, None, T2).unwrap());
        assert!(update_run_progress(&conn, "r1", 4, None, T2).unwrap());

        let s = load_snapshot(&conn, "r1", false, T2).unwrap().unwrap();
        assert_eq!(s.run_status, RunStatus::Active);
        assert_eq!(s.seq.get(), 6, "last_seq never decreases");
        assert_eq!(
            s.posting_counts,
            PostingCounts {
                queued: 1,
                completed: 1,
                error: 1,
                ..Default::default()
            }
        );
        assert_eq!(
            (s.stage, s.current.get(), s.total.get()),
            (LegacyStage::Postings, 2, 3)
        );
        assert_eq!(s.message, "Checked 2 of 3 postings");
        let a = &s.postings[0];
        assert_eq!(a.posting_state, Some(PostingState::Active));
        assert_eq!(
            a.reason.as_deref(),
            Some("Open: listed on Greenhouse board")
        );
        let b = &s.postings[1];
        assert_eq!((b.status, b.posting_state), (PostingStatus::Error, None));
        assert_eq!(b.failure_category.as_deref(), Some("persistence"));
        assert_eq!(b.reason.as_deref(), Some("db locked"));
        assert!(s.postings[2].reason.is_none());
        assert!(!s.live);

        assert_eq!(cancel_remaining_queued(&conn, "r1", T3).unwrap(), ["c"]);
        let d = finalize_run(&conn, "r1", &terminal(RunStatus::CompletedWithErrors, T3)).unwrap();
        assert_eq!(d, 4000);

        let s = load_snapshot(&conn, "r1", false, "2026-01-01T01:00:00.000Z")
            .unwrap()
            .unwrap();
        assert!(s.done);
        assert_eq!(
            s.elapsed_ms.get(),
            4000,
            "terminal elapsed is the recorded duration"
        );
        assert_eq!(s.message, "Posting check completed with errors");
        let summary = s.summary.as_ref().unwrap();
        assert_eq!(summary.status, RunStatus::CompletedWithErrors);
        assert_eq!(
            summary.posting_outcomes,
            PostingOutcomes {
                active: 1,
                error: 1,
                canceled: 1,
                ..Default::default()
            }
        );
        assert_eq!(summary.state_changes.get(), 1);
        assert_eq!(summary.duration_ms.get(), 4000);
        assert!(s.error_reason.is_none());
        assert!(s.is_within_bounds());

        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("null"));
        let back: RunSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn jobs_cycle_snapshot_reports_stages_and_cycle_stage() {
        let conn = db();
        let run = new_run("jc", RunType::JobsCycle, T0);
        insert_accepted_run(&conn, &run, &[posting("a", "active")]).unwrap();
        let mut stages = run.stages.clone();
        stages[0].outcome = StageOutcome::Succeeded;
        stages[1] = StageProgress {
            outcome: StageOutcome::InProgress,
            current: BoundedCount::new(1),
            total: BoundedCount::new(3),
            ..stages[1].clone()
        };
        update_run_progress(&conn, "jc", 5, Some(&stages), T1).unwrap();
        let s = load_snapshot(&conn, "jc", true, T1).unwrap().unwrap();
        assert_eq!(s.stages.as_ref().unwrap().len(), 4);
        assert_eq!(
            (s.stage, s.current.get(), s.total.get()),
            (LegacyStage::Watches, 1, 3)
        );

        finish(&conn, "jc", RunStatus::Canceled, T2);
        let s = load_snapshot(&conn, "jc", true, T2).unwrap().unwrap();
        assert_eq!(
            (s.stage, s.current.get(), s.total.get(), s.done),
            (LegacyStage::Cycle, 1, 1, true)
        );
        assert!(
            s.summary.as_ref().unwrap().stages.is_some(),
            "fallback summary carries stages"
        );
    }

    #[test]
    fn try_mark_posting_active_fails_after_request_cancel() {
        let conn = db();
        accept(&conn, "r1", &["a", "b"], T0);
        assert!(cas_run_status(&conn, "r1", &[RunStatus::Queued], RunStatus::Active, T1).unwrap());
        assert!(try_mark_posting_active(&conn, "r1", "a", T1).unwrap());

        assert_eq!(
            request_cancel(&conn, "r1", T2).unwrap(),
            CancelOutcome::Accepted {
                previous: RunStatus::Active
            }
        );
        assert!(!try_mark_posting_active(&conn, "r1", "b", T2).unwrap());
        assert_eq!(posting_status(&conn, "r1", "b"), "queued");
        // The in-flight posting can still finish (Req 5.5).
        finalize_posting(&conn, "r1", &completed("a", PostingState::Inactive)).unwrap();
        assert_eq!(cancel_remaining_queued(&conn, "r1", T3).unwrap(), ["b"]);
        assert_eq!(posting_status(&conn, "r1", "b"), "canceled");
    }

    #[test]
    fn second_lock_owner_is_rejected_without_partial_writes() {
        let conn = db();
        accept(&conn, "r1", &["a"], T0);
        let second = insert_accepted_run(
            &conn,
            &new_run("r2", RunType::PostingCheck, T1),
            &[posting("a", "unknown")],
        );
        assert!(second.is_err());
        let rows: i64 = conn
            .query_row("SELECT (SELECT COUNT(*) FROM runs WHERE id='r2') + (SELECT COUNT(*) FROM run_postings WHERE run_id='r2')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
        // Duplicate job ids also roll back the whole accept.
        finish(&conn, "r1", RunStatus::Completed, T1);
        assert!(insert_accepted_run(
            &conn,
            &new_run("r3", RunType::PostingCheck, T2),
            &[posting("x", "unknown"), posting("x", "unknown")]
        )
        .is_err());
        assert!(load_snapshot(&conn, "r3", false, T2).unwrap().is_none());
    }

    #[test]
    fn request_cancel_on_terminal_or_missing_run_modifies_nothing() {
        let conn = db();
        accept(&conn, "r1", &["a"], T0);
        finish(&conn, "r1", RunStatus::Completed, T1);
        let before = run_row_digest(&conn, "r1");
        assert_eq!(
            request_cancel(&conn, "r1", T2).unwrap(),
            CancelOutcome::NotAllowed(RunStatus::Completed)
        );
        assert_eq!(run_row_digest(&conn, "r1"), before);
        assert_eq!(
            request_cancel(&conn, "nope", T2).unwrap(),
            CancelOutcome::NotFound
        );

        accept(&conn, "r2", &[], T2);
        assert_eq!(
            request_cancel(&conn, "r2", T2).unwrap(),
            CancelOutcome::Accepted {
                previous: RunStatus::Queued
            }
        );
        let before = run_row_digest(&conn, "r2");
        assert_eq!(
            request_cancel(&conn, "r2", T3).unwrap(),
            CancelOutcome::NotAllowed(RunStatus::Canceling)
        );
        assert_eq!(run_row_digest(&conn, "r2"), before);
    }

    #[test]
    fn load_current_prefers_non_terminal_then_latest_undismissed() {
        let conn = db();
        assert!(load_current(&conn, |_| false, T0).unwrap().is_none());

        accept(&conn, "old", &[], T0);
        finish(&conn, "old", RunStatus::Completed, T1);
        accept(&conn, "open", &["a"], T1);
        // Finished more recently than "open" started, but "open" is non-terminal.
        let s = load_current(&conn, |id| id == "open", T2).unwrap().unwrap();
        assert_eq!(s.run_id, "open");
        assert!(s.live);

        finish(&conn, "open", RunStatus::Canceled, T3);
        let s = load_current(&conn, |_| false, T3).unwrap().unwrap();
        assert_eq!(s.run_id, "open");
        assert!(!s.live);

        assert_eq!(
            dismiss_run(&conn, "open", T3).unwrap(),
            DismissOutcome::Dismissed
        );
        assert_eq!(
            load_current(&conn, |_| false, T3).unwrap().unwrap().run_id,
            "old"
        );
        assert_eq!(
            dismiss_run(&conn, "old", T3).unwrap(),
            DismissOutcome::Dismissed
        );
        assert!(load_current(&conn, |_| false, T3).unwrap().is_none());

        accept(&conn, "live", &[], T3);
        assert_eq!(
            dismiss_run(&conn, "live", T3).unwrap(),
            DismissOutcome::NotAllowed(RunStatus::Queued)
        );
        assert_eq!(
            dismiss_run(&conn, "nope", T3).unwrap(),
            DismissOutcome::NotFound
        );
    }

    #[test]
    fn orphan_recovery_closes_out_queued_and_active_postings() {
        let conn = db();
        let run = new_run("orphan", RunType::JobsCycle, T0);
        insert_accepted_run(
            &conn,
            &run,
            &[
                posting("done", "unknown"),
                posting("mid", "unknown"),
                posting("wait", "unknown"),
            ],
        )
        .unwrap();
        cas_run_status(&conn, "orphan", &[RunStatus::Queued], RunStatus::Active, T1).unwrap();
        try_mark_posting_active(&conn, "orphan", "done", T1).unwrap();
        finalize_posting(&conn, "orphan", &completed("done", PostingState::Active)).unwrap();
        try_mark_posting_active(&conn, "orphan", "mid", T1).unwrap();
        let mut stages = run.stages.clone();
        stages[0].outcome = StageOutcome::InProgress;
        update_run_progress(&conn, "orphan", 3, Some(&stages), T1).unwrap();

        assert_eq!(recover_orphaned_runs(&conn, T3).unwrap(), 1);

        let s = load_snapshot(&conn, "orphan", false, T3).unwrap().unwrap();
        assert_eq!(s.run_status, RunStatus::Error);
        assert_eq!(s.error_reason.as_deref(), Some(RUNNER_INTERRUPTED));
        assert_eq!(
            s.posting_counts,
            PostingCounts {
                completed: 1,
                error: 1,
                canceled: 1,
                ..Default::default()
            }
        );
        let by_id: HashMap<_, _> = s.postings.iter().map(|p| (p.job_id.as_str(), p)).collect();
        assert_eq!(
            by_id["done"].posting_state,
            Some(PostingState::Active),
            "finalized row untouched"
        );
        assert_eq!(by_id["mid"].status, PostingStatus::Error);
        assert_eq!(
            by_id["mid"].failure_category.as_deref(),
            Some(RUN_ABORTED_CATEGORY)
        );
        assert_eq!(by_id["wait"].status, PostingStatus::Canceled);
        let stages = s.stages.unwrap();
        assert_eq!(stages[0].outcome, StageOutcome::Failed);
        assert!(stages[1..]
            .iter()
            .all(|st| st.outcome == StageOutcome::Skipped));
        assert_eq!(s.summary.unwrap().posting_outcomes.total(), 3);
        let lock: i64 = conn
            .query_row(
                "SELECT owns_runner_lock FROM runs WHERE id='orphan'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(lock, 0);

        assert_eq!(recover_orphaned_runs(&conn, T3).unwrap(), 0);
        accept(&conn, "next", &[], T3); // lock is free again
    }

    #[test]
    fn abort_run_closes_open_rows_but_leaves_terminal_rows_unchanged() {
        let conn = db();
        accept(&conn, "open", &["done", "active", "queued"], T0);
        cas_run_status(&conn, "open", &[RunStatus::Queued], RunStatus::Active, T1).unwrap();
        try_mark_posting_active(&conn, "open", "done", T1).unwrap();
        finalize_posting(&conn, "open", &completed("done", PostingState::Active)).unwrap();
        try_mark_posting_active(&conn, "open", "active", T1).unwrap();
        let before_terminal = run_row_digest(&conn, "open");

        abort_run_with_reason(&conn, "open", "connection lost", T2).unwrap();
        let snapshot = load_snapshot(&conn, "open", false, T2).unwrap().unwrap();
        assert_eq!(snapshot.run_status, RunStatus::Error);
        assert_eq!(snapshot.error_reason.as_deref(), Some("connection lost"));
        assert_eq!(
            snapshot.posting_counts,
            PostingCounts {
                completed: 1,
                error: 1,
                canceled: 1,
                ..Default::default()
            }
        );
        let done = snapshot
            .postings
            .iter()
            .find(|p| p.job_id == "done")
            .unwrap();
        assert_eq!(done.posting_state, Some(PostingState::Active));
        assert_eq!(
            snapshot
                .postings
                .iter()
                .find(|p| p.job_id == "active")
                .unwrap()
                .failure_category
                .as_deref(),
            Some(RUN_ABORTED_CATEGORY)
        );
        assert_eq!(
            snapshot
                .postings
                .iter()
                .find(|p| p.job_id == "queued")
                .unwrap()
                .status,
            PostingStatus::Canceled
        );

        // A second recovery attempt must not rewrite the terminal run or its
        // finalized posting, even if the caller supplies a new timestamp.
        let terminal_digest = run_row_digest(&conn, "open");
        assert!(abort_run_with_reason(&conn, "open", "different", T3).is_ok());
        assert_eq!(run_row_digest(&conn, "open"), terminal_digest);
        assert_ne!(before_terminal, terminal_digest);
    }

    #[test]
    fn finalize_run_clears_lock_and_records_duration() {
        let conn = db();
        accept(&conn, "r1", &["a"], T0);
        assert!(
            finalize_run(&conn, "r1", &terminal(RunStatus::Completed, T2)).is_err(),
            "posting still queued"
        );
        cancel_remaining_queued(&conn, "r1", T2).unwrap();
        assert!(
            finalize_run(&conn, "r1", &terminal(RunStatus::Active, T2)).is_err(),
            "non-terminal target"
        );
        let no_reason = TerminalRecord {
            error_reason: None,
            ..terminal(RunStatus::Error, T2)
        };
        assert!(
            finalize_run(&conn, "r1", &no_reason).is_err(),
            "Error needs a reason"
        );

        let rec = TerminalRecord {
            seq: 9,
            ..terminal(RunStatus::Error, T2)
        };
        assert_eq!(finalize_run(&conn, "r1", &rec).unwrap(), 2500);
        let (lock, dur, seq, reason): (i64, i64, i64, String) = conn
            .query_row(
                "SELECT owns_runner_lock, duration_ms, last_seq, error_reason FROM runs WHERE id='r1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((lock, dur, seq, reason.as_str()), (0, 2500, 9, "boom"));
        assert!(
            finalize_run(&conn, "r1", &terminal(RunStatus::Completed, T3)).is_err(),
            "already terminal"
        );
        assert!(!update_run_progress(&conn, "r1", 20, None, T3).unwrap());
        let s = load_snapshot(&conn, "r1", false, T3).unwrap().unwrap();
        assert_eq!(s.error_reason.as_deref(), Some("boom"));
        assert_eq!(s.message, "Posting check failed: boom");
        accept(&conn, "r2", &[], T3);
    }

    #[test]
    fn prune_history_respects_fks_and_keeps_latest_conclusive_evidence() {
        let conn = db();
        accept(&conn, "r1", &["a"], T0);
        finish(&conn, "r1", RunStatus::Completed, T0);
        accept(&conn, "r2", &["a"], T1);
        finish(&conn, "r2", RunStatus::Completed, T1);
        let mut retry = new_run("r3", RunType::PostingCheck, T2);
        retry.source_run_id = Some("r1".into());
        insert_accepted_run(&conn, &retry, &[posting("a", "unknown")]).unwrap();
        finish(&conn, "r3", RunStatus::Completed, T2);
        accept(&conn, "open", &["a"], "2026-01-01T00:00:03.000Z");

        // Job "j": one old conclusive row followed by five newer unknown rows.
        insert_evidence(
            &conn,
            None,
            "j",
            EvidenceKind::Authoritative,
            &evidence(PostingState::Inactive, "2026-01-01T00:00:00.000Z"),
        )
        .unwrap();
        for i in 1..=5 {
            let at = format!("2026-01-01T00:00:0{i}.000Z");
            insert_evidence(
                &conn,
                None,
                "j",
                EvidenceKind::Authoritative,
                &evidence(PostingState::Unknown, &at),
            )
            .unwrap();
        }
        insert_supplementary_evidence(
            &conn,
            "r3",
            "a",
            &evidence(PostingState::Active, "2025-11-01T00:00:00.000Z"),
        )
        .unwrap();
        insert_supplementary_evidence(
            &conn,
            "r3",
            "a",
            &evidence(PostingState::Active, "2026-01-01T00:00:00.000Z"),
        )
        .unwrap();

        let report = prune_history(&conn, 1, 2, "2026-01-02T00:00:00.000Z").unwrap();
        assert_eq!(
            report,
            PruneReport {
                runs: 2,
                run_postings: 2,
                authoritative_evidence: 3,
                supplementary_evidence: 1
            }
        );

        let ids: Vec<String> = conn
            .prepare("SELECT id FROM runs ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(ids, ["open", "r3"], "non-terminal runs are never pruned");
        let source: Option<String> = conn
            .query_row("SELECT source_run_id FROM runs WHERE id='r3'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(source.is_none(), "reference to pruned run cleared");
        let fk_violations = conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count();
        assert_eq!(fk_violations, 0);

        let states: Vec<String> = conn
            .prepare("SELECT posting_state FROM posting_check_evidence WHERE job_id='j' ORDER BY attempted_at")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(states, ["inactive", "unknown", "unknown"]);
        assert_eq!(
            latest_conclusive_evidence_state(&conn, "j").unwrap(),
            Some(PostingState::Inactive)
        );
        let supplementary: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM posting_check_evidence WHERE kind='supplementary'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(supplementary, 1);

        // Idempotent.
        assert_eq!(
            prune_history(&conn, 1, 2, "2026-01-02T00:00:00.000Z").unwrap(),
            PruneReport::default()
        );
    }

    #[test]
    fn supplementary_evidence_never_touches_authoritative_state() {
        let conn = db();
        accept(&conn, "r1", &["a"], T0);
        try_mark_posting_active(&conn, "r1", "a", T1).unwrap();
        insert_evidence(
            &conn,
            Some("r1"),
            "a",
            EvidenceKind::Authoritative,
            &evidence(PostingState::Unknown, T1),
        )
        .unwrap();
        finalize_posting(
            &conn,
            "r1",
            &FinalizedPosting {
                failure_category: Some("timeout".into()),
                ..completed("a", PostingState::Unknown)
            },
        )
        .unwrap();
        let before = load_snapshot(&conn, "r1", false, T2).unwrap().unwrap();
        assert!(before.postings[0].evidence.is_some());
        insert_supplementary_evidence(&conn, "r1", "a", &evidence(PostingState::Active, T3))
            .unwrap();
        assert_eq!(
            load_snapshot(&conn, "r1", false, T2).unwrap().unwrap(),
            before
        );
        let auth = load_authoritative_evidence(&conn, "r1", "a")
            .unwrap()
            .unwrap();
        assert_eq!(auth.record.posting_state, PostingState::Unknown);
        assert_eq!(auth.kind, EvidenceKind::Authoritative);
        assert!(insert_evidence(
            &conn,
            Some("r1"),
            "a",
            EvidenceKind::Authoritative,
            &evidence(PostingState::Active, T3)
        )
        .is_err());
        assert_eq!(states_at_start(&conn, "r1").unwrap()["a"], "unknown");
    }

    #[test]
    fn savepoint_nests_inside_caller_transaction() {
        let conn = db();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        accept(&conn, "r1", &["a"], T0);
        conn.execute_batch("ROLLBACK").unwrap();
        assert!(load_snapshot(&conn, "r1", false, T0).unwrap().is_none());
    }
}
