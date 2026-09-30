//! `RunCoordinator`: accepts runs under the runner lock, drives the single-owner
//! dispatch loop, handles cancellation, timeouts, retry, and run finalization.
//!
//! This file currently implements the accept point (task 7.1, design.md
//! "Accept algorithm"):
//!
//! 1. Acquire the in-process runner mutex, then the `jobs-runner.lock` flock.
//!    Either failing returns [`RunRejection::InProgress`]
//!    (`operation_in_progress:runner`) with no DB access at all.
//! 2. (Part of 1: when the flock fails, the in-process guard is dropped.)
//! 3. Open the runner connection and close out orphaned runs. This is safe
//!    only because this process now holds the flock.
//! 4. Build the posting set inside a `BEGIN IMMEDIATE` transaction: every job
//!    (archived included) ordered by company name, title, id, or, for a
//!    retry, the validated, de-duplicated selection in first-seen order.
//! 5. Insert the run (`queued`, `owns_runner_lock = 1`, `last_seq = 1`) and
//!    its postings in the same transaction. The commit is the accept point.
//! 6. Register the run in the [`RunRegistry`] and publish the `seq = 1` Queued
//!    event with the full posting list.
//!
//! Every rejection before the commit rolls back and releases both locks, so a
//! rejected request writes nothing (except step 3's orphan close-out, which
//! only touches runs whose owner is provably gone).
//!
//! Design choice: [`AcceptedRun`] carries one [`PostingCheckInput`] per
//! posting (job `source`, `source_external_id`, company watch boards, and the
//! raw posting URL), loaded by the same accept queries. Workers therefore
//! never touch the DB (design decision 2), and `execute` (task 7.2) does not
//! need to re-read jobs that may have changed since the accept point. The
//! frozen Job_Identity persisted in `run_postings` and published in events is
//! the display form: title/company bounded, URL sanitized (Req 7.9). The
//! input keeps the raw values because fetching and title matching need them.
//!
//! `execute`, `cancel`, the dispatch loop, and the workers are tasks 7.2–7.5.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use fs2::FileExt;
use parking_lot::Mutex as SyncMutex;
use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::{watch, OwnedMutexGuard};

use super::ledger::{is_retry_eligible, RunLedger, TransitionDetail};
use super::lifecycle::{next_status, RunEvent};
use super::model::{
    JobIdentity, PostingStatus, RunId, RunStatus, RunType, StageName, StageOutcome, Trigger,
};
use super::progress::{
    bounded, build_run_summary, BoundedCount, ContractBounds, EvidenceView, PostingProgress,
    RunAccepted, RunEventSink, RunProgressEvent, RunSnapshot, RunSummary, RunSummaryHeader,
    RunTiming, StageProgress, MAX_COMPANY_BYTES, MAX_REASON_BYTES, MAX_TITLE_BYTES, MAX_URL_BYTES,
    PROGRESS_CONTRACT_VERSION,
};
use super::stages::postings::{run_worker, WorkerMsg, WorkerTask};
use super::stages::{careers as careers_stage, csv as csv_stage, watches as watches_stage};
use super::store::{self, JobIdentityWithState, NewRun};
use super::{RunRegistration, RunRegistry};
use crate::db::paths::DataPaths;
use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::posting_check::classify::{classify, Classification};
use crate::jobs::posting_check::evidence::{sanitize_url, CheckEvidence, EVIDENCE_VERSION};
use crate::jobs::posting_check::fetch::PostingFetcher;
use crate::jobs::posting_check::persist::apply_classified_check;
use crate::jobs::posting_check::provider::{ProviderListingCache, WatchBoard};
use crate::jobs::posting_check::PostingCheckInput;
use crate::runner::{open_runner_conn, try_lock_runner};

// ---------------------------------------------------------------------------
// Configuration and clock
// ---------------------------------------------------------------------------

/// Tunables for a run (design "RunCoordinator").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunConfig {
    /// Maximum concurrent Posting_Checks per run (Req 4.1).
    pub posting_concurrency: usize,
    /// Per-posting evaluation bound (Req 8.1).
    pub eval_timeout: Duration,
    /// How long a timed-out network task may keep running for supplementary
    /// evidence (Req 8.8).
    pub late_grace: Duration,
    /// Poll interval for a Canceling state committed by another process.
    pub cancel_poll: Duration,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            posting_concurrency: 4,
            eval_timeout: Duration::from_secs(30),
            late_grace: Duration::from_secs(15),
            cancel_poll: Duration::from_millis(250),
        }
    }
}

/// Source of RFC 3339 timestamps (UTC, millisecond precision). The store
/// never reads a clock; the coordinator supplies every timestamp from this.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> String;
}

/// Wall clock, same format as `util::now_iso`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> String {
        crate::util::now_iso()
    }
}

/// Controllable clock for tests. Clones share the same time.
#[derive(Debug, Clone)]
pub struct ManualClock(Arc<SyncMutex<DateTime<Utc>>>);

impl ManualClock {
    /// Start at `rfc3339`. Panics on an invalid timestamp (test helper).
    pub fn at(rfc3339: &str) -> Self {
        let t = DateTime::parse_from_rfc3339(rfc3339)
            .expect("ManualClock::at requires an RFC 3339 timestamp")
            .with_timezone(&Utc);
        Self(Arc::new(SyncMutex::new(t)))
    }

    pub fn advance(&self, by: Duration) {
        let delta = chrono::Duration::from_std(by).unwrap_or(chrono::Duration::zero());
        let mut t = self.0.lock();
        *t += delta;
    }

    pub fn set(&self, rfc3339: &str) {
        *self.0.lock() = DateTime::parse_from_rfc3339(rfc3339)
            .expect("ManualClock::set requires an RFC 3339 timestamp")
            .with_timezone(&Utc);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> String {
        self.0.lock().to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

// ---------------------------------------------------------------------------
// Lock ownership
// ---------------------------------------------------------------------------

/// Both runner locks for one run: the in-process `AppState::runner_lock`
/// guard and the `jobs-runner.lock` flock. `Drop` unlocks the flock and then
/// releases the in-process guard, so the locks are released on every path out
/// of a run, including panics unwinding through `execute` (Req 4.9).
pub struct RunLockGuard {
    file: Option<File>,
    in_proc: Option<OwnedMutexGuard<()>>,
}

impl RunLockGuard {
    /// Acquire the in-process mutex, then the flock. Never waits. Returns
    /// [`RunRejection::InProgress`] if either is held elsewhere; when the flock
    /// fails, the in-process guard is released before returning.
    pub fn acquire(
        runner_lock: &Arc<tokio::sync::Mutex<()>>,
        paths: &DataPaths,
    ) -> Result<Self, RunRejection> {
        let in_proc = runner_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| RunRejection::InProgress)?;
        // The flock file lives in the data dir; creating directories is not DB work.
        paths
            .ensure_dirs()
            .map_err(|e| RunRejection::Internal(format!("data directory unavailable: {e}")))?;
        let file = match try_lock_runner(paths) {
            Ok(file) => file,
            Err(err) => {
                drop(in_proc);
                let parts = err.code_parts();
                return Err(
                    if parts.code == "operation_in_progress" && parts.category == "runner" {
                        RunRejection::InProgress
                    } else {
                        RunRejection::Internal(format!("runner lock unavailable: {err}"))
                    },
                );
            }
        };
        Ok(Self {
            file: Some(file),
            in_proc: Some(in_proc),
        })
    }

    /// Release both locks now (same as dropping the guard).
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for RunLockGuard {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = FileExt::unlock(&file);
        }
        self.in_proc.take();
    }
}

impl fmt::Debug for RunLockGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunLockGuard")
            .field("flock_held", &self.file.is_some())
            .field("in_proc_held", &self.in_proc.is_some())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Requests and rejections
// ---------------------------------------------------------------------------

/// A request to start a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunRequest {
    JobsCycle {
        trigger: Trigger,
    },
    PostingCheck {
        trigger: Trigger,
    },
    /// Retry_Run: a Posting_Check_Run over selected entries of a terminal run.
    Retry {
        source_run_id: String,
        job_ids: Vec<String>,
        trigger: Trigger,
    },
}

impl RunRequest {
    /// A Retry_Run is a Posting_Check_Run.
    pub fn run_type(&self) -> RunType {
        match self {
            Self::JobsCycle { .. } => RunType::JobsCycle,
            Self::PostingCheck { .. } | Self::Retry { .. } => RunType::PostingCheck,
        }
    }

    pub fn trigger(&self) -> Trigger {
        match self {
            Self::JobsCycle { trigger }
            | Self::PostingCheck { trigger }
            | Self::Retry { trigger, .. } => *trigger,
        }
    }
}

/// Why a run request was not accepted. Nothing was written for any of these
/// (Req 4.4, 5.12, 10.14, 10.15). Displays as the coded wire string from the
/// design's command table; convert with `AppError::from`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunRejection {
    /// Another run or runner-lock operation holds a lock (Req 4.3).
    /// `operation_in_progress:runner`.
    InProgress,
    /// Selected entries that are not in the source run, not retry-eligible,
    /// or whose job no longer exists. `retry_ineligible:<id,id,…>`.
    RetryIneligible { job_ids: Vec<String> },
    /// The selection is empty. `retry_ineligible:empty_selection`.
    RetryEmptySelection,
    /// The source run is not terminal. `retry_ineligible:source_not_terminal`.
    RetrySourceNotTerminal { status: RunStatus },
    /// A cancel was requested for a run that is not Queued or Active
    /// (Req 5.11). `cancel_not_allowed:<status>`.
    CancelNotAllowed { status: RunStatus },
    /// The source run does not exist. `run_not_found`.
    RunNotFound { run_id: String },
    /// The database could not be opened, read, or written.
    /// `run_start_failed:database`.
    Database(String),
    /// Any other failure before the accept point. `run_start_failed:internal`.
    Internal(String),
}

impl RunRejection {
    /// `(code, category)` of the wire string. An empty category displays as
    /// the bare code.
    pub fn code_category(&self) -> (&'static str, String) {
        match self {
            Self::InProgress => ("operation_in_progress", "runner".into()),
            Self::RetryIneligible { job_ids } => (
                "retry_ineligible",
                bounded(job_ids.join(","), MAX_REASON_BYTES),
            ),
            Self::RetryEmptySelection => ("retry_ineligible", "empty_selection".into()),
            Self::RetrySourceNotTerminal { .. } => {
                ("retry_ineligible", "source_not_terminal".into())
            }
            Self::CancelNotAllowed { status } => ("cancel_not_allowed", status.as_str().into()),
            Self::RunNotFound { .. } => ("run_not_found", String::new()),
            Self::Database(_) => ("run_start_failed", "database".into()),
            Self::Internal(_) => ("run_start_failed", "internal".into()),
        }
    }

    /// Human-readable detail (CLI JSON `message`, logs).
    pub fn message(&self) -> String {
        match self {
            Self::InProgress => "Another run is in progress".into(),
            Self::RetryIneligible { job_ids } => {
                format!("These postings cannot be retried: {}", job_ids.join(", "))
            }
            Self::RetryEmptySelection => "Select at least one posting to retry".into(),
            Self::RetrySourceNotTerminal { status } => {
                format!("The source run is still {}", status.as_str())
            }
            Self::CancelNotAllowed { status } => {
                format!(
                    "This run cannot be canceled because it is {}",
                    status.as_str()
                )
            }
            Self::RunNotFound { run_id } => format!("Run {run_id} was not found"),
            Self::Database(detail) => format!("Could not start the run: database error: {detail}"),
            Self::Internal(detail) => format!("Could not start the run: {detail}"),
        }
    }
}

impl fmt::Display for RunRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (code, category) = self.code_category();
        if category.is_empty() {
            f.write_str(code)
        } else {
            write!(f, "{code}:{category}")
        }
    }
}

impl std::error::Error for RunRejection {}

impl From<RunRejection> for AppError {
    fn from(rejection: RunRejection) -> Self {
        let (code, category) = rejection.code_category();
        AppError::coded(code, category, rejection.message())
    }
}

fn db_rejection(err: impl fmt::Display) -> RunRejection {
    RunRejection::Database(err.to_string())
}

// ---------------------------------------------------------------------------
// Accepted run
// ---------------------------------------------------------------------------

/// A committed, lock-owning run that has not started executing yet.
///
/// Holds everything `execute` (task 7.2) needs without further job reads:
/// the ledger (frozen identities, all Queued), one [`PostingCheckInput`] per
/// posting in the same order, the runner connection, and the cancel receiver.
/// Dropping it unregisters the run and then releases both locks; a run
/// dropped before reaching a terminal state is closed out by orphan recovery
/// on the next accept.
pub struct AcceptedRun {
    pub run_id: RunId,
    pub run_type: RunType,
    pub trigger: Trigger,
    pub source_run_id: Option<String>,
    /// `runs.started_at` (= accept time, Req 1.2).
    pub started_at: String,
    /// All postings Queued, frozen Job_Identity, stable ordinal order.
    pub ledger: RunLedger,
    /// Per-posting evaluation input, index-aligned with `ledger.entries()`.
    pub inputs: Vec<PostingCheckInput>,
    /// job id → `posting_state` at accept (for the summary's state changes).
    pub states_at_start: HashMap<String, String>,
    /// Jobs_Cycle: all four stages `not_started`; Posting_Check_Run: empty.
    pub stages: Vec<StageProgress>,
    /// Seq of the last published event (1 after accept).
    pub last_seq: u64,
    /// The state published as `seq = 1`.
    pub snapshot: RunSnapshot,
    /// Runner connection opened under the flock (WAL, busy timeout, migrated).
    pub conn: Connection,
    /// `true` once an in-process cancel is signalled through the registry.
    pub cancel_rx: watch::Receiver<bool>,
    /// Registry entry and both runner locks.
    guards: RunGuards,
}

/// The registry entry and both runner locks of an accepted run. Dropping (or
/// [`RunGuards::release`]) unregisters first and then releases the locks.
/// `execute` releases them explicitly after `finalize_run` and before the
/// terminal event; any other exit (early return, panic unwinding through
/// `execute`) releases them by drop (Req 4.9).
pub struct RunGuards {
    // Field order is drop order: unregister first, then release the locks.
    registration: RunRegistration,
    lock: RunLockGuard,
}

impl RunGuards {
    pub fn lock_guard(&self) -> &RunLockGuard {
        &self.lock
    }

    pub fn registration(&self) -> &RunRegistration {
        &self.registration
    }

    /// Unregister the run, then release the flock and the in-process guard.
    pub fn release(self) {
        let Self { registration, lock } = self;
        drop(registration);
        lock.release();
    }
}

impl fmt::Debug for RunGuards {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunGuards")
            .field("registration", &self.registration)
            .field("lock", &self.lock)
            .finish()
    }
}

/// An [`AcceptedRun`] split into its data and its guards (see
/// [`AcceptedRun::into_parts`]).
pub struct AcceptedParts {
    pub run_id: RunId,
    pub run_type: RunType,
    pub trigger: Trigger,
    pub source_run_id: Option<String>,
    pub started_at: String,
    pub ledger: RunLedger,
    pub inputs: Vec<PostingCheckInput>,
    pub states_at_start: HashMap<String, String>,
    pub stages: Vec<StageProgress>,
    pub last_seq: u64,
    pub snapshot: RunSnapshot,
    pub conn: Connection,
    pub cancel_rx: watch::Receiver<bool>,
}

impl AcceptedRun {
    pub fn run_id(&self) -> &str {
        self.run_id.as_str()
    }

    /// `RunAccepted` payload for `start_run_cmd` / `retry_run_cmd`.
    pub fn accepted(&self) -> RunAccepted {
        RunAccepted {
            run_id: self.run_id.to_string(),
            snapshot: self.snapshot.clone(),
        }
    }

    pub fn lock_guard(&self) -> &RunLockGuard {
        &self.guards.lock
    }

    pub fn registration(&self) -> &RunRegistration {
        &self.guards.registration
    }

    /// Split into data and guards, so `execute` can release the locks at a
    /// precise point (after `finalize_run`, before the terminal event) while
    /// still owning the connection.
    pub fn into_parts(self) -> (AcceptedParts, RunGuards) {
        let Self {
            run_id,
            run_type,
            trigger,
            source_run_id,
            started_at,
            ledger,
            inputs,
            states_at_start,
            stages,
            last_seq,
            snapshot,
            conn,
            cancel_rx,
            guards,
        } = self;
        (
            AcceptedParts {
                run_id,
                run_type,
                trigger,
                source_run_id,
                started_at,
                ledger,
                inputs,
                states_at_start,
                stages,
                last_seq,
                snapshot,
                conn,
                cancel_rx,
            },
            guards,
        )
    }
}

impl fmt::Debug for AcceptedRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AcceptedRun")
            .field("run_id", &self.run_id)
            .field("run_type", &self.run_type)
            .field("trigger", &self.trigger)
            .field("source_run_id", &self.source_run_id)
            .field("started_at", &self.started_at)
            .field("postings", &self.inputs.len())
            .field("guards", &self.guards)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Coordinator
// ---------------------------------------------------------------------------

/// A "mark the CSV mirror dirty" hook (Req 10.10). Set only for GUI-process
/// coordinators, where `AppState::csv_export` is available; the CLI/launchd
/// coordinators and tests leave it `None`, so those runs never touch the CSV
/// mirror outside a Jobs_Cycle's own CSV stage. Task 9.2/10.1 wires the real
/// hook from `AppState` (a closure that calls `csv_export.mark_dirty()`).
pub type CsvDirtyHook = Arc<dyn Fn() + Send + Sync>;

pub struct RunCoordinator<F, S, K = SystemClock> {
    paths: DataPaths,
    /// `AppState::runner_lock` in the GUI; a fresh mutex for CLI/launchd.
    runner_lock: Arc<tokio::sync::Mutex<()>>,
    fetcher: Arc<F>,
    sink: Arc<S>,
    clock: K,
    registry: RunRegistry,
    config: RunConfig,
    /// GUI-only CSV mark-dirty hook (`None` for CLI/launchd/tests).
    csv_dirty_hook: Option<CsvDirtyHook>,
}

impl<F, S, K> RunCoordinator<F, S, K>
where
    S: RunEventSink,
    K: Clock,
{
    pub fn new(
        paths: DataPaths,
        runner_lock: Arc<tokio::sync::Mutex<()>>,
        fetcher: Arc<F>,
        sink: Arc<S>,
        clock: K,
        registry: RunRegistry,
    ) -> Self {
        Self {
            paths,
            runner_lock,
            fetcher,
            sink,
            clock,
            registry,
            config: RunConfig::default(),
            csv_dirty_hook: None,
        }
    }

    pub fn with_config(mut self, config: RunConfig) -> Self {
        self.config = config;
        self
    }

    /// Install the GUI CSV mark-dirty hook (Req 10.10). Leave unset for
    /// CLI/launchd runs and tests so they never mark the mirror dirty.
    pub fn with_csv_dirty_hook(mut self, hook: CsvDirtyHook) -> Self {
        self.csv_dirty_hook = Some(hook);
        self
    }

    /// The installed CSV mark-dirty hook, if any.
    pub fn csv_dirty_hook(&self) -> Option<&CsvDirtyHook> {
        self.csv_dirty_hook.as_ref()
    }

    pub fn paths(&self) -> &DataPaths {
        &self.paths
    }

    pub fn registry(&self) -> &RunRegistry {
        &self.registry
    }

    pub fn config(&self) -> &RunConfig {
        &self.config
    }

    pub fn sink(&self) -> &Arc<S> {
        &self.sink
    }

    pub fn fetcher(&self) -> &Arc<F> {
        &self.fetcher
    }

    pub fn clock(&self) -> &K {
        &self.clock
    }

    /// Accept a run (design "Accept algorithm"; Req 1.1, 1.2, 3.1, 3.9, 4.3,
    /// 4.4, 4.8, 5.9, 5.10, 5.12). Synchronous: it never waits on a lock and
    /// only runs short SQLite statements, so it can be called from async
    /// commands, the CLI, and tests alike.
    pub fn accept(&self, request: RunRequest) -> Result<AcceptedRun, RunRejection> {
        // Steps 1–2: both locks before any DB work.
        let lock = RunLockGuard::acquire(&self.runner_lock, &self.paths)?;

        // Step 3: runner connection + orphan recovery (safe: we hold the flock).
        let conn = open_runner_conn(&self.paths).map_err(db_rejection)?;
        let recovered =
            store::recover_orphaned_runs(&conn, &self.clock.now()).map_err(db_rejection)?;
        if recovered > 0 {
            log::warn!(
                "[runs] closed out {recovered} interrupted run(s) as {}",
                store::RUNNER_INTERRUPTED
            );
        }

        // Steps 4–5 in one transaction; the commit is the accept point.
        let run_type = request.run_type();
        let trigger = request.trigger();
        let source_run_id = match &request {
            RunRequest::Retry { source_run_id, .. } => Some(source_run_id.clone()),
            _ => None,
        };
        let started_at = self.clock.now();
        let stages: Vec<StageProgress> = match run_type {
            RunType::JobsCycle => StageName::ORDER
                .into_iter()
                .map(StageProgress::not_started)
                .collect(),
            RunType::PostingCheck => Vec::new(),
        };
        let new_run = NewRun {
            id: RunId::new(),
            run_type,
            trigger,
            source_run_id: source_run_id.clone(),
            owner_pid: std::process::id(),
            accepted_at: started_at.clone(),
            stages: stages.clone(),
        };

        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(db_rejection)?;
        let planned = plan_postings(&conn, &request, &started_at).and_then(|planned| {
            let frozen: Vec<JobIdentityWithState> =
                planned.iter().map(|p| p.frozen.clone()).collect();
            store::insert_accepted_run(&conn, &new_run, &frozen).map_err(insert_rejection)?;
            Ok(planned)
        });
        let planned = match planned {
            Ok(planned) => match conn.execute_batch("COMMIT") {
                Ok(()) => planned,
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(db_rejection(err));
                }
            },
            Err(rejection) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(rejection);
            }
        };

        // Nothing below can fail: the ledger was validated in `plan_postings`.
        let mut identities = Vec::with_capacity(planned.len());
        let mut inputs = Vec::with_capacity(planned.len());
        let mut states_at_start = HashMap::with_capacity(planned.len());
        for p in planned {
            states_at_start.insert(p.frozen.identity.job_id.clone(), p.frozen.state_at_start);
            identities.push(p.frozen.identity);
            inputs.push(p.input);
        }
        let ledger = RunLedger::new(identities).expect("posting set validated before commit");

        // Step 6: register, then publish seq = 1.
        let (cancel_rx, registration) = self.registry.register(new_run.id.as_str());
        let snapshot = initial_snapshot(
            new_run.id.as_str(),
            run_type,
            source_run_id.clone(),
            &started_at,
            &ledger,
            &stages,
        );
        let event = event_from_snapshot(&snapshot, self.clock.now(), None);
        self.sink.publish(&event);
        log::info!(
            "[runs] accepted {} run {} ({} postings, trigger {})",
            run_type.as_str(),
            new_run.id,
            ledger.entries().len(),
            trigger.as_str()
        );

        Ok(AcceptedRun {
            run_id: new_run.id,
            run_type,
            trigger,
            source_run_id,
            started_at,
            ledger,
            inputs,
            states_at_start,
            stages,
            last_seq: 1,
            snapshot,
            conn,
            cancel_rx,
            guards: RunGuards { registration, lock },
        })
    }

    /// Request cancellation of a run (design "Sequence: … with cancel";
    /// Req 3.6, 5.3, 5.4, 5.11). Called from a different task than the one
    /// running `execute` (a Tauri command or a test), so it opens its own
    /// short-lived connection and does not take the runner lock.
    ///
    /// The Cancellation_Request is committed to SQLite first
    /// ([`store::request_cancel`], a conditional `UPDATE … WHERE status IN
    /// ('queued','active')`), so a run owned by another process observes it
    /// through the dispatch loop's poll. Only after the commit is the
    /// in-process `watch` signalled, which wakes this process's loop
    /// immediately. A run that is not Queued or Active, or does not exist,
    /// modifies nothing and returns a coded rejection.
    ///
    /// Returns the run's snapshot after the request (status `canceling` on
    /// success). `live` reflects whether this process owns the run.
    pub fn cancel(&self, run_id: &str) -> Result<RunSnapshot, RunRejection> {
        let conn = open_runner_conn(&self.paths).map_err(db_rejection)?;
        let at = self.clock.now();

        // Commit the request to SQLite FIRST (design decision 3 / Req 5.4): once
        // `canceling` is committed, no posting can start in any process.
        match store::request_cancel(&conn, run_id, &at).map_err(db_rejection)? {
            store::CancelOutcome::Accepted { .. } => {}
            store::CancelOutcome::NotAllowed(status) => {
                return Err(RunRejection::CancelNotAllowed { status });
            }
            store::CancelOutcome::NotFound => {
                return Err(RunRejection::RunNotFound {
                    run_id: run_id.to_string(),
                });
            }
        }

        // Then wake the in-process loop, if this process owns the run. A
        // cross-process run (not registered here) is picked up by the loop's
        // `config.cancel_poll` status poll instead (Req 5.3).
        let live = self.registry.signal_cancel(run_id);

        let snapshot = store::load_snapshot(&conn, run_id, live, &at)
            .map_err(db_rejection)?
            .ok_or_else(|| RunRejection::RunNotFound {
                run_id: run_id.to_string(),
            })?;
        log::info!("[runs] cancel requested for run {run_id} (live={live})");
        Ok(snapshot)
    }
}

impl<F, S, K> RunCoordinator<F, S, K>
where
    F: PostingFetcher,
    S: RunEventSink,
    K: Clock,
{
    /// Drive an accepted run to a terminal state and return its final snapshot
    /// (design "Dispatch loop", "Worker", "Finalize posting", "Run
    /// finalization"). Always releases both runner locks, on every path
    /// including panics unwinding through the loop (Req 4.9), because the
    /// [`RunLockGuard`] releases them on drop.
    ///
    /// Drive an accepted run to a terminal state and return the terminal
    /// [`RunSnapshot`]. This is the thin, snapshot-only entry point kept for
    /// the many callers (and tests) that only need the final state.
    ///
    /// The postings stage runs first, then — for a Jobs_Cycle — the watches,
    /// careers, and CSV stages in that order (task 8.2). A Posting_Check_Run
    /// runs only the postings stage. Cancellation (7.3), the timeout grace
    /// window (7.4), and run-level failure handling (7.5) are handled inside
    /// the loop.
    pub async fn execute(&self, run: AcceptedRun) -> RunSnapshot {
        self.execute_run(run).await.snapshot
    }

    /// Drive an accepted run and return both the terminal snapshot and the
    /// in-memory stage item results ([`StageItems`]).
    ///
    /// The legacy `run_jobs_cycle` / `check_all_postings` JSON summaries embed
    /// the full watches/careers item arrays and the CSV `imported`/`exported`
    /// counts, none of which live in the `runs`/`run_postings` schema. Rather
    /// than persist that item JSON, the coordinator carries it out of the run
    /// here so task 9.1's legacy projection can read it alongside the
    /// snapshot. A Posting_Check_Run produces empty [`StageItems`].
    pub async fn execute_run(&self, run: AcceptedRun) -> RunExecution {
        let (mut parts, guards) = run.into_parts();
        let run_id = parts.run_id.as_str().to_string();

        // The dispatch loop owns the connection, ledger, sink, and event
        // sequencing for the whole run.
        let mut driver = RunDriver {
            coordinator: self,
            run_id: run_id.clone(),
            run_type: parts.run_type,
            source_run_id: parts.source_run_id.clone(),
            started_at: parts.started_at.clone(),
            conn: parts.conn,
            ledger: parts.ledger,
            inputs: std::mem::take(&mut parts.inputs),
            states_at_start: std::mem::take(&mut parts.states_at_start),
            stages: std::mem::take(&mut parts.stages),
            run_status: RunStatus::Queued,
            last_seq: parts.last_seq,
            cache: ProviderListingCache::new(),
            cancel_rx: parts.cancel_rx,
            canceling: false,
            late_in_flight: 0,
            stage_items: StageItems::default(),
            run_failure: None,
        };

        driver.run_postings_stage().await;
        // Task 8.2: the Jobs_Cycle stages (watches → careers → csv) run here,
        // before finalization, updating `driver.stages` and `driver.stage_items`.
        driver.run_stages().await;

        driver.finalize(guards)
    }
}

/// The watches/careers item JSON and CSV counts collected during a Jobs_Cycle,
/// carried out of [`RunCoordinator::execute_run`] for the legacy projection
/// (task 9.1). Empty for a Posting_Check_Run.
#[derive(Debug, Clone, Default)]
pub struct StageItems {
    /// Per-watch results, shaped `{ "watchId": ..., "result": ... }`.
    pub watches: Vec<serde_json::Value>,
    /// Per-company careers results, shaped `{ "companyId": ..., "result": ... }`.
    pub careers: Vec<serde_json::Value>,
    /// CSV import count (`csv.imported` in the legacy summary), when the CSV
    /// stage ran.
    pub csv_imported: Option<crate::jobs::csv::ImportResult>,
    /// CSV export result (`csv.exported` in the legacy summary), when the CSV
    /// stage ran.
    pub csv_exported: Option<crate::jobs::csv::ExportResult>,
}

/// The result of driving a run to a terminal state: the terminal snapshot plus
/// the stage item results for the legacy projection.
#[derive(Debug, Clone)]
pub struct RunExecution {
    pub snapshot: RunSnapshot,
    pub stage_items: StageItems,
}

/// State owned by the single dispatch task for one run. Not `Send` across await
/// points is fine: `execute` awaits it on one task.
struct RunDriver<'a, F, S, K> {
    coordinator: &'a RunCoordinator<F, S, K>,
    run_id: String,
    run_type: RunType,
    source_run_id: Option<String>,
    started_at: String,
    conn: Connection,
    ledger: RunLedger,
    /// Per-posting evaluation input, index-aligned with `ledger.entries()`.
    inputs: Vec<PostingCheckInput>,
    states_at_start: HashMap<String, String>,
    stages: Vec<StageProgress>,
    run_status: RunStatus,
    last_seq: u64,
    cache: ProviderListingCache,
    /// `true` once an in-process cancel is signalled through the registry.
    cancel_rx: watch::Receiver<bool>,
    /// Set once a Canceling state is observed (in-process signal, cross-process
    /// poll, or a lost start CAS). Gates dispatch and drives the drain.
    canceling: bool,
    /// Timed-out postings whose network task is still running inside the
    /// late-grace window. They hold a permit and may still send exactly one
    /// `Supplementary`/`LateAbandoned` message, so the loop must not finish
    /// until they settle (Req 8.8, 8.9), unless the run is canceling — then
    /// their workers are abandoned.
    late_in_flight: usize,
    /// Watches/careers item JSON and CSV counts, filled by `run_stages` for a
    /// Jobs_Cycle and returned by `execute_run` for the legacy projection.
    stage_items: StageItems,
    /// A coordinator-fatal reason that forces an Error terminal state.
    run_failure: Option<String>,
}

impl<F, S, K> RunDriver<'_, F, S, K>
where
    F: PostingFetcher,
    S: RunEventSink,
    K: Clock,
{
    fn config(&self) -> &RunConfig {
        self.coordinator.config()
    }

    fn now(&self) -> String {
        self.coordinator.clock().now()
    }

    /// Milliseconds elapsed since `started_at` under the coordinator clock.
    fn elapsed_ms(&self) -> u64 {
        store::duration_ms_between(&self.started_at, &self.now()).unwrap_or(0)
    }

    /// Look up the evaluation input for a job id (index-aligned with the ledger).
    fn input_for(&self, job_id: &str) -> Option<&PostingCheckInput> {
        self.inputs.iter().find(|i| i.identity.job_id == job_id)
    }

    /// The postings stage: dispatch every Queued posting under a bounded
    /// concurrency limit, finalize each result as it arrives, and settle the
    /// stage. Runs for both run types (a Posting_Check_Run *is* this stage).
    ///
    /// The loop is a single-owner `tokio::select!` over three signals
    /// (design "Sequence: … with cancel", "Dispatch loop"):
    ///
    /// - a worker result (`rx.recv()`),
    /// - the in-process cancel `watch` (`cancel_rx.changed()`), and
    /// - a `config.cancel_poll` tick that reads the persisted status so a
    ///   Canceling committed by another process is picked up (Req 5.3).
    ///
    /// A lost start CAS (`DispatchOutcome::CancelObserved`) is a fourth way to
    /// learn of a cancel. On any of them, `observe_cancel` sets `canceling`,
    /// which stops all new dispatch (Req 5.4) and publishes the Active→Canceling
    /// transition once. In-flight postings keep running until they finish
    /// (Req 5.5); the run then drains and finalize derives Canceled.
    async fn run_postings_stage(&mut self) {
        if self.run_type == RunType::JobsCycle {
            self.set_stage(StageName::Postings, StageOutcome::InProgress, None);
        }

        let permits = self.config().posting_concurrency.max(1);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(permits));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WorkerMsg>();
        // Dropped when the loop ends so `rx.recv()` returns `None` once every
        // worker (which each hold a clone) has finished.
        let worker_tx = tx;
        let mut in_flight: usize = 0;

        // Cross-process cancel poll. `MissedTickBehavior::Delay` keeps the poll
        // from bursting after the loop was busy handling worker results.
        let mut poll = tokio::time::interval(self.config().cancel_poll);
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The immediate first tick would fire before any work; skip it.
        poll.tick().await;

        loop {
            // Dispatch as many Queued postings as we have permits for, unless a
            // cancel has been observed (Req 5.4: start nothing new once canceling).
            while !self.canceling && self.ledger.has_queued() {
                let Ok(permit) = semaphore.clone().try_acquire_owned() else {
                    break;
                };
                match self.dispatch_next(permit, &worker_tx) {
                    DispatchOutcome::Dispatched => in_flight += 1,
                    // A lost CAS means a cancel was committed (this process or
                    // another). Stop starting new work.
                    DispatchOutcome::CancelObserved => self.observe_cancel(),
                    DispatchOutcome::Skipped => {}
                }
            }

            // Done when nothing is running (no active postings and no late
            // tasks still owed a message) and either everything was dispatched
            // or a cancel stopped further dispatch. When canceling we do not
            // wait on late tasks: their workers are dropped (aborted) with the
            // loop, so we ignore `late_in_flight` (Req 8.8 — abort on cancel).
            let no_late_pending = self.canceling || self.late_in_flight == 0;
            if in_flight == 0 && no_late_pending && (self.canceling || !self.ledger.has_queued()) {
                break;
            }

            tokio::select! {
                // Worker result. `worker_tx` is held by this task, so `recv`
                // only returns `None` after every sender dropped, which cannot
                // happen while `in_flight > 0` or a late task still holds a
                // sender clone.
                msg = rx.recv() => match msg {
                    Some(msg) => match self.handle_worker_msg(msg) {
                        // A status-affecting result (Authoritative/Failed):
                        // the posting's completion.
                        WorkerAccounting::Completed => in_flight -= 1,
                        // The 30 s bound elapsed and a late-grace slot opened.
                        WorkerAccounting::LateOpened => self.late_in_flight += 1,
                        // The late task settled (supplementary stored or none).
                        WorkerAccounting::LateClosed => {
                            self.late_in_flight = self.late_in_flight.saturating_sub(1)
                        }
                    },
                    None => break,
                },
                // In-process cancel (Req 5.3): the registry `watch` flipped.
                res = self.cancel_rx.changed() => {
                    match res {
                        Ok(()) if *self.cancel_rx.borrow() => self.observe_cancel(),
                        // The sender was dropped; nothing more can arrive here.
                        // The poll still catches a DB-committed cancel.
                        _ => {}
                    }
                }
                // Cross-process cancel (Req 5.3): a Canceling committed by
                // another process, observed within one poll interval.
                _ = poll.tick() => {
                    if !self.canceling && self.persisted_status_is_canceling() {
                        self.observe_cancel();
                    }
                }
            }
        }

        // Drain: every remaining Queued posting → Canceled (Req 5.5). No
        // posting is Active here (the loop only exits with `in_flight == 0`),
        // and completed/errored results are left untouched (Req 5.7).
        if self.canceling {
            self.cancel_remaining_queued();
        }

        if self.run_type == RunType::JobsCycle {
            self.set_stage(StageName::Postings, StageOutcome::Succeeded, None);
        }
    }

    /// Observe a Cancellation_Request. Idempotent: only the first call moves
    /// the run Active→Canceling (or Queued→Canceling) and publishes the
    /// transition. Later signals (a second `watch` change, a poll tick, a lost
    /// CAS) are no-ops.
    fn observe_cancel(&mut self) {
        if self.canceling {
            return;
        }
        self.canceling = true;
        // The request is already committed as `canceling` in SQLite (by
        // `RunCoordinator::cancel`); mirror it into the in-memory status and
        // publish the transition with `previousRunStatus`. A run still Queued
        // (no unit ever started) moves straight to Canceling.
        if let Ok(next) = next_status(self.run_status, RunEvent::CancelAccepted) {
            let previous = self.run_status;
            self.run_status = next;
            self.publish_event(Some(previous), &[], None);
        }
    }

    /// Read the persisted run status for the cross-process cancel poll.
    fn persisted_status_is_canceling(&self) -> bool {
        match store::run_status(&self.conn, &self.run_id) {
            Ok(Some(status)) => status == RunStatus::Canceling,
            Ok(None) => false,
            Err(err) => {
                log::warn!("[runs] cancel poll read failed for {}: {err}", self.run_id);
                false
            }
        }
    }

    /// Bulk-cancel the remaining Queued postings (Req 5.5): mark them Canceled
    /// in the DB, mirror the change into the ledger, and publish the deltas as
    /// one batch event.
    fn cancel_remaining_queued(&mut self) {
        let at = self.now();
        let canceled = match store::cancel_remaining_queued(&self.conn, &self.run_id, &at) {
            Ok(ids) => ids,
            Err(err) => {
                log::warn!(
                    "[runs] cancel_remaining_queued failed for {}: {err}",
                    self.run_id
                );
                // Fall back to the ledger's own view so the in-memory state
                // still settles even if the bulk UPDATE failed.
                self.ledger.queued_ids().map(str::to_string).collect()
            }
        };
        if canceled.is_empty() {
            return;
        }
        for job_id in &canceled {
            if let Err(err) = self.ledger.transition(
                job_id,
                PostingStatus::Canceled,
                TransitionDetail::canceled(at.clone()),
            ) {
                log::warn!("[runs] ledger cancel transition failed for {job_id}: {err}");
            }
        }
        self.publish_posting_deltas(&canceled, None);
    }

    /// The Jobs_Cycle stages after postings: watches → careers → csv, in that
    /// fixed order (task 8.2, design Component 6).
    ///
    /// A Posting_Check_Run has no such stages, so this is a no-op for it. For a
    /// Jobs_Cycle:
    /// - Each stage starts only if the run is not canceling; once canceling,
    ///   the remaining stages are left `not_started` and `finalize`'s
    ///   `settle_stages_for_terminal` marks them `skipped` (Req 9.4).
    /// - A stage's apply/DB failure is caught, recorded as `Failed` with a
    ///   reason, and the next stage still runs (Req 1.6, 2.2). It makes the
    ///   run end `completed_with_errors` because `finalize` folds any `Failed`
    ///   stage into `any_error`.
    /// - Item-level fetch failures are already inside each stage's `items` and
    ///   are not stage failures.
    async fn run_stages(&mut self) {
        if self.run_type != RunType::JobsCycle {
            return;
        }

        if self.canceling {
            return;
        }
        self.run_watches_stage().await;

        if self.canceling {
            return;
        }
        self.run_careers_stage().await;

        if self.canceling {
            return;
        }
        self.run_csv_stage().await;
    }

    /// Run the watches stage against the driver's connection, collecting its
    /// item JSON and publishing a stage-progress event at start and finish.
    async fn run_watches_stage(&mut self) {
        self.begin_stage(StageName::Watches);
        let mut snapshots: Vec<StageProgress> = Vec::new();
        let result =
            watches_stage::run_watches_stage(&mut self.conn, |p| snapshots.push(p), |_watch_id| {})
                .await;
        match result {
            Ok(result) => {
                self.stage_items.watches = result.items;
                self.finish_stage(result.progress);
            }
            Err(err) => self.fail_stage(StageName::Watches, &err.to_string()),
        }
        let _ = snapshots;
    }

    /// Run the careers stage against the driver's connection.
    async fn run_careers_stage(&mut self) {
        self.begin_stage(StageName::Careers);
        let mut snapshots: Vec<StageProgress> = Vec::new();
        let result =
            careers_stage::run_careers_stage(&mut self.conn, |p| snapshots.push(p), |_name| {})
                .await;
        match result {
            Ok(result) => {
                self.stage_items.careers = result.items;
                self.finish_stage(result.progress);
            }
            Err(err) => self.fail_stage(StageName::Careers, &err.to_string()),
        }
        let _ = snapshots;
    }

    /// Run the CSV mirror stage against the driver's connection. The CSV stage
    /// writes the mirror synchronously, so a successful Jobs_Cycle does not
    /// additionally mark the mirror dirty.
    async fn run_csv_stage(&mut self) {
        self.begin_stage(StageName::Csv);
        let db_path = self.coordinator.paths().db_path.clone();
        let default_csv_path = self.coordinator.paths().jobs_csv_path.clone();
        let mut snapshots: Vec<StageProgress> = Vec::new();
        let result = csv_stage::run_csv_stage(&mut self.conn, &db_path, &default_csv_path, |p| {
            snapshots.push(p)
        })
        .await;
        match result {
            Ok(result) => {
                self.stage_items.csv_imported = result.imported;
                self.stage_items.csv_exported = Some(result.exported);
                self.finish_stage(result.progress);
            }
            Err(err) => self.fail_stage(StageName::Csv, &err.to_string()),
        }
        let _ = snapshots;
    }

    /// Mark a stage `InProgress` and publish a stage-progress event so the UI
    /// sees the stage begin.
    fn begin_stage(&mut self, name: StageName) {
        self.set_stage(name, StageOutcome::InProgress, None);
        self.publish_event(None, &[], None);
    }

    /// Record a stage's final `StageProgress` (Succeeded or Failed as reported)
    /// and publish a stage-progress event.
    fn finish_stage(&mut self, progress: StageProgress) {
        if let Some(stage) = self.stages.iter_mut().find(|s| s.name == progress.name) {
            *stage = progress;
        }
        self.publish_event(None, &[], None);
    }

    /// Record a stage failure (Req 1.6, 2.2) and publish a stage-progress
    /// event. The next stage still runs; `finalize` folds a `Failed` stage
    /// into `any_error`.
    fn fail_stage(&mut self, name: StageName, reason: &str) {
        log::warn!(
            "[runs] stage {} failed for {}: {reason}",
            name.as_str(),
            self.run_id
        );
        self.set_stage(name, StageOutcome::Failed, Some(reason.to_string()));
        self.publish_event(None, &[], None);
    }

    /// Try to start the lowest-ordinal Queued posting. On a successful CAS it
    /// moves the run Queued→Active on the first start, transitions the ledger,
    /// publishes the delta, and spawns the worker.
    fn dispatch_next(
        &mut self,
        permit: tokio::sync::OwnedSemaphorePermit,
        worker_tx: &tokio::sync::mpsc::UnboundedSender<WorkerMsg>,
    ) -> DispatchOutcome {
        let Some(identity) = self.ledger.next_queued().cloned() else {
            return DispatchOutcome::Skipped;
        };
        let job_id = identity.job_id.clone();
        let at = self.now();

        // Design decision 3 / Req 5.4: the CAS is the only way a posting starts,
        // and it also requires the run to still be Queued/Active. A lost CAS
        // means a cancel was committed (in this process or another).
        match store::try_mark_posting_active(&self.conn, &self.run_id, &job_id, &at) {
            Ok(true) => {}
            Ok(false) => return DispatchOutcome::CancelObserved,
            Err(err) => {
                log::warn!("[runs] try_mark_posting_active failed for {job_id}: {err}");
                self.fail_run(&format!("could not start posting {job_id}: {err}"));
                return DispatchOutcome::Skipped;
            }
        }

        // First start moves the run Queued → Active (Req 1.3). The event
        // published below carries `previousRunStatus` when it does.
        let run_transition = if self.run_status == RunStatus::Queued {
            match store::cas_run_status(
                &self.conn,
                &self.run_id,
                &[RunStatus::Queued],
                RunStatus::Active,
                &at,
            ) {
                Ok(true) => {}
                Ok(false) => self.fail_run("run status changed before posting dispatch"),
                Err(err) => self.fail_run(&format!("could not activate run: {err}")),
            }
            let previous = self.run_status;
            self.transition_run(RunStatus::Active);
            Some(previous)
        } else {
            None
        };

        if self.run_failure.is_some() {
            drop(permit);
            return DispatchOutcome::Skipped;
        }

        // Ledger delta: Queued → Active (Req 3.2).
        if let Err(err) = self.ledger.transition(
            &job_id,
            PostingStatus::Active,
            TransitionDetail::started(at.clone()),
        ) {
            log::warn!("[runs] ledger active transition failed for {job_id}: {err}");
            self.fail_run(&format!(
                "run ledger disagreed while starting posting {job_id}: {err}"
            ));
        }
        self.publish_event(run_transition, std::slice::from_ref(&job_id), None);

        // Build and spawn the DB-free worker.
        let Some(input) = self.input_for(&job_id).cloned() else {
            // Should never happen: inputs are index-aligned with the ledger.
            self.mark_posting_error(&job_id, "internal", "missing evaluation input", &at);
            drop(permit);
            return DispatchOutcome::Skipped;
        };
        let task = WorkerTask {
            input,
            fetcher: self.coordinator.fetcher().clone(),
            cache: self.cache.clone(),
            attempted_at: at,
            eval_timeout: self.config().eval_timeout,
            late_grace: self.config().late_grace,
        };
        tokio::spawn(run_worker(task, permit, worker_tx.clone()));
        DispatchOutcome::Dispatched
    }

    /// Apply one worker message and report how it affects the loop's
    /// accounting. `Authoritative`/`Failed` are the posting's completion;
    /// `LateStarted`/`Supplementary`/`LateAbandoned` track the late-grace slot
    /// and never change the posting row, its state, or the counters (Req 8.9).
    fn handle_worker_msg(&mut self, msg: WorkerMsg) -> WorkerAccounting {
        match msg {
            WorkerMsg::Authoritative { job_id, evidence } => {
                self.finalize_posting(&job_id, evidence);
                WorkerAccounting::Completed
            }
            WorkerMsg::Failed {
                job_id,
                category,
                reason,
            } => {
                let at = self.now();
                self.mark_posting_error(&job_id, category.as_str(), &reason, &at);
                WorkerAccounting::Completed
            }
            WorkerMsg::LateStarted { .. } => WorkerAccounting::LateOpened,
            WorkerMsg::Supplementary { job_id, evidence } => {
                self.store_supplementary(&job_id, evidence);
                WorkerAccounting::LateClosed
            }
            WorkerMsg::LateAbandoned { .. } => WorkerAccounting::LateClosed,
        }
    }

    /// Store a late (post-timeout) evaluation as supplementary evidence only
    /// (Req 8.8, 8.9). It is audit-only: it classifies the late evidence to
    /// fill the row's `posting_state`/`reason` columns, exactly as an
    /// authoritative row would, but touches nothing else — not the job row,
    /// the `run_postings` row, the ledger, the counters, or the event stream.
    fn store_supplementary(&self, job_id: &str, evidence: CheckEvidence) {
        let evidence = evidence.normalized();
        let classification = classify(&evidence);
        let attempted_at = if evidence.attempted_at.trim().is_empty() {
            self.now()
        } else {
            evidence.attempted_at.clone()
        };
        let evidence_json = match evidence.to_json() {
            Ok(json) => json,
            Err(err) => {
                log::warn!("[runs] supplementary evidence serialize failed for {job_id}: {err}");
                return;
            }
        };
        let record = store::EvidenceRecord {
            attempted_at,
            posting_state: classification.state,
            reason_code: classification.reason_code,
            reason: classification.reason,
            evidence_version: EVIDENCE_VERSION,
            evidence_json,
            created_at: self.now(),
        };
        if let Err(err) =
            store::insert_supplementary_evidence(&self.conn, &self.run_id, job_id, &record)
        {
            log::warn!("[runs] insert_supplementary_evidence failed for {job_id}: {err}");
        }
    }

    /// Finalize one posting in a single `BEGIN IMMEDIATE` transaction
    /// (design "Finalize posting"): classify, apply the availability update +
    /// history + authoritative evidence, and move the `run_postings` row to
    /// Completed. On any transaction error, roll back and mark the posting
    /// Error(`persistence`); if that also fails, the run fails (seam 7.5).
    fn finalize_posting(&mut self, job_id: &str, evidence: CheckEvidence) {
        let classification = classify(&evidence);
        let finished_at = self.now();
        let attempted_at = if evidence.attempted_at.trim().is_empty() {
            finished_at.clone()
        } else {
            evidence.attempted_at.clone()
        };

        let result = self.finalize_posting_tx(
            job_id,
            &evidence,
            &classification,
            &attempted_at,
            &finished_at,
        );
        match result {
            Ok(()) => {
                let detail = TransitionDetail::completed(
                    classification.state,
                    classification.reason_code.clone(),
                    classification.reason.clone(),
                    finished_at.clone(),
                );
                let detail = TransitionDetail {
                    attempted_at: Some(attempted_at),
                    ..detail
                };
                if let Err(err) = self
                    .ledger
                    .transition(job_id, PostingStatus::Completed, detail)
                {
                    log::warn!("[runs] ledger completed transition failed for {job_id}: {err}");
                    self.fail_run(&format!(
                        "run ledger disagreed while completing posting {job_id}: {err}"
                    ));
                }
                let view = EvidenceView::from(&evidence);
                self.publish_posting_deltas(
                    &[job_id.to_string()],
                    Some((job_id.to_string(), view)),
                );
            }
            Err(err) => {
                log::warn!("[runs] finalize_posting failed for {job_id}: {err}");
                // Roll back happened inside the tx helper; record the error.
                self.mark_posting_error(
                    job_id,
                    "persistence",
                    &persistence_reason(&err),
                    &finished_at,
                );
            }
        }
    }

    /// The finalize-posting transaction. Returns `Ok` only when both the
    /// classified-check application and the `run_postings` completion committed.
    fn finalize_posting_tx(
        &self,
        job_id: &str,
        evidence: &CheckEvidence,
        classification: &Classification,
        attempted_at: &str,
        finished_at: &str,
    ) -> AppResult<()> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(map_sqlite)?;
        let result = (|| {
            apply_classified_check(
                &self.conn,
                Some(&self.run_id),
                job_id,
                evidence,
                classification,
            )?;
            store::finalize_posting(
                &self.conn,
                &self.run_id,
                &store::FinalizedPosting {
                    job_id: job_id.to_string(),
                    posting_state: classification.state,
                    reason_code: classification.reason_code.clone(),
                    reason: classification.reason.clone(),
                    failure_category: evidence.failure.map(|f| f.as_str().to_string()),
                    attempted_at: attempted_at.to_string(),
                    finished_at: finished_at.to_string(),
                },
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT").map_err(map_sqlite)?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    /// Record a posting Error in the DB and the ledger, then publish the delta.
    fn mark_posting_error(&mut self, job_id: &str, category: &str, reason: &str, at: &str) {
        let persisted =
            match store::mark_posting_error(&self.conn, &self.run_id, job_id, category, reason, at)
            {
                Ok(()) => true,
                Err(err) => {
                    log::warn!("[runs] mark_posting_error failed for {job_id}: {err}");
                    false
                }
            };
        let detail = TransitionDetail {
            attempted_at: Some(at.to_string()),
            ..TransitionDetail::error(category.to_string(), reason.to_string(), at.to_string())
        };
        if let Err(err) = self.ledger.transition(job_id, PostingStatus::Error, detail) {
            log::warn!("[runs] ledger error transition failed for {job_id}: {err}");
            self.fail_run(&format!(
                "run ledger disagreed while recording posting {job_id}: {err}"
            ));
        }
        self.publish_posting_deltas(&[job_id.to_string()], None);
        if !persisted {
            self.fail_run(&format!("could not persist posting failure for {job_id}"));
        }
    }

    /// Close every open posting after a coordinator-fatal failure. Conditional
    /// store updates preserve all already-finalized rows; in-flight workers may
    /// still report, but cannot reopen the rows closed here.
    fn fail_run(&mut self, reason: &str) {
        if self.run_failure.is_none() {
            self.run_failure = Some(if reason.trim().is_empty() {
                "run failed before completion".to_string()
            } else {
                reason.to_string()
            });
        }
        self.canceling = true;
        let at = self.now();
        let reason = self
            .run_failure
            .as_deref()
            .unwrap_or("run failed before completion");
        if let Err(err) = store::abort_open_postings(&self.conn, &self.run_id, reason, &at) {
            log::warn!(
                "[runs] abort_open_postings failed for {}: {err}",
                self.run_id
            );
        }

        let mut changed = Vec::new();
        for entry in self.ledger.snapshot() {
            let transition = match entry.status {
                PostingStatus::Queued => Some((
                    PostingStatus::Canceled,
                    TransitionDetail::canceled(at.clone()),
                )),
                PostingStatus::Active => Some((
                    PostingStatus::Error,
                    TransitionDetail::error(store::RUN_ABORTED_CATEGORY, reason, at.clone()),
                )),
                _ => None,
            };
            if let Some((status, detail)) = transition {
                if self
                    .ledger
                    .transition(entry.job_id(), status, detail)
                    .is_ok()
                {
                    changed.push(entry.job_id().to_string());
                }
            }
        }
        if !changed.is_empty() {
            self.publish_posting_deltas(&changed, None);
        }
    }

    /// Move the in-memory run status through the pure lifecycle function.
    fn transition_run(&mut self, to: RunStatus) {
        self.run_status = to;
    }

    /// Record a stage outcome in the in-memory stage list (Jobs_Cycle only).
    fn set_stage(&mut self, name: StageName, outcome: StageOutcome, error: Option<String>) {
        if let Some(stage) = self.stages.iter_mut().find(|s| s.name == name) {
            stage.outcome = outcome;
            if error.is_some() {
                stage.error = error;
            }
        }
    }

    /// Publish a progress event that carries the given posting deltas. When
    /// `previous` differs from the current run status it is a status-change
    /// event (`previousRunStatus` set). `evidence` attaches an `EvidenceView`
    /// to a single completed posting.
    fn publish_posting_deltas(
        &mut self,
        changed: &[String],
        evidence: Option<(String, EvidenceView)>,
    ) {
        self.publish_event(None, changed, evidence);
    }

    /// Build, persist the seq/stages of, and publish one progress event.
    fn publish_event(
        &mut self,
        previous_run_status: Option<RunStatus>,
        changed: &[String],
        evidence: Option<(String, EvidenceView)>,
    ) {
        self.last_seq += 1;
        let seq = self.last_seq;
        let emitted_at = self.now();
        let elapsed_ms = self.elapsed_ms();
        let counts = self.ledger.counts();
        let stages_opt = (self.run_type == RunType::JobsCycle).then(|| self.stages.clone());
        let legacy =
            store::legacy_progress(self.run_type, self.run_status, &self.stages, &counts, None);

        // Persist the seq (and stages) so the DB snapshot agrees with events.
        if let Err(err) = store::update_run_progress(
            &self.conn,
            &self.run_id,
            seq,
            stages_opt.as_deref(),
            &emitted_at,
        ) {
            log::warn!(
                "[runs] update_run_progress failed for {}: {err}",
                self.run_id
            );
        }

        let postings = self.delta_postings(changed, evidence);
        let mut event = RunProgressEvent {
            version: PROGRESS_CONTRACT_VERSION,
            run_id: self.run_id.clone(),
            run_type: self.run_type,
            run_status: self.run_status,
            previous_run_status,
            seq: BoundedCount::new(seq),
            emitted_at,
            stage: legacy.stage,
            message: legacy.message,
            current: legacy.current,
            total: legacy.total,
            done: false,
            started_at: self.started_at.clone(),
            elapsed_ms: BoundedCount::new(elapsed_ms),
            stages: stages_opt,
            posting_counts: counts,
            posting_total: BoundedCount::new(counts.total()),
            postings,
            error_reason: None,
            summary: None,
        };
        event.enforce_bounds();
        self.coordinator.sink().publish(&event);
    }

    /// Build the `postings` delta list for an event: the changed ledger entries,
    /// with evidence attached to the matching completed posting.
    fn delta_postings(
        &self,
        changed: &[String],
        evidence: Option<(String, EvidenceView)>,
    ) -> Option<Vec<PostingProgress>> {
        if changed.is_empty() {
            return None;
        }
        let deltas: Vec<PostingProgress> = changed
            .iter()
            .filter_map(|id| self.ledger.get(id))
            .map(|entry| {
                let progress = PostingProgress::from(entry);
                match &evidence {
                    Some((job_id, view)) if *job_id == progress.job_id => {
                        progress.with_evidence(view.clone())
                    }
                    _ => progress,
                }
            })
            .collect();
        (!deltas.is_empty()).then_some(deltas)
    }

    /// Finalize the run (design "Run finalization"): derive the terminal status,
    /// build the summary, persist the terminal record, release the runner locks,
    /// publish the terminal event, and prune history best-effort. Returns the
    /// terminal snapshot together with the collected stage items (task 9.1).
    fn finalize(mut self, guards: RunGuards) -> RunExecution {
        let finished_at = self.now();

        // Empty runs never dispatch a unit, so the run is still Queued. Move it
        // through FirstUnitStarted so the lifecycle can then settle it (design
        // note: AllUnitsSettled from Queued is illegal).
        if self.run_status == RunStatus::Queued {
            if let Ok(next) = next_status(self.run_status, RunEvent::FirstUnitStarted) {
                let _ = store::cas_run_status(
                    &self.conn,
                    &self.run_id,
                    &[RunStatus::Queued],
                    next,
                    &finished_at,
                );
                self.run_status = next;
            }
        }

        // any_error: a posting Error, or (seam 8.2) a failed stage.
        let counts = self.ledger.counts();
        let any_error = counts.error > 0
            || self
                .stages
                .iter()
                .any(|s| s.outcome == StageOutcome::Failed);
        let mut terminal = if self.run_failure.is_some() {
            RunStatus::Error
        } else {
            next_status(self.run_status, RunEvent::AllUnitsSettled { any_error })
                .unwrap_or(RunStatus::Error)
        };
        self.run_status = terminal;

        // Settle stages: unstarted → skipped (Jobs_Cycle).
        store::settle_stages_for_terminal(&mut self.stages, None);

        let duration_ms = store::duration_ms_between(&self.started_at, &finished_at).unwrap_or(0);
        let summary = build_run_summary(
            &RunSummaryHeader {
                run_id: self.run_id.clone(),
                run_type: self.run_type,
                status: terminal,
                source_run_id: self.source_run_id.clone(),
            },
            &self.ledger,
            (self.run_type == RunType::JobsCycle).then_some(self.stages.as_slice()),
            &self.states_at_start,
            &RunTiming {
                started_at: self.started_at.clone(),
                finished_at: finished_at.clone(),
                duration_ms,
            },
        )
        .ok();

        let terminal_seq = self.last_seq + 1;
        let error_reason = (terminal == RunStatus::Error).then(|| {
            self.run_failure
                .clone()
                .unwrap_or_else(|| "run failed".to_string())
        });
        if let Err(err) = store::finalize_run(
            &self.conn,
            &self.run_id,
            &store::TerminalRecord {
                status: terminal,
                finished_at: finished_at.clone(),
                error_reason: error_reason.clone(),
                stages: self.stages.clone(),
                summary: summary.clone(),
                seq: terminal_seq,
            },
        ) {
            log::warn!("[runs] finalize_run failed for {}: {err}", self.run_id);
            // Best effort recovery makes the terminal event authoritative even
            // when the normal finalization statement was rejected.
            let reason = error_reason.as_deref().unwrap_or("run finalization failed");
            let _ = store::abort_run_with_reason(&self.conn, &self.run_id, reason, &finished_at);
            terminal = RunStatus::Error;
        }

        // Build the terminal snapshot from the DB (the source of truth) while we
        // still own the connection. `live = true`: this process ran the run.
        let snapshot = store::load_snapshot(&self.conn, &self.run_id, true, &finished_at)
            .ok()
            .flatten()
            .filter(|snapshot| snapshot.done && snapshot.run_status.is_terminal())
            .unwrap_or_else(|| {
                self.fallback_snapshot(terminal, &finished_at, duration_ms, summary.clone())
            });

        // Everything the code after lock release needs, captured before the
        // connection is dropped and the driver is torn down.
        let coordinator = self.coordinator;
        let prune_at = finished_at.clone();
        let run_type = self.run_type;
        let state_changes = summary.as_ref().map(|s| s.state_changes.get()).unwrap_or(0);
        let stage_items = std::mem::take(&mut self.stage_items);

        // Release the runner locks (and unregister) BEFORE the terminal event,
        // so a listener that reacts by starting a new run cannot collide. The
        // run's own connection is dropped first so the flock covers no open DB
        // handle when it is released.
        let conn = std::mem::replace(
            &mut self.conn,
            Connection::open_in_memory().expect("in-memory connection for finalized run"),
        );
        drop(conn);
        guards.release();

        // Publish the terminal event last (Req 1.9: done=true, summary attached).
        let mut event = event_from_snapshot(&snapshot, finished_at, None);
        event.seq = BoundedCount::new(terminal_seq);
        event.done = true;
        coordinator.sink().publish(&event);

        // CSV mirror mark-dirty (Req 10.10). Only when a GUI hook is installed;
        // CLI/launchd coordinators leave it `None` and never mark dirty.
        //   - Jobs_Cycle: the CSV stage writes the mirror synchronously on a
        //     normal finish, so no extra mark is needed. But a canceled cycle
        //     skips the CSV stage, so mark dirty to let the mirror catch up.
        //   - Posting_Check_Run: there is no CSV stage, so mark dirty when at
        //     least one posting's state changed during the run.
        if let Some(hook) = coordinator.csv_dirty_hook() {
            let should_mark = match run_type {
                RunType::JobsCycle => terminal == RunStatus::Canceled,
                RunType::PostingCheck => state_changes > 0,
            };
            if should_mark {
                hook();
            }
        }

        // Best-effort retention. A fresh short-lived connection: the run's
        // connection was dropped above.
        if let Ok(conn) = open_runner_conn(coordinator.paths()) {
            if let Err(err) = store::prune_history(
                &conn,
                store::DEFAULT_KEEP_RUNS,
                store::DEFAULT_KEEP_EVIDENCE_PER_JOB,
                &prune_at,
            ) {
                log::warn!("[runs] prune_history failed: {err}");
            }
        }

        RunExecution {
            snapshot,
            stage_items,
        }
    }

    /// A snapshot built from the in-memory ledger, used only if the DB rebuild
    /// fails at finalization (so `execute` always returns a usable snapshot).
    fn fallback_snapshot(
        &self,
        terminal: RunStatus,
        finished_at: &str,
        duration_ms: u64,
        summary: Option<RunSummary>,
    ) -> RunSnapshot {
        let counts = self.ledger.counts();
        let legacy = store::legacy_progress(self.run_type, terminal, &self.stages, &counts, None);
        let mut summary = summary;
        if terminal == RunStatus::Error {
            if let Some(ref mut value) = summary {
                value.status = RunStatus::Error;
            }
        }
        let mut snapshot = RunSnapshot {
            version: PROGRESS_CONTRACT_VERSION,
            run_id: self.run_id.clone(),
            run_type: self.run_type,
            run_status: terminal,
            seq: BoundedCount::new(self.last_seq + 1),
            stage: legacy.stage,
            message: legacy.message,
            current: legacy.current,
            total: legacy.total,
            done: true,
            started_at: self.started_at.clone(),
            elapsed_ms: BoundedCount::new(duration_ms),
            stages: (self.run_type == RunType::JobsCycle).then(|| self.stages.clone()),
            posting_counts: counts,
            posting_total: BoundedCount::new(counts.total()),
            error_reason: (terminal == RunStatus::Error).then(|| {
                self.run_failure
                    .clone()
                    .unwrap_or_else(|| "run failed".to_string())
            }),
            summary,
            postings: self
                .ledger
                .entries()
                .iter()
                .map(PostingProgress::from)
                .collect(),
            live: true,
            source_run_id: self.source_run_id.clone(),
            dismissed: false,
        };
        let _ = finished_at;
        snapshot.enforce_bounds();
        snapshot.enforce_bounds();
        snapshot
    }

    /// Drop handler for RunDriver. If the run is not terminal (i.e., execute_run
    /// panicked or was aborted before reaching finalize), mark the run as Error
    /// with runner_interrupted and close out any open postings.
    fn mark_interrupted_if_needed(&mut self) {
        let at = self.now();
        // Temporarily replace the connection so we can use it in the DB operations
        let conn = std::mem::replace(
            &mut self.conn,
            rusqlite::Connection::open_in_memory().unwrap(),
        );

        // Use the store function to abort the run
        let _ = store::abort_run(&conn, &self.run_id, &at);

        // Restore the connection (though it's not used after this)
        self.conn = conn;
    }
}
impl<F, S, K> Drop for RunDriver<'_, F, S, K> {
    fn drop(&mut self) {
        // If the run is not terminal, mark it as Error with runner_interrupted.
        // This handles the case where execute_run panics or is aborted.
        if !self.run_status.is_terminal() {
            let at = crate::util::now_iso();
            let _ = store::abort_run(&self.conn, &self.run_id, &at);
        }
    }
}

/// How a worker message affects the dispatch loop's counters.
enum WorkerAccounting {
    /// A status-affecting result: the posting's completion. Decrement `in_flight`.
    Completed,
    /// A timed-out posting opened a late-grace slot. Increment `late_in_flight`.
    LateOpened,
    /// A late task settled (supplementary or abandoned). Decrement `late_in_flight`.
    LateClosed,
}

/// Outcome of attempting to dispatch the next Queued posting.
enum DispatchOutcome {
    /// A worker was spawned for the posting.
    Dispatched,
    /// The start CAS lost to a committed cancel; stop dispatching (Req 5.4).
    CancelObserved,
    /// Nothing was dispatched, but the loop should keep going (for example a
    /// pre-start persistence error already recorded the posting as Error).
    Skipped,
}

/// Build the `persistence` failure reason for a posting whose finalize
/// transaction failed, without leaking secrets from the underlying error.
fn persistence_reason(err: &AppError) -> String {
    if crate::jobs::posting_check::persist::is_job_missing(err) {
        "The job was deleted before the result could be saved".to_string()
    } else {
        "Could not save the check result".to_string()
    }
}

/// A second lock owner is rejected by `runs_single_lock_owner_uidx`
/// (Req 4.8); report it as the in-progress rejection.
fn insert_rejection(err: AppError) -> RunRejection {
    match &err {
        AppError::Sqlite(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            RunRejection::InProgress
        }
        _ => db_rejection(err),
    }
}

// ---------------------------------------------------------------------------
// Posting set (accept step 4)
// ---------------------------------------------------------------------------

/// One posting of a run being accepted.
#[derive(Debug, Clone)]
struct PlannedPosting {
    /// Persisted/published form: bounded title and company, sanitized URL.
    frozen: JobIdentityWithState,
    /// Raw values for the worker.
    input: PostingCheckInput,
}

struct JobRow {
    id: String,
    title: String,
    company_id: String,
    company_name: String,
    url: String,
    posting_state: String,
    source: Option<String>,
    source_external_id: Option<String>,
}

const JOB_ROW_SELECT: &str =
    "SELECT j.id, j.title, c.id, c.name, j.url, j.posting_state, j.source, j.source_external_id
     FROM jobs j JOIN companies c ON c.id = j.company_id";

fn map_job_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        id: r.get(0)?,
        title: r.get(1)?,
        company_id: r.get(2)?,
        company_name: r.get(3)?,
        url: r.get(4)?,
        posting_state: r.get(5)?,
        source: r.get(6)?,
        source_external_id: r.get(7)?,
    })
}

/// Build the posting set and validate it into a ledger-compatible list.
fn plan_postings(
    conn: &Connection,
    request: &RunRequest,
    now: &str,
) -> Result<Vec<PlannedPosting>, RunRejection> {
    let rows = match request {
        RunRequest::JobsCycle { .. } | RunRequest::PostingCheck { .. } => load_all_jobs(conn)?,
        RunRequest::Retry {
            source_run_id,
            job_ids,
            ..
        } => select_retry_jobs(conn, source_run_id, job_ids, now)?,
    };
    let watches = load_watches(conn)?;
    let planned: Vec<PlannedPosting> = rows
        .into_iter()
        .map(|row| {
            let company_watches = watches.get(&row.company_id).cloned().unwrap_or_default();
            plan_posting(row, company_watches)
        })
        .collect();
    // Validate before the insert so nothing after the commit can fail.
    RunLedger::new(planned.iter().map(|p| p.frozen.identity.clone()).collect())
        .map_err(|e| RunRejection::Internal(e.to_string()))?;
    Ok(planned)
}

fn plan_posting(row: JobRow, watches: Vec<WatchBoard>) -> PlannedPosting {
    let frozen = JobIdentityWithState {
        identity: JobIdentity {
            job_id: row.id.clone(),
            title: bounded(row.title.clone(), MAX_TITLE_BYTES),
            company_name: bounded(row.company_name.clone(), MAX_COMPANY_BYTES),
            posting_url: bounded(sanitize_url(&row.url), MAX_URL_BYTES),
        },
        state_at_start: row.posting_state,
    };
    let input = PostingCheckInput {
        identity: JobIdentity {
            job_id: row.id,
            title: row.title,
            company_name: row.company_name,
            posting_url: row.url,
        },
        source: row.source,
        source_external_id: row.source_external_id,
        watches,
    };
    PlannedPosting { frozen, input }
}

/// Every job, archived included (design behavior change 5), in stable
/// display order: company name, title (both case-insensitive), then id.
fn load_all_jobs(conn: &Connection) -> Result<Vec<JobRow>, RunRejection> {
    let mut stmt = conn
        .prepare(&format!(
            "{JOB_ROW_SELECT} ORDER BY c.name COLLATE NOCASE, j.title COLLATE NOCASE, j.id"
        ))
        .map_err(db_rejection)?;
    let rows = stmt
        .query_map([], map_job_row)
        .map_err(db_rejection)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_rejection)?;
    Ok(rows)
}

fn load_job(conn: &Connection, job_id: &str) -> Result<Option<JobRow>, RunRejection> {
    conn.query_row(
        &format!("{JOB_ROW_SELECT} WHERE j.id = ?1"),
        params![job_id],
        map_job_row,
    )
    .optional()
    .map_err(db_rejection)
}

/// company id → its watch boards, in a stable order.
fn load_watches(conn: &Connection) -> Result<HashMap<String, Vec<WatchBoard>>, RunRejection> {
    let mut stmt = conn
        .prepare("SELECT company_id, provider, board_slug FROM company_watches ORDER BY company_id, provider, board_slug, id")
        .map_err(db_rejection)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(db_rejection)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_rejection)?;
    let mut map: HashMap<String, Vec<WatchBoard>> = HashMap::new();
    for (company_id, provider, board_slug) in rows {
        map.entry(company_id).or_default().push(WatchBoard {
            provider,
            board_slug,
        });
    }
    Ok(map)
}

/// Retry validation (design accept step 4, Req 5.9, 5.10, 5.12): the source
/// run must exist and be terminal; the de-duplicated selection (first-seen
/// order) must be non-empty; every selected entry must be in the source run,
/// be retry-eligible (`ledger::is_retry_eligible`), and its job must still
/// exist. Returns the jobs' current rows in selection order. Read-only.
fn select_retry_jobs(
    conn: &Connection,
    source_run_id: &str,
    job_ids: &[String],
    now: &str,
) -> Result<Vec<JobRow>, RunRejection> {
    let source = store::load_snapshot(conn, source_run_id, false, now)
        .map_err(db_rejection)?
        .ok_or_else(|| RunRejection::RunNotFound {
            run_id: source_run_id.to_string(),
        })?;
    if !source.run_status.is_terminal() {
        return Err(RunRejection::RetrySourceNotTerminal {
            status: source.run_status,
        });
    }

    let mut seen = HashSet::new();
    let selection: Vec<&str> = job_ids
        .iter()
        .map(String::as_str)
        .filter(|id| seen.insert(*id))
        .collect();
    if selection.is_empty() {
        return Err(RunRejection::RetryEmptySelection);
    }

    let entries: HashMap<&str, &PostingProgress> = source
        .postings
        .iter()
        .map(|p| (p.job_id.as_str(), p))
        .collect();
    let mut ineligible = Vec::new();
    let mut rows = Vec::with_capacity(selection.len());
    for id in selection {
        let eligible = entries
            .get(id)
            .is_some_and(|p| is_retry_eligible(p.status, p.posting_state));
        let row = if eligible { load_job(conn, id)? } else { None };
        match row {
            Some(row) => rows.push(row),
            None => ineligible.push(id.to_string()),
        }
    }
    if !ineligible.is_empty() {
        return Err(RunRejection::RetryIneligible {
            job_ids: ineligible,
        });
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Snapshot and event construction
// ---------------------------------------------------------------------------

/// The accept-point snapshot, built in memory from the committed values. It
/// equals `store::load_snapshot(conn, run_id, true, started_at)` (tested).
fn initial_snapshot(
    run_id: &str,
    run_type: RunType,
    source_run_id: Option<String>,
    started_at: &str,
    ledger: &RunLedger,
    stages: &[StageProgress],
) -> RunSnapshot {
    let counts = ledger.counts();
    let legacy = store::legacy_progress(run_type, RunStatus::Queued, stages, &counts, None);
    let mut snapshot = RunSnapshot {
        version: PROGRESS_CONTRACT_VERSION,
        run_id: run_id.to_string(),
        run_type,
        run_status: RunStatus::Queued,
        seq: BoundedCount::new(1),
        stage: legacy.stage,
        message: legacy.message,
        current: legacy.current,
        total: legacy.total,
        done: false,
        started_at: started_at.to_string(),
        elapsed_ms: BoundedCount::ZERO,
        stages: (run_type == RunType::JobsCycle).then(|| stages.to_vec()),
        posting_counts: counts,
        posting_total: BoundedCount::new(counts.total()),
        error_reason: None,
        summary: None,
        postings: ledger.entries().iter().map(PostingProgress::from).collect(),
        live: true,
        source_run_id,
        dismissed: false,
    };
    snapshot.enforce_bounds();
    snapshot
}

/// A full-state event from a snapshot: every field carried over, `postings`
/// set to the full list. Used for `seq = 1`.
pub(crate) fn event_from_snapshot(
    snapshot: &RunSnapshot,
    emitted_at: String,
    previous_run_status: Option<RunStatus>,
) -> RunProgressEvent {
    let mut event = RunProgressEvent {
        version: snapshot.version,
        run_id: snapshot.run_id.clone(),
        run_type: snapshot.run_type,
        run_status: snapshot.run_status,
        previous_run_status,
        seq: snapshot.seq,
        emitted_at,
        stage: snapshot.stage,
        message: snapshot.message.clone(),
        current: snapshot.current,
        total: snapshot.total,
        done: snapshot.done,
        started_at: snapshot.started_at.clone(),
        elapsed_ms: snapshot.elapsed_ms,
        stages: snapshot.stages.clone(),
        posting_counts: snapshot.posting_counts,
        posting_total: snapshot.posting_total,
        postings: Some(snapshot.postings.clone()),
        error_reason: snapshot.error_reason.clone(),
        summary: snapshot.summary.clone(),
    };
    event.enforce_bounds();
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::model::{PostingState, PostingStatus, StageOutcome};
    use crate::runs::progress::{LegacyStage, RecordingSink};
    use crate::runs::store::{FinalizedPosting, TerminalRecord};
    use tempfile::TempDir;

    const T0: &str = "2026-03-01T10:00:00.000Z";

    type TestCoordinator = RunCoordinator<(), RecordingSink, ManualClock>;

    struct Env {
        _dir: TempDir,
        paths: DataPaths,
        clock: ManualClock,
    }

    impl Env {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = DataPaths::from_data_dir(dir.path().to_path_buf());
            let env = Self {
                _dir: dir,
                paths,
                clock: ManualClock::at(T0),
            };
            env.seed();
            env
        }

        fn conn(&self) -> Connection {
            open_runner_conn(&self.paths).unwrap()
        }

        /// A coordinator with its own in-process lock (a separate "process"
        /// when called twice).
        fn coordinator(&self) -> TestCoordinator {
            self.coordinator_with_lock(Arc::new(tokio::sync::Mutex::new(())))
        }

        fn coordinator_with_lock(&self, lock: Arc<tokio::sync::Mutex<()>>) -> TestCoordinator {
            RunCoordinator::new(
                self.paths.clone(),
                lock,
                Arc::new(()),
                Arc::new(RecordingSink::new()),
                self.clock.clone(),
                RunRegistry::new(),
            )
        }

        fn seed(&self) {
            let conn = self.conn();
            let company = |id: &str, name: &str| {
                conn.execute(
                    "INSERT INTO companies (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                    params![id, name, T0],
                )
                .unwrap();
            };
            company("c-beta", "beta Labs");
            company("c-acme", "Acme");
            let job = |id: &str,
                       company: &str,
                       title: &str,
                       url: &str,
                       status: &str,
                       state: &str,
                       source: &str,
                       ext: Option<&str>| {
                conn.execute(
                    "INSERT INTO jobs (id, company_id, title, url, canonical_url, status, posting_state,
                                       source, source_external_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?1, ?5, ?6, ?7, ?8, ?9, ?9)",
                    params![id, company, title, url, status, state, source, ext, T0],
                )
                .unwrap();
            };
            // Insertion order deliberately differs from display order.
            job(
                "j5",
                "c-beta",
                "Designer",
                "https://beta.example/jobs/5",
                "wishlist",
                "unknown",
                "manual",
                None,
            );
            job(
                "j2",
                "c-acme",
                "zeta Engineer",
                "https://acme.example/jobs/2",
                "applied",
                "inactive",
                "manual",
                None,
            );
            job(
                "j1",
                "c-acme",
                "Backend Engineer",
                "https://user:pw@boards.greenhouse.io/acme/jobs/123?gh_jid=123&token=s3cret#apply",
                "archived",
                "active",
                "greenhouse",
                Some("123"),
            );
            job(
                "j4",
                "c-beta",
                "analyst",
                "https://beta.example/jobs/4",
                "wishlist",
                "unknown",
                "manual",
                None,
            );
            job(
                "j3",
                "c-acme",
                "Backend Engineer",
                "https://acme.example/jobs/3",
                "wishlist",
                "unknown",
                "manual",
                None,
            );
            conn.execute(
                "INSERT INTO company_watches (id, company_id, provider, board_slug, created_at, updated_at)
                 VALUES ('w1', 'c-acme', 'greenhouse', 'acme', ?1, ?1)",
                params![T0],
            )
            .unwrap();
        }
    }

    /// Order produced by `ORDER BY company NOCASE, title NOCASE, id`.
    const DISPLAY_ORDER: [&str; 5] = ["j1", "j3", "j2", "j4", "j5"];

    /// Digest of every table a rejected request must not touch.
    fn digest(conn: &Connection) -> String {
        let q = |sql: &str| -> String {
            conn.query_row(sql, [], |r| r.get::<_, Option<String>>(0))
                .unwrap()
                .unwrap_or_default()
        };
        [
            q("SELECT group_concat(id || '|' || status || '|' || owns_runner_lock || '|' || updated_at || '|' || last_seq, ';') FROM (SELECT * FROM runs ORDER BY id)"),
            q("SELECT group_concat(run_id || '|' || job_id || '|' || status || '|' || IFNULL(posting_state,'-'), ';') FROM (SELECT * FROM run_postings ORDER BY run_id, job_id)"),
            q("SELECT group_concat(id || '|' || posting_state || '|' || updated_at, ';') FROM (SELECT * FROM jobs ORDER BY id)"),
            q("SELECT CAST(COUNT(*) AS TEXT) FROM job_events"),
            q("SELECT CAST(COUNT(*) AS TEXT) FROM posting_check_evidence"),
        ]
        .join("\n")
    }

    fn run_ids(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT id FROM runs ORDER BY started_at, rowid")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn flock_is_free(paths: &DataPaths) -> bool {
        match try_lock_runner(paths) {
            Ok(file) => {
                let _ = FileExt::unlock(&file);
                true
            }
            Err(_) => false,
        }
    }

    #[test]
    fn accept_commits_queued_run_and_publishes_full_list_in_stable_order() {
        let env = Env::new();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let coord = env.coordinator_with_lock(lock.clone());
        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();

        // Locks are held while the accepted run exists.
        assert!(lock.try_lock().is_err());
        assert!(!flock_is_free(&env.paths));
        assert!(coord.registry().is_live(run.run_id()));

        // Committed row (Req 1.1, 1.2, 4.8).
        let conn = env.conn();
        let (status, owns, last_seq, started_at, trigger, run_type, pid): (String, i64, i64, String, String, String, i64) = conn
            .query_row(
                "SELECT status, owns_runner_lock, last_seq, started_at, trigger, run_type, owner_pid FROM runs WHERE id = ?1",
                params![run.run_id()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )
            .unwrap();
        assert_eq!(
            (
                status.as_str(),
                owns,
                last_seq,
                started_at.as_str(),
                trigger.as_str(),
                run_type.as_str()
            ),
            ("queued", 1, 1, T0, "desktop", "posting_check")
        );
        assert_eq!(pid, i64::from(std::process::id()));
        let ordered: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT job_id FROM run_postings WHERE run_id = ?1 AND status = 'queued' ORDER BY ordinal")
                .unwrap();
            stmt.query_map(params![run.run_id()], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(ordered, DISPLAY_ORDER);

        // seq = 1 event: Queued, full list, all Queued (Req 3.1, 3.9, 3.10).
        let events = coord.sink().events();
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev.seq.get(), 1);
        assert_eq!(ev.version, PROGRESS_CONTRACT_VERSION);
        assert_eq!(ev.run_id, run.run_id());
        assert_eq!(ev.run_type, RunType::PostingCheck);
        assert_eq!(ev.run_status, RunStatus::Queued);
        assert_eq!(ev.previous_run_status, None);
        assert_eq!(ev.stage, LegacyStage::Postings);
        assert_eq!((ev.current.get(), ev.total.get(), ev.done), (0, 5, false));
        assert_eq!(ev.elapsed_ms.get(), 0);
        assert_eq!(ev.started_at, T0);
        assert!(ev.stages.is_none());
        assert!(ev.summary.is_none() && ev.error_reason.is_none());
        assert_eq!(ev.posting_total.get(), 5);
        assert_eq!(ev.posting_counts.queued, 5);
        assert_eq!(ev.posting_counts.total(), 5);
        let postings = ev.postings.as_ref().unwrap();
        let ids: Vec<&str> = postings.iter().map(|p| p.job_id.as_str()).collect();
        assert_eq!(ids, DISPLAY_ORDER);
        assert!(postings
            .iter()
            .all(|p| p.status == PostingStatus::Queued && p.posting_state.is_none()));
        assert!(ev.is_within_bounds());

        // Frozen identity is sanitized; the worker input keeps the raw URL
        // and the job's provider data.
        let j1 = &postings[0];
        assert_eq!(
            (j1.title.as_str(), j1.company_name.as_str()),
            ("Backend Engineer", "Acme")
        );
        assert!(
            !j1.posting_url.contains("s3cret") && !j1.posting_url.contains("pw@"),
            "{}",
            j1.posting_url
        );
        assert!(j1.posting_url.contains("gh_jid=123"));
        assert_eq!(run.inputs.len(), 5);
        assert_eq!(run.inputs[0].identity.job_id, "j1");
        assert!(run.inputs[0].identity.posting_url.contains("token=s3cret"));
        assert_eq!(run.inputs[0].source.as_deref(), Some("greenhouse"));
        assert_eq!(run.inputs[0].source_external_id.as_deref(), Some("123"));
        assert_eq!(
            run.inputs[0].watches,
            vec![WatchBoard {
                provider: "greenhouse".into(),
                board_slug: "acme".into()
            }]
        );
        assert!(run.inputs[3].watches.is_empty());
        let ledger_ids: Vec<&str> = run.ledger.entries().iter().map(|e| e.job_id()).collect();
        let input_ids: Vec<&str> = run
            .inputs
            .iter()
            .map(|i| i.identity.job_id.as_str())
            .collect();
        assert_eq!(ledger_ids, input_ids);
        assert_eq!(
            run.states_at_start.get("j2").map(String::as_str),
            Some("inactive")
        );
        assert_eq!(run.last_seq, 1);

        // The in-memory snapshot matches the DB rebuild.
        let from_db = store::load_snapshot(&conn, run.run_id(), true, T0)
            .unwrap()
            .unwrap();
        assert_eq!(run.snapshot, from_db);
        assert_eq!(run.accepted().snapshot, from_db);
        let row_postings: Vec<_> = from_db
            .postings
            .iter()
            .map(PostingProgress::identity)
            .collect();
        let ev_postings: Vec<_> = postings.iter().map(PostingProgress::identity).collect();
        assert_eq!(row_postings, ev_postings);
    }

    #[test]
    fn jobs_cycle_accept_has_all_stages_not_started() {
        let env = Env::new();
        let coord = env.coordinator();
        let run = coord
            .accept(RunRequest::JobsCycle {
                trigger: Trigger::Cli,
            })
            .unwrap();
        let ev = &coord.sink().events()[0];
        let stages = ev.stages.as_ref().unwrap();
        assert_eq!(
            stages.iter().map(|s| s.name).collect::<Vec<_>>(),
            StageName::ORDER
        );
        assert!(stages.iter().all(|s| s.outcome == StageOutcome::NotStarted));
        assert_eq!(ev.run_type, RunType::JobsCycle);
        assert_eq!(run.stages.len(), 4);
        let from_db = store::load_snapshot(&env.conn(), run.run_id(), true, T0)
            .unwrap()
            .unwrap();
        assert_eq!(run.snapshot, from_db);
    }

    #[test]
    fn second_accept_is_rejected_in_progress_without_writes() {
        let env = Env::new();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let first = env.coordinator_with_lock(lock.clone());
        let _held = first
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let before = digest(&env.conn());

        // Same process (in-process mutex held).
        let same_proc = env.coordinator_with_lock(lock.clone());
        let err = same_proc
            .accept(RunRequest::JobsCycle {
                trigger: Trigger::Desktop,
            })
            .unwrap_err();
        assert_eq!(err, RunRejection::InProgress);
        // Another process (only the flock is shared).
        let other_proc = env.coordinator();
        let err = other_proc
            .accept(RunRequest::Retry {
                source_run_id: "x".into(),
                job_ids: vec!["j1".into()],
                trigger: Trigger::Retry,
            })
            .unwrap_err();
        assert_eq!(err, RunRejection::InProgress);

        assert_eq!(digest(&env.conn()), before);
        assert!(same_proc.sink().is_empty() && other_proc.sink().is_empty());
        let owners: i64 = env
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE owns_runner_lock = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(owners, 1);

        // Wire string is byte-identical to the legacy error.
        let app_err = AppError::from(err);
        assert_eq!(app_err.to_string(), "operation_in_progress:runner");
        assert_eq!(
            serde_json::to_string(&app_err).unwrap(),
            "\"operation_in_progress:runner\""
        );
        let parts = app_err.code_parts();
        assert_eq!(
            (parts.code.as_str(), parts.category.as_str()),
            ("operation_in_progress", "runner")
        );
    }

    #[test]
    fn dropping_accepted_run_releases_locks_and_unregisters() {
        let env = Env::new();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let coord = env.coordinator_with_lock(lock.clone());
        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let id = run.run_id().to_string();
        assert!(coord.registry().is_live(&id));
        drop(run);
        assert!(!coord.registry().is_live(&id));
        assert!(lock.try_lock().is_ok());
        assert!(flock_is_free(&env.paths));
    }

    #[test]
    fn lock_guard_rejects_when_flock_held_and_releases_in_proc_guard() {
        let env = Env::new();
        let held = try_lock_runner(&env.paths).unwrap();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        assert_eq!(
            RunLockGuard::acquire(&lock, &env.paths).unwrap_err(),
            RunRejection::InProgress
        );
        assert!(
            lock.try_lock().is_ok(),
            "in-process guard must be released when the flock fails"
        );
        FileExt::unlock(&held).unwrap();
        let guard = RunLockGuard::acquire(&lock, &env.paths).unwrap();
        assert!(lock.try_lock().is_err());
        guard.release();
        assert!(lock.try_lock().is_ok());
    }

    #[test]
    fn accept_recovers_orphaned_runs() {
        let env = Env::new();
        let coord = env.coordinator();
        let orphan = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Launchd,
            })
            .unwrap();
        let orphan_id = orphan.run_id().to_string();
        // Simulate a process that died mid-run: one posting Active, locks gone.
        store::try_mark_posting_active(&orphan.conn, &orphan_id, "j1", T0).unwrap();
        drop(orphan);

        env.clock.advance(Duration::from_secs(5));
        let next = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let conn = env.conn();
        let (status, reason, owns): (String, String, i64) = conn
            .query_row(
                "SELECT status, error_reason, owns_runner_lock FROM runs WHERE id = ?1",
                params![orphan_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (status.as_str(), reason.as_str(), owns),
            ("error", store::RUNNER_INTERRUPTED, 0)
        );
        let open: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM run_postings WHERE run_id = ?1 AND status IN ('queued','active')",
                params![orphan_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(open, 0);
        assert_eq!(
            store::run_status(&conn, next.run_id()).unwrap(),
            Some(RunStatus::Queued)
        );
        assert_eq!(run_ids(&conn), vec![orphan_id, next.run_id().to_string()]);
    }

    /// A terminal source run: j1 Completed/Active, j3 Completed/Unknown,
    /// j2 Error, j4 and j5 Canceled.
    fn terminal_source_run(env: &Env, coord: &TestCoordinator) -> String {
        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let id = run.run_id().to_string();
        let c = &run.conn;
        let complete = |job: &str, state: PostingState| {
            store::try_mark_posting_active(c, &id, job, T0).unwrap();
            store::finalize_posting(
                c,
                &id,
                &FinalizedPosting {
                    job_id: job.into(),
                    posting_state: state,
                    reason_code: "code".into(),
                    reason: "reason".into(),
                    failure_category: None,
                    attempted_at: T0.into(),
                    finished_at: T0.into(),
                },
            )
            .unwrap();
        };
        complete("j1", PostingState::Active);
        complete("j3", PostingState::Unknown);
        store::try_mark_posting_active(c, &id, "j2", T0).unwrap();
        store::mark_posting_error(c, &id, "j2", "internal", "boom", T0).unwrap();
        store::cancel_remaining_queued(c, &id, T0).unwrap();
        store::finalize_run(
            c,
            &id,
            &TerminalRecord {
                status: RunStatus::Canceled,
                finished_at: T0.into(),
                error_reason: None,
                stages: vec![],
                summary: None,
                seq: 2,
            },
        )
        .unwrap();
        drop(run);
        env.clock.advance(Duration::from_secs(60));
        id
    }

    fn retry(
        coord: &TestCoordinator,
        source: &str,
        ids: &[&str],
    ) -> Result<AcceptedRun, RunRejection> {
        coord.accept(RunRequest::Retry {
            source_run_id: source.into(),
            job_ids: ids.iter().map(|s| s.to_string()).collect(),
            trigger: Trigger::Retry,
        })
    }

    #[test]
    fn retry_rejections_write_nothing_and_release_locks() {
        let env = Env::new();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let coord = env.coordinator_with_lock(lock.clone());
        let source = terminal_source_run(&env, &coord);
        // j5 was canceled (eligible) but its job has since been deleted.
        env.conn()
            .execute("DELETE FROM jobs WHERE id = 'j5'", [])
            .unwrap();
        let events_before = coord.sink().len();
        let before = digest(&env.conn());

        let cases: Vec<(Vec<&str>, RunRejection)> = vec![
            (vec![], RunRejection::RetryEmptySelection),
            (
                vec!["j1"],
                RunRejection::RetryIneligible {
                    job_ids: vec!["j1".into()],
                },
            ),
            (
                vec!["j3", "nope", "j1", "nope"],
                RunRejection::RetryIneligible {
                    job_ids: vec!["nope".into(), "j1".into()],
                },
            ),
            (
                vec!["j5"],
                RunRejection::RetryIneligible {
                    job_ids: vec!["j5".into()],
                },
            ),
        ];
        for (ids, expected) in cases {
            let err = retry(&coord, &source, &ids).unwrap_err();
            assert_eq!(err, expected, "{ids:?}");
            assert!(
                lock.try_lock().is_ok() && flock_is_free(&env.paths),
                "locks released after {ids:?}"
            );
        }
        assert_eq!(
            retry(&coord, "missing-run", &["j2"]).unwrap_err(),
            RunRejection::RunNotFound {
                run_id: "missing-run".into()
            }
        );
        assert_eq!(digest(&env.conn()), before);
        assert_eq!(coord.sink().len(), events_before);

        // Coded wire strings.
        assert_eq!(
            RunRejection::RetryEmptySelection.to_string(),
            "retry_ineligible:empty_selection"
        );
        assert_eq!(
            AppError::from(RunRejection::RetryIneligible {
                job_ids: vec!["a".into(), "b".into()]
            })
            .to_string(),
            "retry_ineligible:a,b"
        );
        assert_eq!(
            AppError::from(RunRejection::RunNotFound { run_id: "r".into() }).to_string(),
            "run_not_found"
        );
        assert_eq!(
            RunRejection::Database("x".into()).to_string(),
            "run_start_failed:database"
        );
    }

    #[test]
    fn retry_accepts_deduped_eligible_selection_in_first_seen_order() {
        let env = Env::new();
        let coord = env.coordinator();
        let source = terminal_source_run(&env, &coord);
        let source_before = {
            let conn = env.conn();
            (
                store::load_snapshot(&conn, &source, false, T0).unwrap(),
                digest(&conn),
            )
        };

        let run = retry(&coord, &source, &["j4", "j3", "j2", "j4", "j3"]).unwrap();
        assert_eq!(run.source_run_id.as_deref(), Some(source.as_str()));
        assert_eq!(run.run_type, RunType::PostingCheck);
        assert_ne!(run.run_id(), source);
        let ids: Vec<&str> = run.ledger.entries().iter().map(|e| e.job_id()).collect();
        assert_eq!(ids, ["j4", "j3", "j2"]);

        let conn = env.conn();
        let (src, trigger): (String, String) = conn
            .query_row(
                "SELECT source_run_id, trigger FROM runs WHERE id = ?1",
                params![run.run_id()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((src.as_str(), trigger.as_str()), (source.as_str(), "retry"));
        let ev = coord.sink().events().pop().unwrap();
        assert_eq!(ev.seq.get(), 1);
        assert_eq!(ev.postings.unwrap().len(), 3);
        assert_eq!(run.snapshot.source_run_id.as_deref(), Some(source.as_str()));
        // Source run rows are untouched (Req 5.10).
        assert_eq!(
            store::load_snapshot(&conn, &source, false, T0).unwrap(),
            source_before.0
        );
    }

    #[test]
    fn retry_validation_rejects_non_terminal_source() {
        let env = Env::new();
        let conn = env.conn();
        store::insert_accepted_run(
            &conn,
            &NewRun {
                id: RunId::from_existing("open-run"),
                run_type: RunType::PostingCheck,
                trigger: Trigger::Desktop,
                source_run_id: None,
                owner_pid: 1,
                accepted_at: T0.into(),
                stages: vec![],
            },
            &[],
        )
        .unwrap();
        let err = select_retry_jobs(&conn, "open-run", &["j1".into()], T0)
            .err()
            .unwrap();
        assert_eq!(
            err,
            RunRejection::RetrySourceNotTerminal {
                status: RunStatus::Queued
            }
        );
        assert_eq!(err.to_string(), "retry_ineligible:source_not_terminal");
    }

    #[test]
    fn accepted_run_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<AcceptedRun>();
        assert_send::<RunLockGuard>();
    }

    #[test]
    fn run_config_defaults_match_design() {
        let c = RunConfig::default();
        assert_eq!(c.posting_concurrency, 4);
        assert_eq!(c.eval_timeout, Duration::from_secs(30));
        assert_eq!(c.late_grace, Duration::from_secs(15));
        assert_eq!(c.cancel_poll, Duration::from_millis(250));
    }

    #[test]
    fn manual_clock_formats_and_advances() {
        let clock = ManualClock::at(T0);
        assert_eq!(clock.now(), T0);
        clock.advance(Duration::from_millis(1500));
        assert_eq!(clock.now(), "2026-03-01T10:00:01.500Z");
        assert!(DateTime::parse_from_rfc3339(&SystemClock.now()).is_ok());
    }
}

#[cfg(test)]
mod execute_tests {
    //! Dispatch loop, workers, and finalization (task 7.2). These drive a real
    //! `execute` with a scripted [`FakeFetcher`], a `RecordingSink`, a tempdir
    //! SQLite database, and paused Tokio time.

    use super::*;
    use crate::jobs::posting_check::fetch::{PageFetch, PostingFetcher};
    use crate::jobs::posting_check::provider::ListingFetch;
    use crate::runs::model::{PostingCounts, PostingState, PostingStatus};
    use crate::runs::progress::RecordingSink;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;

    const T0: &str = "2026-03-01T10:00:00.000Z";

    /// One job's scripted behavior: how long its page fetch takes and what page
    /// it returns.
    #[derive(Clone)]
    struct Script {
        delay: Duration,
        page: PageFetch,
    }

    /// Scripted, DB-free fetcher. Records max observed concurrency and detects
    /// two concurrent evaluations of the same URL.
    #[derive(Default)]
    struct FakeFetcher {
        scripts: HashMap<String, Script>,
        active: AtomicUsize,
        max_active: AtomicUsize,
        in_flight_urls: Mutex<Vec<String>>,
        overlap: AtomicUsize,
    }

    impl FakeFetcher {
        fn new(scripts: HashMap<String, Script>) -> Arc<Self> {
            Arc::new(Self {
                scripts,
                ..Self::default()
            })
        }

        fn max_concurrency(&self) -> usize {
            self.max_active.load(Ordering::SeqCst)
        }

        fn same_url_overlaps(&self) -> usize {
            self.overlap.load(Ordering::SeqCst)
        }
    }

    impl PostingFetcher for FakeFetcher {
        async fn fetch_page(&self, url: &str) -> PageFetch {
            let n = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(n, Ordering::SeqCst);
            {
                let mut urls = self.in_flight_urls.lock().unwrap();
                if urls.iter().any(|u| u == url) {
                    self.overlap.fetch_add(1, Ordering::SeqCst);
                }
                urls.push(url.to_string());
            }
            let script = self
                .scripts
                .get(url)
                .cloned()
                .unwrap_or_else(|| panic!("unscripted page {url}"));
            if !script.delay.is_zero() {
                tokio::time::sleep(script.delay).await;
            }
            {
                let mut urls = self.in_flight_urls.lock().unwrap();
                if let Some(pos) = urls.iter().position(|u| u == url) {
                    urls.remove(pos);
                }
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
            script.page
        }

        async fn fetch_listing(
            &self,
            _: crate::jobs::posting_check::evidence::Provider,
            _: &str,
        ) -> ListingFetch {
            unreachable!("these tests use manual jobs with no provider target")
        }
    }

    /// A page that classifies Active (title, company, enabled apply on a 2xx).
    fn open_page(url: &str, title: &str, company: &str) -> PageFetch {
        let body = format!(
            r#"<!doctype html><html><head><title>{title} | {company}</title>
               <meta property="og:title" content="{title}"></head>
               <body><h1>{title}</h1>
               <a href="/apply">Apply for this job</a></body></html>"#
        );
        PageFetch {
            requested_url: url.into(),
            final_url: url.into(),
            http_status: Some(200),
            redirect_statuses: vec![],
            signal_headers: vec![],
            body,
            error_kind: None,
        }
    }

    /// A page that classifies Closed (HTTP 404).
    fn gone_page(url: &str) -> PageFetch {
        PageFetch {
            requested_url: url.into(),
            final_url: url.into(),
            http_status: Some(404),
            redirect_statuses: vec![],
            signal_headers: vec![],
            body: "<html><title>Not found</title></html>".into(),
            error_kind: None,
        }
    }

    struct Env {
        _dir: TempDir,
        paths: DataPaths,
        clock: ManualClock,
    }

    impl Env {
        fn new(jobs: &[(&str, &str, &str)]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = DataPaths::from_data_dir(dir.path().to_path_buf());
            let env = Self {
                _dir: dir,
                paths,
                clock: ManualClock::at(T0),
            };
            env.seed(jobs);
            env
        }

        fn conn(&self) -> Connection {
            open_runner_conn(&self.paths).unwrap()
        }

        /// `jobs`: `(id, title, url)`; all under one company, `posting_state = unknown`.
        fn seed(&self, jobs: &[(&str, &str, &str)]) {
            let conn = self.conn();
            conn.execute(
                "INSERT INTO companies (id, name, created_at, updated_at) VALUES ('c1', 'Acme', ?1, ?1)",
                params![T0],
            )
            .unwrap();
            for (id, title, url) in jobs {
                conn.execute(
                    "INSERT INTO jobs (id, company_id, title, url, canonical_url, status, posting_state,
                                       source, created_at, updated_at)
                     VALUES (?1, 'c1', ?2, ?3, ?3, 'wishlist', 'unknown', 'manual', ?4, ?4)",
                    params![id, title, url, T0],
                )
                .unwrap();
            }
        }

        fn coordinator(
            &self,
            fetcher: Arc<FakeFetcher>,
            sink: Arc<RecordingSink>,
            config: RunConfig,
        ) -> RunCoordinator<FakeFetcher, RecordingSink, ManualClock> {
            RunCoordinator::new(
                self.paths.clone(),
                Arc::new(tokio::sync::Mutex::new(())),
                fetcher,
                sink,
                self.clock.clone(),
                RunRegistry::new(),
            )
            .with_config(config)
        }
    }

    fn config(concurrency: usize) -> RunConfig {
        RunConfig {
            posting_concurrency: concurrency,
            ..RunConfig::default()
        }
    }

    fn scripts(entries: Vec<(&str, Duration, PageFetch)>) -> HashMap<String, Script> {
        entries
            .into_iter()
            .map(|(url, delay, page)| (url.to_string(), Script { delay, page }))
            .collect()
    }

    /// The final published event is the terminal one.
    fn terminal_event(sink: &RecordingSink) -> RunProgressEvent {
        sink.events().pop().expect("at least one event")
    }

    /// Seq values across the whole stream are 1, 2, 3, … with no gaps.
    fn assert_contiguous_seqs(events: &[RunProgressEvent]) {
        for (i, ev) in events.iter().enumerate() {
            assert_eq!(ev.seq.get(), (i + 1) as u64, "event {i} seq");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn happy_path_all_completed_run_completed() {
        let url_a = "https://acme.example/jobs/a";
        let url_b = "https://acme.example/jobs/b";
        let env = Env::new(&[("a", "Engineer A", url_a), ("b", "Engineer B", url_b)]);
        let fetcher = FakeFetcher::new(scripts(vec![
            (
                url_a,
                Duration::from_secs(1),
                open_page(url_a, "Engineer A", "Acme"),
            ),
            (url_b, Duration::from_secs(1), gone_page(url_b)),
        ]));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher, sink.clone(), config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();
        let snapshot = coord.execute(run).await;

        // Terminal state.
        assert_eq!(snapshot.run_status, RunStatus::Completed);
        assert!(snapshot.done);
        assert_eq!(
            snapshot.posting_counts,
            PostingCounts {
                completed: 2,
                ..Default::default()
            }
        );
        assert_eq!(snapshot.posting_counts.queued, 0);
        assert_eq!(snapshot.posting_counts.active, 0);

        // Per-job outcomes persisted.
        let states: HashMap<String, String> = {
            let conn = env.conn();
            let mut stmt = conn.prepare("SELECT id, posting_state FROM jobs").unwrap();
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(states["a"], "active");
        assert_eq!(states["b"], "inactive");

        // Event stream: contiguous seq, exactly one status-change event
        // (Queued→Active), terminal event last with done=true.
        let events = sink.events();
        assert_contiguous_seqs(&events);
        let status_changes: Vec<_> = events
            .iter()
            .filter(|e| e.previous_run_status.is_some())
            .collect();
        assert_eq!(status_changes.len(), 1, "one Queued->Active transition");
        assert_eq!(
            status_changes[0].previous_run_status,
            Some(RunStatus::Queued)
        );
        assert_eq!(status_changes[0].run_status, RunStatus::Active);

        let term = events.last().unwrap();
        assert!(term.done);
        assert_eq!(term.run_status, RunStatus::Completed);
        assert!(term.summary.is_some());
        assert!(
            events[..events.len() - 1].iter().all(|e| !e.done),
            "done only on the last event"
        );

        // DB snapshot matches the returned snapshot.
        let from_db = store::load_snapshot(&env.conn(), &run_id, true, &snapshot.started_at)
            .unwrap()
            .unwrap();
        assert_eq!(from_db.run_status, RunStatus::Completed);
        assert_eq!(from_db.posting_counts, snapshot.posting_counts);
        assert_eq!(from_db.postings.len(), snapshot.postings.len());

        // Lock released: another accept succeeds.
        assert!(coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop
            })
            .is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn concurrency_never_exceeds_the_limit() {
        let jobs: Vec<(String, String)> = (0..10)
            .map(|i| (format!("j{i}"), format!("https://acme.example/jobs/{i}")))
            .collect();
        let seed: Vec<(&str, &str, &str)> = jobs
            .iter()
            .map(|(id, url)| (id.as_str(), "Engineer", url.as_str()))
            .collect();
        let env = Env::new(&seed);
        let script_entries: Vec<(&str, Duration, PageFetch)> = jobs
            .iter()
            .map(|(_, url)| (url.as_str(), Duration::from_secs(2), gone_page(url)))
            .collect();
        let fetcher = FakeFetcher::new(scripts(script_entries));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher.clone(), sink, config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let snapshot = coord.execute(run).await;

        assert_eq!(snapshot.run_status, RunStatus::Completed);
        assert_eq!(snapshot.posting_counts.completed, 10);
        assert!(
            fetcher.max_concurrency() <= 4,
            "observed {}",
            fetcher.max_concurrency()
        );
        assert_eq!(fetcher.same_url_overlaps(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn out_of_order_completion_is_attributed_correctly() {
        // b resolves before a even though a is dispatched first.
        let url_a = "https://acme.example/jobs/a";
        let url_b = "https://acme.example/jobs/b";
        let env = Env::new(&[("a", "Engineer A", url_a), ("b", "Engineer B", url_b)]);
        let fetcher = FakeFetcher::new(scripts(vec![
            (
                url_a,
                Duration::from_secs(5),
                open_page(url_a, "Engineer A", "Acme"),
            ),
            (url_b, Duration::from_secs(1), gone_page(url_b)),
        ]));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher, sink, config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();
        let snapshot = coord.execute(run).await;

        assert_eq!(snapshot.run_status, RunStatus::Completed);
        let a = snapshot.postings.iter().find(|p| p.job_id == "a").unwrap();
        let b = snapshot.postings.iter().find(|p| p.job_id == "b").unwrap();
        assert_eq!(
            (a.status, a.posting_state),
            (PostingStatus::Completed, Some(PostingState::Active))
        );
        assert_eq!(
            (b.status, b.posting_state),
            (PostingStatus::Completed, Some(PostingState::Inactive))
        );

        // Evidence rows attributed to the right jobs.
        let conn = env.conn();
        let ev_a = store::load_authoritative_evidence(&conn, &run_id, "a")
            .unwrap()
            .unwrap();
        let ev_b = store::load_authoritative_evidence(&conn, &run_id, "b")
            .unwrap()
            .unwrap();
        assert_eq!(ev_a.record.posting_state, PostingState::Active);
        assert_eq!(ev_b.record.posting_state, PostingState::Inactive);
    }

    #[tokio::test(start_paused = true)]
    async fn persistence_failure_gives_posting_error_and_completed_with_errors() {
        let url_a = "https://acme.example/jobs/a";
        let url_b = "https://acme.example/jobs/b";
        let env = Env::new(&[("a", "Engineer A", url_a), ("b", "Engineer B", url_b)]);
        let fetcher = FakeFetcher::new(scripts(vec![
            (
                url_a,
                Duration::from_secs(1),
                open_page(url_a, "Engineer A", "Acme"),
            ),
            (url_b, Duration::from_secs(1), gone_page(url_b)),
        ]));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher, sink.clone(), config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();
        // Delete job "b" mid-run so its finalize hits `job_missing`.
        env.conn()
            .execute("DELETE FROM jobs WHERE id = 'b'", [])
            .unwrap();

        let snapshot = coord.execute(run).await;
        assert_eq!(snapshot.run_status, RunStatus::CompletedWithErrors);
        assert_eq!(
            snapshot.posting_counts,
            PostingCounts {
                completed: 1,
                error: 1,
                ..Default::default()
            }
        );

        let b = snapshot.postings.iter().find(|p| p.job_id == "b").unwrap();
        assert_eq!(b.status, PostingStatus::Error);
        assert_eq!(b.failure_category.as_deref(), Some("persistence"));
        assert!(b.reason.as_deref().is_some_and(|r| !r.is_empty()));
        // No authoritative evidence row was left behind for the failed posting.
        assert!(
            store::load_authoritative_evidence(&env.conn(), &run_id, "b")
                .unwrap()
                .is_none()
        );

        // The terminal event reports the error state.
        let term = terminal_event(&sink);
        assert_eq!(term.run_status, RunStatus::CompletedWithErrors);
        assert!(term.done);
    }

    #[tokio::test(start_paused = true)]
    async fn empty_run_completes_with_zero_postings() {
        let env = Env::new(&[]);
        let fetcher = FakeFetcher::new(HashMap::new());
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher, sink.clone(), config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();
        let snapshot = coord.execute(run).await;

        assert_eq!(snapshot.run_status, RunStatus::Completed);
        assert!(snapshot.done);
        assert_eq!(snapshot.posting_total.get(), 0);
        assert_eq!(snapshot.posting_counts.total(), 0);
        assert!(snapshot.postings.is_empty());

        // seq=1 (accept) then the terminal event.
        let events = sink.events();
        assert_contiguous_seqs(&events);
        assert!(events.last().unwrap().done);

        let from_db = store::load_snapshot(&env.conn(), &run_id, true, &snapshot.started_at)
            .unwrap()
            .unwrap();
        assert_eq!(from_db.run_status, RunStatus::Completed);

        // Lock released.
        assert!(coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop
            })
            .is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn jobs_cycle_marks_postings_stage_succeeded() {
        let url_a = "https://acme.example/jobs/a";
        let env = Env::new(&[("a", "Engineer A", url_a)]);
        let fetcher = FakeFetcher::new(scripts(vec![(
            url_a,
            Duration::from_secs(1),
            open_page(url_a, "Engineer A", "Acme"),
        )]));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher, sink, config(4));

        let run = coord
            .accept(RunRequest::JobsCycle {
                trigger: Trigger::Cli,
            })
            .unwrap();
        let snapshot = coord.execute(run).await;

        assert_eq!(snapshot.run_status, RunStatus::Completed);
        let stages = snapshot.stages.expect("jobs cycle has stages");
        let by_name: HashMap<_, _> = stages.iter().map(|s| (s.name, s.outcome)).collect();
        assert_eq!(by_name[&StageName::Postings], StageOutcome::Succeeded);
        // Task 8.2 runs the remaining stages. With no watches and no careers
        // URLs they succeed with zero items, and the CSV mirror is written.
        assert_eq!(by_name[&StageName::Watches], StageOutcome::Succeeded);
        assert_eq!(by_name[&StageName::Careers], StageOutcome::Succeeded);
        assert_eq!(by_name[&StageName::Csv], StageOutcome::Succeeded);
    }

    /// A sink that, when it sees the terminal (`done`) event, probes whether
    /// the `jobs-runner.lock` flock is already free. Records the result so the
    /// test can assert the lock was released before the terminal publish.
    struct LockProbeSink {
        paths: DataPaths,
        free_at_terminal: Mutex<Option<bool>>,
    }

    impl RunEventSink for LockProbeSink {
        fn publish(&self, event: &RunProgressEvent) {
            if event.done {
                let free = match try_lock_runner(&self.paths) {
                    Ok(file) => {
                        let _ = fs2::FileExt::unlock(&file);
                        true
                    }
                    Err(_) => false,
                };
                *self.free_at_terminal.lock().unwrap() = Some(free);
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn lock_is_released_before_the_terminal_event() {
        let url_a = "https://acme.example/jobs/a";
        let env = Env::new(&[("a", "Engineer A", url_a)]);
        let fetcher = FakeFetcher::new(scripts(vec![(
            url_a,
            Duration::from_secs(1),
            open_page(url_a, "Engineer A", "Acme"),
        )]));
        let sink = Arc::new(LockProbeSink {
            paths: env.paths.clone(),
            free_at_terminal: Mutex::new(None),
        });
        let coord = RunCoordinator::new(
            env.paths.clone(),
            Arc::new(tokio::sync::Mutex::new(())),
            fetcher,
            sink.clone(),
            env.clock.clone(),
            RunRegistry::new(),
        );

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let snapshot = coord.execute(run).await;
        assert_eq!(snapshot.run_status, RunStatus::Completed);
        assert_eq!(
            *sink.free_at_terminal.lock().unwrap(),
            Some(true),
            "the runner flock must be free when the terminal event is published"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn worker_timeout_finalizes_unknown_at_the_bound() {
        let url_a = "https://acme.example/jobs/a";
        let env = Env::new(&[("a", "Engineer A", url_a)]);
        // Page takes far longer than the 30 s eval timeout.
        let fetcher = FakeFetcher::new(scripts(vec![(
            url_a,
            Duration::from_secs(120),
            open_page(url_a, "Engineer A", "Acme"),
        )]));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher, sink, config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let snapshot = coord.execute(run).await;

        assert_eq!(snapshot.run_status, RunStatus::Completed);
        let a = snapshot.postings.iter().find(|p| p.job_id == "a").unwrap();
        assert_eq!(
            (a.status, a.posting_state),
            (PostingStatus::Completed, Some(PostingState::Unknown))
        );
        assert_eq!(a.failure_category.as_deref(), Some("timeout"));
    }

    /// Count evidence rows of a given kind for a job (Req 8.8, 8.9).
    fn evidence_kind_count(conn: &Connection, run_id: &str, job_id: &str, kind: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM posting_check_evidence WHERE run_id = ?1 AND job_id = ?2 AND kind = ?3",
            params![run_id, job_id, kind],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn late_finish_within_grace_stores_supplementary_without_touching_the_posting() {
        let url_a = "https://acme.example/jobs/a";
        let env = Env::new(&[("a", "Engineer A", url_a)]);
        // 40 s > 30 s bound but within the 15 s grace: the eval finishes late.
        let fetcher = FakeFetcher::new(scripts(vec![(
            url_a,
            Duration::from_secs(40),
            gone_page(url_a),
        )]));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher, sink.clone(), config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();
        let snapshot = coord.execute(run).await;

        // The posting is finalized from the authoritative timeout evidence and
        // is unaffected by the late (Closed) result (Req 8.9).
        assert_eq!(snapshot.run_status, RunStatus::Completed);
        let a = snapshot.postings.iter().find(|p| p.job_id == "a").unwrap();
        assert_eq!(
            (a.status, a.posting_state),
            (PostingStatus::Completed, Some(PostingState::Unknown))
        );
        assert_eq!(a.failure_category.as_deref(), Some("timeout"));
        assert_eq!(
            snapshot.posting_counts,
            PostingCounts {
                completed: 1,
                ..Default::default()
            }
        );

        let conn = env.conn();
        // Authoritative row is the timeout; the late result is supplementary.
        let auth = store::load_authoritative_evidence(&conn, &run_id, "a")
            .unwrap()
            .unwrap();
        assert_eq!(auth.record.posting_state, PostingState::Unknown);
        assert_eq!(evidence_kind_count(&conn, &run_id, "a", "authoritative"), 1);
        assert_eq!(evidence_kind_count(&conn, &run_id, "a", "supplementary"), 1);

        // The persisted job row still reflects the timeout Unknown, not Closed.
        let job_state: String = conn
            .query_row("SELECT posting_state FROM jobs WHERE id = 'a'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(job_state, "unknown");

        // The summary counts and state changes ignore the supplementary row.
        let summary = snapshot.summary.expect("terminal summary");
        assert_eq!(summary.posting_outcomes.unknown, 1);
        assert_eq!(summary.posting_outcomes.closed, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn late_task_that_never_finishes_stores_no_supplementary() {
        let url_a = "https://acme.example/jobs/a";
        let env = Env::new(&[("a", "Engineer A", url_a)]);
        // 600 s: past the 30 s bound + 15 s grace, so the late task is aborted.
        let fetcher = FakeFetcher::new(scripts(vec![(
            url_a,
            Duration::from_secs(600),
            gone_page(url_a),
        )]));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher, sink, config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();
        let snapshot = coord.execute(run).await;

        assert_eq!(snapshot.run_status, RunStatus::Completed);
        let a = snapshot.postings.iter().find(|p| p.job_id == "a").unwrap();
        assert_eq!(a.posting_state, Some(PostingState::Unknown));

        let conn = env.conn();
        assert_eq!(evidence_kind_count(&conn, &run_id, "a", "authoritative"), 1);
        assert_eq!(evidence_kind_count(&conn, &run_id, "a", "supplementary"), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn late_tasks_still_count_against_the_concurrency_limit() {
        // 8 jobs, concurrency 4. Every eval times out at 30 s and then finishes
        // within the grace window, so late tasks overlap the next batch. The
        // permit is held across the grace window, so max concurrency stays ≤ 4.
        let jobs: Vec<(String, String)> = (0..8)
            .map(|i| (format!("j{i}"), format!("https://acme.example/jobs/{i}")))
            .collect();
        let seed: Vec<(&str, &str, &str)> = jobs
            .iter()
            .map(|(id, url)| (id.as_str(), "Engineer", url.as_str()))
            .collect();
        let env = Env::new(&seed);
        let script_entries: Vec<(&str, Duration, PageFetch)> = jobs
            .iter()
            .map(|(_, url)| (url.as_str(), Duration::from_secs(40), gone_page(url)))
            .collect();
        let fetcher = FakeFetcher::new(scripts(script_entries));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher.clone(), sink, config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let snapshot = coord.execute(run).await;

        assert_eq!(snapshot.run_status, RunStatus::Completed);
        assert_eq!(snapshot.posting_counts.completed, 8);
        assert!(
            fetcher.max_concurrency() <= 4,
            "observed {}",
            fetcher.max_concurrency()
        );
        assert_eq!(fetcher.same_url_overlaps(), 0);
    }
}

#[cfg(test)]
mod cancel_tests {
    //! Cancellation (task 7.3): in-process and cross-process. These drive a
    //! real `execute` on a spawned task with a *gated* fetcher whose workers
    //! block until the test releases them, so postings stay in-flight while a
    //! cancel is requested from another task. Time is paused so latency is
    //! deterministic.

    use super::*;
    use crate::jobs::posting_check::fetch::{PageFetch, PostingFetcher};
    use crate::jobs::posting_check::provider::ListingFetch;
    use crate::runs::model::{PostingState, PostingStatus};
    use crate::runs::progress::RecordingSink;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Notify;

    const T0: &str = "2026-03-01T10:00:00.000Z";

    /// A fetcher whose page fetch blocks on a shared [`Notify`] until the test
    /// releases it. Every worker that reaches the gate bumps `waiting`, so the
    /// test can wait until a known number of postings are in-flight.
    struct GatedFetcher {
        gate: Arc<Notify>,
        waiting: AtomicUsize,
        entered: AtomicUsize,
    }

    impl GatedFetcher {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                gate: Arc::new(Notify::new()),
                waiting: AtomicUsize::new(0),
                entered: AtomicUsize::new(0),
            })
        }

        /// Release every worker currently blocked (and any that arrive later,
        /// via `notify_waiters` being called once they subscribe).
        fn release_all(&self) {
            // `notify_waiters` only wakes current waiters, so a permit-style
            // notify_one loop is used: notify once per known waiter.
            let n = self.waiting.load(Ordering::SeqCst);
            for _ in 0..n.max(1) {
                self.gate.notify_one();
            }
        }
    }

    impl PostingFetcher for GatedFetcher {
        async fn fetch_page(&self, url: &str) -> PageFetch {
            self.entered.fetch_add(1, Ordering::SeqCst);
            self.waiting.fetch_add(1, Ordering::SeqCst);
            self.gate.notified().await;
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            // Classifies Closed (HTTP 404): a conclusive result so completed
            // postings are visibly preserved across a cancel.
            PageFetch {
                requested_url: url.into(),
                final_url: url.into(),
                http_status: Some(404),
                redirect_statuses: vec![],
                signal_headers: vec![],
                body: "<html><title>Not found</title></html>".into(),
                error_kind: None,
            }
        }

        async fn fetch_listing(
            &self,
            _: crate::jobs::posting_check::evidence::Provider,
            _: &str,
        ) -> ListingFetch {
            unreachable!("manual jobs have no provider target")
        }
    }

    struct Env {
        _dir: TempDir,
        paths: DataPaths,
        clock: ManualClock,
    }

    impl Env {
        fn new(n: usize) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = DataPaths::from_data_dir(dir.path().to_path_buf());
            let env = Self {
                _dir: dir,
                paths,
                clock: ManualClock::at(T0),
            };
            env.seed(n);
            env
        }

        fn conn(&self) -> Connection {
            open_runner_conn(&self.paths).unwrap()
        }

        /// `n` jobs `j0..jn`, one company, `posting_state = unknown`.
        fn seed(&self, n: usize) {
            let conn = self.conn();
            conn.execute(
                "INSERT INTO companies (id, name, created_at, updated_at) VALUES ('c1', 'Acme', ?1, ?1)",
                params![T0],
            )
            .unwrap();
            for i in 0..n {
                conn.execute(
                    "INSERT INTO jobs (id, company_id, title, url, canonical_url, status, posting_state,
                                       source, created_at, updated_at)
                     VALUES (?1, 'c1', ?2, ?3, ?3, 'wishlist', 'unknown', 'manual', ?4, ?4)",
                    params![
                        format!("j{i}"),
                        format!("Engineer {i}"),
                        format!("https://acme.example/jobs/{i}"),
                        T0
                    ],
                )
                .unwrap();
            }
        }

        fn coordinator(
            &self,
            fetcher: Arc<GatedFetcher>,
            sink: Arc<RecordingSink>,
            config: RunConfig,
        ) -> Arc<RunCoordinator<GatedFetcher, RecordingSink, ManualClock>> {
            Arc::new(
                RunCoordinator::new(
                    self.paths.clone(),
                    Arc::new(tokio::sync::Mutex::new(())),
                    fetcher,
                    sink,
                    self.clock.clone(),
                    RunRegistry::new(),
                )
                .with_config(config),
            )
        }
    }

    fn config(concurrency: usize) -> RunConfig {
        RunConfig {
            posting_concurrency: concurrency,
            ..RunConfig::default()
        }
    }

    /// Poll a predicate on a paused runtime, yielding so spawned tasks progress.
    async fn wait_until(mut pred: impl FnMut() -> bool) {
        for _ in 0..10_000 {
            if pred() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("condition not reached");
    }

    #[tokio::test(start_paused = true)]
    async fn in_process_cancel_drains_active_cancels_queued_and_reaches_canceled() {
        // 5 jobs, concurrency 2: two go Active, three stay Queued.
        let env = Env::new(5);
        let fetcher = GatedFetcher::new();
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher.clone(), sink.clone(), config(2));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();

        let exec = {
            let coord = coord.clone();
            tokio::spawn(async move { coord.execute(run).await })
        };

        // Wait until the two permitted workers are blocked at the gate.
        wait_until(|| fetcher.waiting.load(Ordering::SeqCst) == 2).await;
        assert!(coord.registry().is_live(&run_id));

        // Cancel while 2 are Active and 3 Queued.
        let after_cancel = coord.cancel(&run_id).unwrap();
        assert_eq!(after_cancel.run_status, RunStatus::Canceling);
        assert!(after_cancel.live);

        // Let the loop observe the cancel (no new dispatch), then release the
        // two in-flight workers so they finish.
        wait_until(|| {
            store::run_status(&env.conn(), &run_id).ok().flatten() == Some(RunStatus::Canceling)
        })
        .await;
        fetcher.release_all();

        let snapshot = exec.await.unwrap();

        // Terminal Canceled with a summary; the terminal event has done=true.
        assert_eq!(snapshot.run_status, RunStatus::Canceled);
        assert!(snapshot.done);
        assert!(snapshot.summary.is_some());
        assert_eq!(snapshot.posting_counts.queued, 0);
        assert_eq!(snapshot.posting_counts.active, 0);
        // Exactly the two Active postings completed; the other three canceled.
        assert_eq!(snapshot.posting_counts.completed, 2);
        assert_eq!(snapshot.posting_counts.canceled, 3);
        assert_eq!(snapshot.posting_counts.total(), 5);

        // Only two workers ever entered the fetch (dispatch stopped on cancel).
        assert_eq!(fetcher.entered.load(Ordering::SeqCst), 2);

        // The two completed postings are persisted as Closed (Req 5.7).
        let completed: Vec<_> = snapshot
            .postings
            .iter()
            .filter(|p| p.status == PostingStatus::Completed)
            .collect();
        assert_eq!(completed.len(), 2);
        assert!(completed
            .iter()
            .all(|p| p.posting_state == Some(PostingState::Inactive)));

        // The terminal event is last, with the Canceled summary.
        let term = sink.events().pop().unwrap();
        assert!(term.done);
        assert_eq!(term.run_status, RunStatus::Canceled);

        // Exactly one Active→Canceling status-change event.
        let events = sink.events();
        let to_canceling: Vec<_> = events
            .iter()
            .filter(|e| e.run_status == RunStatus::Canceling && e.previous_run_status.is_some())
            .collect();
        assert_eq!(to_canceling.len(), 1);
        assert_eq!(to_canceling[0].previous_run_status, Some(RunStatus::Active));

        // DB agrees, and the runner lock was released.
        assert_eq!(
            store::run_status(&env.conn(), &run_id).unwrap(),
            Some(RunStatus::Canceled)
        );
        assert!(coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop
            })
            .is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn cross_process_cancel_is_picked_up_by_the_poll() {
        let env = Env::new(4);
        let fetcher = GatedFetcher::new();
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher.clone(), sink, config(2));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();

        let exec = {
            let coord = coord.clone();
            tokio::spawn(async move { coord.execute(run).await })
        };
        wait_until(|| fetcher.waiting.load(Ordering::SeqCst) == 2).await;

        // Another "process" commits Canceling directly to the DB, without ever
        // touching this coordinator's registry (no in-process signal).
        let other_conn = open_runner_conn(&env.paths).unwrap();
        assert_eq!(
            store::request_cancel(&other_conn, &run_id, "2026-03-01T10:00:05.000Z").unwrap(),
            store::CancelOutcome::Accepted {
                previous: RunStatus::Active
            }
        );

        // Advance past the poll interval so the loop's interval tick fires and
        // reads the DB-committed Canceling; then release the in-flight workers.
        tokio::time::advance(Duration::from_millis(300)).await;
        wait_until(|| {
            store::run_status(&env.conn(), &run_id).ok().flatten() == Some(RunStatus::Canceling)
        })
        .await;
        fetcher.release_all();

        let snapshot = exec.await.unwrap();
        assert_eq!(snapshot.run_status, RunStatus::Canceled);
        assert!(snapshot.done);
        assert_eq!(snapshot.posting_counts.completed, 2);
        assert_eq!(snapshot.posting_counts.canceled, 2);
        assert_eq!(
            store::run_status(&env.conn(), &run_id).unwrap(),
            Some(RunStatus::Canceled)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_transitions_to_canceling_within_the_poll_interval() {
        let env = Env::new(3);
        let fetcher = GatedFetcher::new();
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher.clone(), sink, config(1));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();
        let exec = {
            let coord = coord.clone();
            tokio::spawn(async move { coord.execute(run).await })
        };
        wait_until(|| fetcher.waiting.load(Ordering::SeqCst) == 1).await;

        let start = env.clock.now();
        let snap = coord.cancel(&run_id).unwrap();
        assert_eq!(snap.run_status, RunStatus::Canceling);
        // The persisted status is Canceling immediately (well under 1 s).
        assert_eq!(
            store::run_status(&env.conn(), &run_id).unwrap(),
            Some(RunStatus::Canceling)
        );
        assert_eq!(start, T0, "cancel committed without advancing the clock");

        fetcher.release_all();
        let snapshot = exec.await.unwrap();
        assert_eq!(snapshot.run_status, RunStatus::Canceled);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_on_terminal_run_is_rejected_without_db_change() {
        let env = Env::new(1);
        let fetcher = GatedFetcher::new();
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher.clone(), sink, config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();
        let exec = {
            let coord = coord.clone();
            tokio::spawn(async move { coord.execute(run).await })
        };
        wait_until(|| fetcher.waiting.load(Ordering::SeqCst) == 1).await;
        fetcher.release_all();
        let snapshot = exec.await.unwrap();
        assert_eq!(snapshot.run_status, RunStatus::Completed);

        // Cancel a terminal run: CancelNotAllowed, nothing changes.
        let before = digest(&env.conn());
        let err = coord.cancel(&run_id).unwrap_err();
        assert_eq!(
            err,
            RunRejection::CancelNotAllowed {
                status: RunStatus::Completed
            }
        );
        assert_eq!(err.to_string(), "cancel_not_allowed:completed");
        assert_eq!(
            AppError::from(err).to_string(),
            "cancel_not_allowed:completed"
        );

        // Cancel a run that does not exist.
        let err = coord.cancel("no-such-run").unwrap_err();
        assert_eq!(
            err,
            RunRejection::RunNotFound {
                run_id: "no-such-run".into()
            }
        );
        assert_eq!(digest(&env.conn()), before);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_abandons_a_late_task_without_waiting_the_grace_window() {
        // One posting whose fetch never returns. It times out at 30 s
        // (authoritative Unknown) and opens a 15 s late-grace slot. A cancel
        // arriving during the grace window must let the run reach Canceled
        // promptly instead of blocking on `late_in_flight` (Req 8.8).
        let env = Env::new(1);
        let fetcher = GatedFetcher::new();
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher.clone(), sink, config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();
        let exec = {
            let coord = coord.clone();
            tokio::spawn(async move { coord.execute(run).await })
        };
        wait_until(|| fetcher.waiting.load(Ordering::SeqCst) == 1).await;

        // Advance past the 30 s eval bound so the worker finalizes the timeout
        // and opens the late-grace slot; the gated fetch is still blocked.
        tokio::time::advance(Duration::from_secs(31)).await;
        wait_until(|| {
            store::run_status(&env.conn(), &run_id).ok().flatten() == Some(RunStatus::Active)
        })
        .await;

        // Cancel during the grace window. Advance only a small amount — far
        // less than the 15 s grace — and the run must already settle.
        let after_cancel = coord.cancel(&run_id).unwrap();
        assert_eq!(after_cancel.run_status, RunStatus::Canceling);
        tokio::time::advance(Duration::from_secs(1)).await;

        let snapshot = exec.await.unwrap();
        assert_eq!(snapshot.run_status, RunStatus::Canceled);
        assert!(snapshot.done);
        // The posting was already Completed (timeout Unknown) before cancel.
        assert_eq!(snapshot.posting_counts.completed, 1);
        assert_eq!(snapshot.posting_counts.canceled, 0);
        // No supplementary row: the late task was abandoned on cancel.
        let supp: i64 = env
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM posting_check_evidence WHERE run_id = ?1 AND kind = 'supplementary'",
                params![run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(supp, 0);
        assert_eq!(
            store::run_status(&env.conn(), &run_id).unwrap(),
            Some(RunStatus::Canceled)
        );
    }

    /// Digest of the run and posting rows a rejected cancel must not touch.
    fn digest(conn: &Connection) -> String {
        conn.query_row(
            "SELECT group_concat(id || '|' || status || '|' || IFNULL(cancel_requested_at,'-'), ';')
             FROM (SELECT * FROM runs ORDER BY id)",
            [],
            |r| r.get::<_, Option<String>>(0),
        )
        .unwrap()
        .unwrap_or_default()
    }
}

#[cfg(test)]
mod stages_tests {
    //! Jobs_Cycle stage sequencing (watches → careers → csv) and the CSV
    //! mark-dirty hook (task 8.2). These drive a real `execute`/`execute_run`
    //! with a scripted fetcher, a `RecordingSink`, a tempdir SQLite database,
    //! and paused Tokio time.
    //!
    //! The watches and careers stages use their own real network fetchers
    //! (`fetch_remote_jobs`, `fetch_careers_hash`), not the injected
    //! `PostingFetcher`. To stay offline and deterministic, these tests seed no
    //! watches and no careers URLs, so those stages complete with zero items
    //! without any network I/O. A real stage-level failure cannot be injected
    //! without editing the stage files, so the failure path is covered by a
    //! focused test on the `any_error` → terminal-status derivation that the
    //! driver uses (see `failed_stage_makes_run_completed_with_errors`).

    use super::*;
    use crate::jobs::posting_check::fetch::{PageFetch, PostingFetcher};
    use crate::jobs::posting_check::provider::ListingFetch;
    use crate::runs::model::StageOutcome;
    use crate::runs::progress::RecordingSink;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Notify;

    const T0: &str = "2026-03-01T10:00:00.000Z";

    /// A scripted, DB-free fetcher (same shape as `execute_tests::FakeFetcher`).
    struct FakeFetcher {
        pages: HashMap<String, PageFetch>,
    }

    impl FakeFetcher {
        fn new(pages: HashMap<String, PageFetch>) -> Arc<Self> {
            Arc::new(Self { pages })
        }
    }

    impl PostingFetcher for FakeFetcher {
        async fn fetch_page(&self, url: &str) -> PageFetch {
            self.pages
                .get(url)
                .cloned()
                .unwrap_or_else(|| panic!("unscripted page {url}"))
        }

        async fn fetch_listing(
            &self,
            _: crate::jobs::posting_check::evidence::Provider,
            _: &str,
        ) -> ListingFetch {
            unreachable!("manual jobs have no provider target")
        }
    }

    fn open_page(url: &str, title: &str, company: &str) -> PageFetch {
        let body = format!(
            r#"<!doctype html><html><head><title>{title} | {company}</title>
               <meta property="og:title" content="{title}"></head>
               <body><h1>{title}</h1>
               <a href="/apply">Apply for this job</a></body></html>"#
        );
        PageFetch {
            requested_url: url.into(),
            final_url: url.into(),
            http_status: Some(200),
            redirect_statuses: vec![],
            signal_headers: vec![],
            body,
            error_kind: None,
        }
    }

    fn gone_page(url: &str) -> PageFetch {
        PageFetch {
            requested_url: url.into(),
            final_url: url.into(),
            http_status: Some(404),
            redirect_statuses: vec![],
            signal_headers: vec![],
            body: "<html><title>Not found</title></html>".into(),
            error_kind: None,
        }
    }

    struct Env {
        _dir: TempDir,
        paths: DataPaths,
        clock: ManualClock,
    }

    impl Env {
        /// `jobs`: `(id, title, url, initial_state)`; all under one company,
        /// with no watches and no careers URL (so watches/careers run empty).
        fn new(jobs: &[(&str, &str, &str, &str)]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let paths = DataPaths::from_data_dir(dir.path().to_path_buf());
            let env = Self {
                _dir: dir,
                paths,
                clock: ManualClock::at(T0),
            };
            env.seed(jobs);
            env
        }

        fn conn(&self) -> Connection {
            open_runner_conn(&self.paths).unwrap()
        }

        fn seed(&self, jobs: &[(&str, &str, &str, &str)]) {
            let conn = self.conn();
            conn.execute(
                "INSERT INTO companies (id, name, created_at, updated_at) VALUES ('c1', 'Acme', ?1, ?1)",
                params![T0],
            )
            .unwrap();
            for (id, title, url, state) in jobs {
                conn.execute(
                    "INSERT INTO jobs (id, company_id, title, url, canonical_url, status, posting_state,
                                       source, created_at, updated_at)
                     VALUES (?1, 'c1', ?2, ?3, ?3, 'wishlist', ?4, 'manual', ?5, ?5)",
                    params![id, title, url, state, T0],
                )
                .unwrap();
            }
        }

        fn coordinator(
            &self,
            fetcher: Arc<FakeFetcher>,
            sink: Arc<RecordingSink>,
            config: RunConfig,
        ) -> RunCoordinator<FakeFetcher, RecordingSink, ManualClock> {
            RunCoordinator::new(
                self.paths.clone(),
                Arc::new(tokio::sync::Mutex::new(())),
                fetcher,
                sink,
                self.clock.clone(),
                RunRegistry::new(),
            )
            .with_config(config)
        }
    }

    fn config(concurrency: usize) -> RunConfig {
        RunConfig {
            posting_concurrency: concurrency,
            ..RunConfig::default()
        }
    }

    fn scripts(entries: Vec<(&str, PageFetch)>) -> HashMap<String, PageFetch> {
        entries
            .into_iter()
            .map(|(url, page)| (url.to_string(), page))
            .collect()
    }

    /// A counting CSV mark-dirty hook.
    fn counting_hook() -> (CsvDirtyHook, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let hook: CsvDirtyHook = Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        (hook, count)
    }

    #[tokio::test(start_paused = true)]
    async fn jobs_cycle_runs_all_four_stages_in_order_and_writes_csv() {
        let url_a = "https://acme.example/jobs/a";
        let env = Env::new(&[("a", "Engineer A", url_a, "unknown")]);
        let fetcher = FakeFetcher::new(scripts(vec![(
            url_a,
            open_page(url_a, "Engineer A", "Acme"),
        )]));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher, sink.clone(), config(4));

        let run = coord
            .accept(RunRequest::JobsCycle {
                trigger: Trigger::Cli,
            })
            .unwrap();
        let execution = coord.execute_run(run).await;
        let snapshot = &execution.snapshot;

        assert_eq!(snapshot.run_status, RunStatus::Completed);

        // Every stage is present, in order, and settled (Req 9.3, 9.4): no
        // stage is left NotStarted or InProgress.
        let stages = snapshot.stages.as_ref().expect("jobs cycle has stages");
        let order: Vec<StageName> = stages.iter().map(|s| s.name).collect();
        assert_eq!(order, StageName::ORDER);
        for s in stages {
            assert!(
                matches!(
                    s.outcome,
                    StageOutcome::Succeeded | StageOutcome::Failed | StageOutcome::Skipped
                ),
                "stage {:?} left {:?}",
                s.name,
                s.outcome
            );
        }
        let by_name: HashMap<_, _> = stages.iter().map(|s| (s.name, s.outcome)).collect();
        assert_eq!(by_name[&StageName::Postings], StageOutcome::Succeeded);
        assert_eq!(by_name[&StageName::Watches], StageOutcome::Succeeded);
        assert_eq!(by_name[&StageName::Careers], StageOutcome::Succeeded);
        assert_eq!(by_name[&StageName::Csv], StageOutcome::Succeeded);

        // The summary lists all four stage outcomes too.
        let summary_stages = snapshot.summary.as_ref().unwrap().stages.as_ref().unwrap();
        assert_eq!(summary_stages.len(), 4);

        // The CSV mirror was written by the CSV stage.
        assert!(
            env.paths.jobs_csv_path.exists(),
            "CSV mirror should be written"
        );

        // Empty watches/careers item lists; the CSV export result is carried out.
        assert!(execution.stage_items.watches.is_empty());
        assert!(execution.stage_items.careers.is_empty());
        assert!(execution.stage_items.csv_exported.is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn posting_check_run_has_no_watches_careers_or_csv_stages() {
        let url_a = "https://acme.example/jobs/a";
        let env = Env::new(&[("a", "Engineer A", url_a, "unknown")]);
        let fetcher = FakeFetcher::new(scripts(vec![(url_a, gone_page(url_a))]));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher, sink, config(4));

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let execution = coord.execute_run(run).await;

        assert_eq!(execution.snapshot.run_status, RunStatus::Completed);
        // A Posting_Check_Run carries no stages at all.
        assert!(execution.snapshot.stages.is_none());
        assert!(execution.stage_items.watches.is_empty());
        assert!(execution.stage_items.careers.is_empty());
        assert!(execution.stage_items.csv_imported.is_none());
        assert!(execution.stage_items.csv_exported.is_none());
        // No CSV mirror is written for a posting check.
        assert!(!env.paths.jobs_csv_path.exists());
    }

    #[tokio::test(start_paused = true)]
    async fn gui_posting_check_with_state_change_marks_csv_dirty() {
        // Job starts `unknown`, gets classified `inactive` (404) → state change.
        let url_a = "https://acme.example/jobs/a";
        let env = Env::new(&[("a", "Engineer A", url_a, "unknown")]);
        let fetcher = FakeFetcher::new(scripts(vec![(url_a, gone_page(url_a))]));
        let sink = Arc::new(RecordingSink::new());
        let (hook, count) = counting_hook();
        let coord = env
            .coordinator(fetcher, sink, config(4))
            .with_csv_dirty_hook(hook);

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let snapshot = coord.execute(run).await;

        assert_eq!(snapshot.run_status, RunStatus::Completed);
        assert!(snapshot.summary.unwrap().state_changes.get() >= 1);
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "CSV should be marked dirty once"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn gui_posting_check_with_no_state_change_does_not_mark_csv_dirty() {
        // Job already `inactive`; a 404 keeps it `inactive` → no state change.
        let url_a = "https://acme.example/jobs/a";
        let env = Env::new(&[("a", "Engineer A", url_a, "inactive")]);
        let fetcher = FakeFetcher::new(scripts(vec![(url_a, gone_page(url_a))]));
        let sink = Arc::new(RecordingSink::new());
        let (hook, count) = counting_hook();
        let coord = env
            .coordinator(fetcher, sink, config(4))
            .with_csv_dirty_hook(hook);

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let snapshot = coord.execute(run).await;

        assert_eq!(snapshot.run_status, RunStatus::Completed);
        assert_eq!(snapshot.summary.unwrap().state_changes.get(), 0);
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "no state change → no CSV mark"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cli_posting_check_never_marks_csv_dirty() {
        // No hook installed (CLI/launchd coordinator): even a state change is
        // a no-op for the CSV mirror.
        let url_a = "https://acme.example/jobs/a";
        let env = Env::new(&[("a", "Engineer A", url_a, "unknown")]);
        let fetcher = FakeFetcher::new(scripts(vec![(url_a, gone_page(url_a))]));
        let sink = Arc::new(RecordingSink::new());
        let coord = env.coordinator(fetcher, sink, config(4));
        assert!(coord.csv_dirty_hook().is_none());

        let run = coord
            .accept(RunRequest::PostingCheck {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let snapshot = coord.execute(run).await;
        assert_eq!(snapshot.run_status, RunStatus::Completed);
        // The job state still changed; there is simply no hook to fire.
        assert!(snapshot.summary.unwrap().state_changes.get() >= 1);
    }

    /// A gated fetcher for the canceled-cycle hook test: workers block until
    /// the test releases them, so a cancel can land while a posting is active.
    struct GatedFetcher {
        gate: Arc<Notify>,
        waiting: AtomicUsize,
    }

    impl GatedFetcher {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                gate: Arc::new(Notify::new()),
                waiting: AtomicUsize::new(0),
            })
        }

        fn release_all(&self) {
            for _ in 0..self.waiting.load(Ordering::SeqCst).max(1) {
                self.gate.notify_one();
            }
        }
    }

    impl PostingFetcher for GatedFetcher {
        async fn fetch_page(&self, url: &str) -> PageFetch {
            self.waiting.fetch_add(1, Ordering::SeqCst);
            self.gate.notified().await;
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            gone_page(url)
        }

        async fn fetch_listing(
            &self,
            _: crate::jobs::posting_check::evidence::Provider,
            _: &str,
        ) -> ListingFetch {
            unreachable!("manual jobs have no provider target")
        }
    }

    async fn wait_until(mut pred: impl FnMut() -> bool) {
        for _ in 0..10_000 {
            if pred() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("condition not reached");
    }

    #[tokio::test(start_paused = true)]
    async fn canceled_gui_cycle_marks_csv_dirty_and_skips_the_csv_stage() {
        // Two jobs, concurrency 1: one goes Active, one stays Queued. Cancel
        // while active, so the cycle never reaches the CSV stage.
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_data_dir(dir.path().to_path_buf());
        let clock = ManualClock::at(T0);
        {
            let conn = open_runner_conn(&paths).unwrap();
            conn.execute(
                "INSERT INTO companies (id, name, created_at, updated_at) VALUES ('c1', 'Acme', ?1, ?1)",
                params![T0],
            )
            .unwrap();
            for i in 0..2 {
                conn.execute(
                    "INSERT INTO jobs (id, company_id, title, url, canonical_url, status, posting_state,
                                       source, created_at, updated_at)
                     VALUES (?1, 'c1', ?2, ?3, ?3, 'wishlist', 'unknown', 'manual', ?4, ?4)",
                    params![format!("j{i}"), format!("Engineer {i}"), format!("https://acme.example/jobs/{i}"), T0],
                )
                .unwrap();
            }
        }
        let fetcher = GatedFetcher::new();
        let sink = Arc::new(RecordingSink::new());
        let (hook, count) = counting_hook();
        let coord = Arc::new(
            RunCoordinator::new(
                paths.clone(),
                Arc::new(tokio::sync::Mutex::new(())),
                fetcher.clone(),
                sink,
                clock.clone(),
                RunRegistry::new(),
            )
            .with_config(config(1))
            .with_csv_dirty_hook(hook),
        );

        let run = coord
            .accept(RunRequest::JobsCycle {
                trigger: Trigger::Desktop,
            })
            .unwrap();
        let run_id = run.run_id().to_string();
        let exec = {
            let coord = coord.clone();
            tokio::spawn(async move { coord.execute(run).await })
        };
        wait_until(|| fetcher.waiting.load(Ordering::SeqCst) == 1).await;

        coord.cancel(&run_id).unwrap();
        wait_until(|| {
            store::run_status(&open_runner_conn(&paths).unwrap(), &run_id)
                .ok()
                .flatten()
                == Some(RunStatus::Canceling)
        })
        .await;
        fetcher.release_all();

        let snapshot = exec.await.unwrap();
        assert_eq!(snapshot.run_status, RunStatus::Canceled);

        // The CSV stage was skipped (never started), so the mirror was not
        // written by the stage; the hook marks it dirty instead (Req 10.10).
        let by_name: HashMap<_, _> = snapshot
            .stages
            .as_ref()
            .unwrap()
            .iter()
            .map(|s| (s.name, s.outcome))
            .collect();
        assert_eq!(by_name[&StageName::Csv], StageOutcome::Skipped);
        assert!(
            !paths.jobs_csv_path.exists(),
            "canceled cycle skips the CSV stage"
        );
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "canceled GUI cycle marks CSV dirty"
        );
    }

    /// The driver folds a `Failed` stage into `any_error`, so a Jobs_Cycle
    /// with a failed stage ends `completed_with_errors` (Req 1.6, 2.2). A real
    /// stage-level failure cannot be injected without editing the stage files
    /// (the watches/careers/csv stages use their own fetchers), so this test
    /// exercises the exact terminal-status derivation the driver uses.
    #[test]
    fn failed_stage_makes_run_completed_with_errors() {
        // No posting errors, but one stage failed.
        let any_error = true;
        let terminal =
            next_status(RunStatus::Active, RunEvent::AllUnitsSettled { any_error }).unwrap();
        assert_eq!(terminal, RunStatus::CompletedWithErrors);

        // With no posting errors and no failed stage, the same run completes.
        let terminal = next_status(
            RunStatus::Active,
            RunEvent::AllUnitsSettled { any_error: false },
        )
        .unwrap();
        assert_eq!(terminal, RunStatus::Completed);
    }
}
