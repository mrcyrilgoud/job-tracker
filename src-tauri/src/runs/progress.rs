//! Progress_Contract v1 types, encoded-length bounds, run summary, and event sinks.
//!
//! Every value published on [`EVENT_NAME`] or returned by the run commands is
//! one of the types below. Wire rules (design.md, "Progress contract v1"):
//! - camelCase field names; enum values follow `runs::model` casing.
//! - Optional fields are absent, never `null` (`skip_serializing_if`).
//! - Strings are bounded by the `MAX_*_BYTES` constants through [`bounded`],
//!   which truncates on a UTF-8 boundary and appends `…` (Req 11.2).
//! - Counts are [`BoundedCount`], saturating at 2^53 − 1 so they stay exact
//!   JavaScript numbers, and `current <= total` (Req 11.3).
//! - The legacy fields `stage`, `message`, `current`, `total`, and `done` keep
//!   their names and meanings (Req 11.6). The final Jobs_Cycle event uses
//!   `stage: "cycle"`.
//!
//! Constructors are plain struct literals; call [`ContractBounds::enforce_bounds`]
//! before publishing or returning a value built from untrusted text.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};

use super::ledger::{PostingEntry, RunLedger};
use super::model::{
    JobIdentity, PostingCounts, PostingState, PostingStatus, RunStatus, RunType, StageName,
    StageOutcome,
};

/// Contract version carried in every event and snapshot (Req 11.7).
pub const PROGRESS_CONTRACT_VERSION: u32 = 1;
/// Unchanged Tauri event channel shared with the legacy runner.
pub const EVENT_NAME: &str = "jobs-runner-progress";

/// Run and job identifiers.
pub const MAX_ID_BYTES: usize = 64;
/// Job title.
pub const MAX_TITLE_BYTES: usize = 300;
/// Company name.
pub const MAX_COMPANY_BYTES: usize = 200;
/// Posting, requested, and final URLs.
pub const MAX_URL_BYTES: usize = 2048;
/// Legacy `message`.
pub const MAX_MESSAGE_BYTES: usize = 500;
/// Classification_Reason, failure reason, run error reason, stage error.
pub const MAX_REASON_BYTES: usize = 500;
/// Reason codes, failure categories, evidence-category values.
pub const MAX_CATEGORY_BYTES: usize = 64;

const ELLIPSIS: &str = "…";

/// Bound `s` to at most `max_bytes` UTF-8 bytes. Longer input is cut on a char
/// boundary and suffixed with `…`; the result, suffix included, never exceeds
/// `max_bytes`. When `max_bytes` is too small for the suffix, the input is only cut.
pub fn bounded(s: impl Into<String>, max_bytes: usize) -> String {
    let mut s = s.into();
    if s.len() <= max_bytes {
        return s;
    }
    let (budget, suffix) = if max_bytes >= ELLIPSIS.len() {
        (max_bytes - ELLIPSIS.len(), ELLIPSIS)
    } else {
        (max_bytes, "")
    };
    let cut = (0..=budget)
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0);
    s.truncate(cut);
    s.push_str(suffix);
    s
}

fn bound_in_place(s: &mut String, max_bytes: usize) {
    if s.len() > max_bytes {
        *s = bounded(std::mem::take(s), max_bytes);
    }
}

fn bound_opt_in_place(s: &mut Option<String>, max_bytes: usize) {
    if let Some(v) = s {
        bound_in_place(v, max_bytes);
    }
}

fn fits(s: &str, max_bytes: usize) -> bool {
    s.len() <= max_bytes
}

fn fits_opt(s: &Option<String>, max_bytes: usize) -> bool {
    s.as_deref().map_or(true, |v| fits(v, max_bytes))
}

/// A count that is always an exact JavaScript number: `0 ..= 2^53 − 1`.
/// Construction saturates; deserializing an out-of-range value fails.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(try_from = "u64", into = "u64")]
pub struct BoundedCount(u64);

impl BoundedCount {
    /// `Number.MAX_SAFE_INTEGER`.
    pub const MAX: u64 = (1u64 << 53) - 1;
    pub const ZERO: BoundedCount = BoundedCount(0);

    pub fn new(v: u64) -> Self {
        Self(v.min(Self::MAX))
    }

    pub fn from_usize(v: usize) -> Self {
        Self::new(u64::try_from(v).unwrap_or(u64::MAX))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("count {0} exceeds 2^53 - 1")]
pub struct CountOutOfRange(pub u64);

impl TryFrom<u64> for BoundedCount {
    type Error = CountOutOfRange;
    fn try_from(v: u64) -> Result<Self, Self::Error> {
        if v > Self::MAX {
            Err(CountOutOfRange(v))
        } else {
            Ok(Self(v))
        }
    }
}

impl From<BoundedCount> for u64 {
    fn from(c: BoundedCount) -> u64 {
        c.0
    }
}

fn clamp_u64(v: &mut u64) {
    *v = (*v).min(BoundedCount::MAX);
}

fn counts_fit(c: &PostingCounts) -> bool {
    [c.queued, c.active, c.completed, c.error, c.canceled]
        .iter()
        .all(|&v| v <= BoundedCount::MAX)
}

fn clamp_counts(c: &mut PostingCounts) {
    for v in [
        &mut c.queued,
        &mut c.active,
        &mut c.completed,
        &mut c.error,
        &mut c.canceled,
    ] {
        clamp_u64(v);
    }
}

/// Bound enforcement shared by every contract type.
pub trait ContractBounds {
    /// Truncate strings and clamp counts so every bound holds.
    fn enforce_bounds(&mut self);
    /// True when every string and count is within its bound.
    fn is_within_bounds(&self) -> bool;
}

/// Legacy `stage` field value: a stage name, or `cycle` for the final Jobs_Cycle event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyStage {
    Postings,
    Watches,
    Careers,
    Csv,
    Cycle,
}

impl LegacyStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Postings => "postings",
            Self::Watches => "watches",
            Self::Careers => "careers",
            Self::Csv => "csv",
            Self::Cycle => "cycle",
        }
    }
}

impl From<StageName> for LegacyStage {
    fn from(s: StageName) -> Self {
        match s {
            StageName::Postings => Self::Postings,
            StageName::Watches => Self::Watches,
            StageName::Careers => Self::Careers,
            StageName::Csv => Self::Csv,
        }
    }
}

/// Provider part of [`EvidenceView`]. Values are the snake_case category names
/// of the posting_check evidence enums (for example `greenhouse`, `listed_open`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceProviderView {
    pub provider: String,
    pub signal: String,
    pub posting_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

impl ContractBounds for EvidenceProviderView {
    fn enforce_bounds(&mut self) {
        bound_in_place(&mut self.provider, MAX_CATEGORY_BYTES);
        bound_in_place(&mut self.signal, MAX_CATEGORY_BYTES);
        bound_in_place(&mut self.posting_id, MAX_ID_BYTES);
    }

    fn is_within_bounds(&self) -> bool {
        fits(&self.provider, MAX_CATEGORY_BYTES)
            && fits(&self.signal, MAX_CATEGORY_BYTES)
            && fits(&self.posting_id, MAX_ID_BYTES)
    }
}

/// Sanitized Check_Evidence as shown by the evidence disclosure (Req 7.6).
/// Plain wire types so `jobs::posting_check::evidence::CheckEvidence` can convert into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceView {
    pub evidence_version: u32,
    pub attempted_at: String,
    pub requested_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirect_statuses: Vec<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<EvidenceProviderView>,
    /// Content signal category names, in canonical order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_category: Option<String>,
}

impl ContractBounds for EvidenceView {
    fn enforce_bounds(&mut self) {
        bound_in_place(&mut self.requested_url, MAX_URL_BYTES);
        bound_opt_in_place(&mut self.final_url, MAX_URL_BYTES);
        if let Some(p) = &mut self.provider {
            p.enforce_bounds();
        }
        for c in &mut self.content {
            bound_in_place(c, MAX_CATEGORY_BYTES);
        }
        bound_opt_in_place(&mut self.failure_category, MAX_CATEGORY_BYTES);
    }

    fn is_within_bounds(&self) -> bool {
        fits(&self.requested_url, MAX_URL_BYTES)
            && fits_opt(&self.final_url, MAX_URL_BYTES)
            && self
                .provider
                .as_ref()
                .map_or(true, |p| p.is_within_bounds())
            && self.content.iter().all(|c| fits(c, MAX_CATEGORY_BYTES))
            && fits_opt(&self.failure_category, MAX_CATEGORY_BYTES)
    }
}

/// One posting's Job_Identity and progress within a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostingProgress {
    pub job_id: String,
    pub title: String,
    pub company_name: String,
    pub posting_url: String,
    pub status: PostingStatus,
    /// Present iff `status == Completed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posting_state: Option<PostingState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    /// Present iff `status` is Completed or Error (Req 3.4, 3.8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_category: Option<String>,
    /// Present only for Completed postings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceView>,
}

impl PostingProgress {
    pub fn identity(&self) -> JobIdentity {
        JobIdentity {
            job_id: self.job_id.clone(),
            title: self.title.clone(),
            company_name: self.company_name.clone(),
            posting_url: self.posting_url.clone(),
        }
    }

    /// Attach evidence to a Completed posting. Ignored for any other status.
    pub fn with_evidence(mut self, evidence: EvidenceView) -> Self {
        if self.status == PostingStatus::Completed {
            let mut evidence = evidence;
            evidence.enforce_bounds();
            self.evidence = Some(evidence);
        }
        self
    }
}

impl From<&PostingEntry> for PostingProgress {
    fn from(e: &PostingEntry) -> Self {
        let completed = e.status == PostingStatus::Completed;
        let has_reason = completed || e.status == PostingStatus::Error;
        let mut p = Self {
            job_id: e.identity.job_id.clone(),
            title: e.identity.title.clone(),
            company_name: e.identity.company_name.clone(),
            posting_url: e.identity.posting_url.clone(),
            status: e.status,
            posting_state: if completed { e.posting_state } else { None },
            reason_code: e.reason_code.clone(),
            reason: if has_reason { e.reason.clone() } else { None },
            failure_category: e.failure_category.clone(),
            evidence: None,
        };
        p.enforce_bounds();
        p
    }
}

impl ContractBounds for PostingProgress {
    fn enforce_bounds(&mut self) {
        bound_in_place(&mut self.job_id, MAX_ID_BYTES);
        bound_in_place(&mut self.title, MAX_TITLE_BYTES);
        bound_in_place(&mut self.company_name, MAX_COMPANY_BYTES);
        bound_in_place(&mut self.posting_url, MAX_URL_BYTES);
        bound_opt_in_place(&mut self.reason_code, MAX_CATEGORY_BYTES);
        bound_opt_in_place(&mut self.reason, MAX_REASON_BYTES);
        bound_opt_in_place(&mut self.failure_category, MAX_CATEGORY_BYTES);
        if let Some(ev) = &mut self.evidence {
            ev.enforce_bounds();
        }
    }

    fn is_within_bounds(&self) -> bool {
        fits(&self.job_id, MAX_ID_BYTES)
            && fits(&self.title, MAX_TITLE_BYTES)
            && fits(&self.company_name, MAX_COMPANY_BYTES)
            && fits(&self.posting_url, MAX_URL_BYTES)
            && fits_opt(&self.reason_code, MAX_CATEGORY_BYTES)
            && fits_opt(&self.reason, MAX_REASON_BYTES)
            && fits_opt(&self.failure_category, MAX_CATEGORY_BYTES)
            && self
                .evidence
                .as_ref()
                .map_or(true, |e| e.is_within_bounds())
    }
}

/// One Jobs_Cycle stage's outcome and item progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageProgress {
    pub name: StageName,
    pub outcome: StageOutcome,
    pub current: BoundedCount,
    pub total: BoundedCount,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl StageProgress {
    pub fn not_started(name: StageName) -> Self {
        Self {
            name,
            outcome: StageOutcome::NotStarted,
            current: BoundedCount::ZERO,
            total: BoundedCount::ZERO,
            error: None,
        }
    }
}

impl ContractBounds for StageProgress {
    fn enforce_bounds(&mut self) {
        self.current = self.current.min(self.total);
        bound_opt_in_place(&mut self.error, MAX_REASON_BYTES);
    }

    fn is_within_bounds(&self) -> bool {
        self.current <= self.total && fits_opt(&self.error, MAX_REASON_BYTES)
    }
}

/// Posting outcome counts in a Run_Summary. All five keys are always present (Req 9.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostingOutcomes {
    pub active: u64,
    /// Posting_State Inactive.
    pub closed: u64,
    pub unknown: u64,
    pub error: u64,
    pub canceled: u64,
}

impl PostingOutcomes {
    pub fn total(&self) -> u64 {
        self.active + self.closed + self.unknown + self.error + self.canceled
    }

    fn fields_mut(&mut self) -> [&mut u64; 5] {
        [
            &mut self.active,
            &mut self.closed,
            &mut self.unknown,
            &mut self.error,
            &mut self.canceled,
        ]
    }

    fn fits(&self) -> bool {
        [
            self.active,
            self.closed,
            self.unknown,
            self.error,
            self.canceled,
        ]
        .iter()
        .all(|&v| v <= BoundedCount::MAX)
    }
}

/// Run_Summary, present on the terminal event and terminal snapshots.
/// Built by [`build_run_summary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub run_id: String,
    pub run_type: RunType,
    pub status: RunStatus,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: BoundedCount,
    pub posting_outcomes: PostingOutcomes,
    /// Entries whose posting state differs from `state_at_start` (Req 9.8).
    pub state_changes: BoundedCount,
    /// Jobs_Cycle only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stages: Option<Vec<StageProgress>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
}

impl ContractBounds for RunSummary {
    fn enforce_bounds(&mut self) {
        bound_in_place(&mut self.run_id, MAX_ID_BYTES);
        bound_opt_in_place(&mut self.source_run_id, MAX_ID_BYTES);
        for v in self.posting_outcomes.fields_mut() {
            clamp_u64(v);
        }
        for s in self.stages.iter_mut().flatten() {
            s.enforce_bounds();
        }
    }

    fn is_within_bounds(&self) -> bool {
        fits(&self.run_id, MAX_ID_BYTES)
            && fits_opt(&self.source_run_id, MAX_ID_BYTES)
            && self.posting_outcomes.fits()
            && self.stages.iter().flatten().all(|s| s.is_within_bounds())
    }
}

/// Run identity carried into a [`RunSummary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummaryHeader {
    pub run_id: String,
    pub run_type: RunType,
    /// Terminal Run_Status the coordinator derived through `lifecycle::next_status`.
    pub status: RunStatus,
    /// Set for a Retry_Run.
    pub source_run_id: Option<String>,
}

/// Terminal timing, computed by the coordinator from its `Clock` (Req 1.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunTiming {
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SummaryError {
    #[error("run summary requires a terminal status, got {}", .0.as_str())]
    NotTerminal(RunStatus),
    #[error("run summary requires a settled ledger ({queued} queued, {active} active)")]
    Unsettled { queued: u64, active: u64 },
}

/// Map one terminal ledger entry to its Run_Summary outcome bucket.
/// `None` for Queued and Active, which have no outcome yet.
fn outcome_slot<'a>(
    outcomes: &'a mut PostingOutcomes,
    entry: &PostingEntry,
) -> Option<&'a mut u64> {
    match entry.status {
        PostingStatus::Queued | PostingStatus::Active => None,
        // The ledger guarantees Completed carries a Posting_State; count a
        // missing one as Unknown rather than dropping the entry.
        PostingStatus::Completed => {
            Some(match entry.posting_state.unwrap_or(PostingState::Unknown) {
                PostingState::Active => &mut outcomes.active,
                PostingState::Inactive => &mut outcomes.closed,
                PostingState::Unknown => &mut outcomes.unknown,
            })
        }
        PostingStatus::Error => Some(&mut outcomes.error),
        PostingStatus::Canceled => Some(&mut outcomes.canceled),
    }
}

/// Build the Run_Summary for a terminal run. Pure: no DB, no clock.
///
/// - `postingOutcomes` always has all five keys (Req 9.1). Completed entries
///   count under their Posting_State (Inactive → `closed`), Error and Canceled
///   under their status, so `postingOutcomes.total() == ledger.total()`.
/// - Each Job_Identity appears exactly once (Req 9.2). The summary wire type
///   carries counts only; the per-posting list travels in the terminal
///   snapshot's `postings`, built from the same ledger. `RunLedger::new`
///   rejects duplicate job ids, so every entry counted here is a distinct
///   Job_Identity counted once.
/// - `state_at_start` maps job id → the persisted `run_postings.state_at_start`
///   text (`active` / `inactive` / `unknown`, or any legacy `jobs.posting_state`
///   value). It is compared as persisted text, the same way
///   `apply_classified_check` decides whether to write a `posting_state_changed`
///   event. `stateChanges` counts Completed entries whose `PostingState::as_str()`
///   differs from that text; a missing key counts as a change. Queued, Active,
///   Error, and Canceled entries never count (Req 9.8).
/// - `stages` is `Some` for Jobs_Cycle and passed through unchanged apart from
///   bound enforcement (Req 9.3, 9.4); Posting_Check_Run passes `None`.
///
/// Rejects a non-terminal `header.status` and a ledger that still has Queued or
/// Active entries, since neither can produce a consistent summary.
pub fn build_run_summary(
    header: &RunSummaryHeader,
    ledger: &RunLedger,
    stages: Option<&[StageProgress]>,
    state_at_start: &HashMap<String, String>,
    timing: &RunTiming,
) -> Result<RunSummary, SummaryError> {
    if !header.status.is_terminal() {
        return Err(SummaryError::NotTerminal(header.status));
    }
    if !ledger.is_settled() {
        let counts = ledger.counts();
        return Err(SummaryError::Unsettled {
            queued: counts.queued,
            active: counts.active,
        });
    }

    let mut outcomes = PostingOutcomes::default();
    let mut state_changes: u64 = 0;
    for entry in ledger.entries() {
        if let Some(slot) = outcome_slot(&mut outcomes, entry) {
            *slot += 1;
        }
        if entry.status == PostingStatus::Completed {
            let now = entry
                .posting_state
                .unwrap_or(PostingState::Unknown)
                .as_str();
            let changed = state_at_start
                .get(entry.job_id())
                .map_or(true, |start| start != now);
            if changed {
                state_changes += 1;
            }
        }
    }
    debug_assert_eq!(outcomes.total(), ledger.total());

    let mut summary = RunSummary {
        run_id: header.run_id.clone(),
        run_type: header.run_type,
        status: header.status,
        started_at: timing.started_at.clone(),
        finished_at: timing.finished_at.clone(),
        duration_ms: BoundedCount::new(timing.duration_ms),
        posting_outcomes: outcomes,
        state_changes: BoundedCount::new(state_changes),
        stages: stages.map(<[StageProgress]>::to_vec),
        source_run_id: header.source_run_id.clone(),
    };
    summary.enforce_bounds();
    Ok(summary)
}

/// One published progress event (Req 1.9, 11.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunProgressEvent {
    /// Always [`PROGRESS_CONTRACT_VERSION`].
    pub version: u32,
    pub run_id: String,
    pub run_type: RunType,
    pub run_status: RunStatus,
    /// Present iff this event carries a run-status transition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_run_status: Option<RunStatus>,
    /// 1-based, +1 per event per run.
    pub seq: BoundedCount,
    pub emitted_at: String,
    // Legacy fields retained for the v1 wire contract.
    pub stage: LegacyStage,
    pub message: String,
    pub current: BoundedCount,
    pub total: BoundedCount,
    /// True only on the terminal event.
    pub done: bool,
    // Run detail.
    pub started_at: String,
    pub elapsed_ms: BoundedCount,
    /// Jobs_Cycle only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stages: Option<Vec<StageProgress>>,
    pub posting_counts: PostingCounts,
    pub posting_total: BoundedCount,
    /// Full list when `seq == 1`, otherwise changed entries only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postings: Option<Vec<PostingProgress>>,
    /// Present iff `run_status == Error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    /// Present iff `done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<RunSummary>,
}

impl RunProgressEvent {
    /// The legacy log line: `[jobs-runner] {stage} ({current}/{total}) {message}`.
    pub fn log_line(&self) -> String {
        format!(
            "[jobs-runner] {} ({}/{}) {}",
            self.stage.as_str(),
            self.current.get(),
            self.total.get(),
            self.message
        )
    }
}

impl ContractBounds for RunProgressEvent {
    fn enforce_bounds(&mut self) {
        bound_in_place(&mut self.run_id, MAX_ID_BYTES);
        bound_in_place(&mut self.message, MAX_MESSAGE_BYTES);
        self.current = self.current.min(self.total);
        clamp_counts(&mut self.posting_counts);
        for s in self.stages.iter_mut().flatten() {
            s.enforce_bounds();
        }
        for p in self.postings.iter_mut().flatten() {
            p.enforce_bounds();
        }
        bound_opt_in_place(&mut self.error_reason, MAX_REASON_BYTES);
        if let Some(s) = &mut self.summary {
            s.enforce_bounds();
        }
    }

    fn is_within_bounds(&self) -> bool {
        fits(&self.run_id, MAX_ID_BYTES)
            && fits(&self.message, MAX_MESSAGE_BYTES)
            && self.current <= self.total
            && counts_fit(&self.posting_counts)
            && self.stages.iter().flatten().all(|s| s.is_within_bounds())
            && self.postings.iter().flatten().all(|p| p.is_within_bounds())
            && fits_opt(&self.error_reason, MAX_REASON_BYTES)
            && self.summary.as_ref().map_or(true, |s| s.is_within_bounds())
    }
}

/// Full run state rebuilt from SQLite (`get_run_cmd`, `get_current_run_cmd`).
/// Same fields as [`RunProgressEvent`] minus `previousRunStatus`, `emittedAt`,
/// and the delta `postings`, plus the full posting list and snapshot flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSnapshot {
    pub version: u32,
    pub run_id: String,
    pub run_type: RunType,
    pub run_status: RunStatus,
    pub seq: BoundedCount,
    pub stage: LegacyStage,
    pub message: String,
    pub current: BoundedCount,
    pub total: BoundedCount,
    pub done: bool,
    pub started_at: String,
    pub elapsed_ms: BoundedCount,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stages: Option<Vec<StageProgress>>,
    pub posting_counts: PostingCounts,
    pub posting_total: BoundedCount,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<RunSummary>,
    /// Always the full list, in ordinal order.
    pub postings: Vec<PostingProgress>,
    /// True when this process owns the run and will emit events for it.
    pub live: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
    pub dismissed: bool,
}

impl ContractBounds for RunSnapshot {
    fn enforce_bounds(&mut self) {
        bound_in_place(&mut self.run_id, MAX_ID_BYTES);
        bound_in_place(&mut self.message, MAX_MESSAGE_BYTES);
        self.current = self.current.min(self.total);
        clamp_counts(&mut self.posting_counts);
        for s in self.stages.iter_mut().flatten() {
            s.enforce_bounds();
        }
        bound_opt_in_place(&mut self.error_reason, MAX_REASON_BYTES);
        if let Some(s) = &mut self.summary {
            s.enforce_bounds();
        }
        for p in &mut self.postings {
            p.enforce_bounds();
        }
        bound_opt_in_place(&mut self.source_run_id, MAX_ID_BYTES);
    }

    fn is_within_bounds(&self) -> bool {
        fits(&self.run_id, MAX_ID_BYTES)
            && fits(&self.message, MAX_MESSAGE_BYTES)
            && self.current <= self.total
            && counts_fit(&self.posting_counts)
            && self.stages.iter().flatten().all(|s| s.is_within_bounds())
            && fits_opt(&self.error_reason, MAX_REASON_BYTES)
            && self.summary.as_ref().map_or(true, |s| s.is_within_bounds())
            && self.postings.iter().all(|p| p.is_within_bounds())
            && fits_opt(&self.source_run_id, MAX_ID_BYTES)
    }
}

/// Returned by `start_run_cmd` and `retry_run_cmd` at the accept point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunAccepted {
    pub run_id: String,
    pub snapshot: RunSnapshot,
}

/// Destination for published progress events.
pub trait RunEventSink: Send + Sync {
    fn publish(&self, event: &RunProgressEvent);
}

/// Emits on the Tauri [`EVENT_NAME`] channel and writes the legacy log line.
pub struct TauriSink<R: Runtime = tauri::Wry>(pub AppHandle<R>);

impl<R: Runtime> TauriSink<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self(app)
    }
}

impl<R: Runtime> RunEventSink for TauriSink<R> {
    fn publish(&self, event: &RunProgressEvent) {
        if let Err(err) = self.0.emit(EVENT_NAME, event) {
            log::warn!("[jobs-runner] failed to emit {EVENT_NAME}: {err}");
        }
        log::info!("{}", event.log_line());
    }
}

/// Log-only sink for CLI and launchd runs (the current behavior without an `AppHandle`).
#[derive(Debug, Default, Clone, Copy)]
pub struct LogSink;

impl RunEventSink for LogSink {
    fn publish(&self, event: &RunProgressEvent) {
        log::info!("{}", event.log_line());
    }
}

/// Records every published event in order. For tests.
#[derive(Debug, Default)]
pub struct RecordingSink(Mutex<Vec<RunProgressEvent>>);

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<RunProgressEvent>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Copy of every event recorded so far, in publish order.
    pub fn events(&self) -> Vec<RunProgressEvent> {
        self.lock().clone()
    }

    /// Drain the recorded events.
    pub fn take(&self) -> Vec<RunProgressEvent> {
        std::mem::take(&mut *self.lock())
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

impl RunEventSink for RecordingSink {
    fn publish(&self, event: &RunProgressEvent) {
        self.lock().push(event.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::ledger::{RunLedger, TransitionDetail};
    use serde_json::{json, Value};

    fn has_null(v: &Value) -> bool {
        match v {
            Value::Null => true,
            Value::Array(a) => a.iter().any(has_null),
            Value::Object(o) => o.values().any(has_null),
            _ => false,
        }
    }

    fn keys(v: &Value) -> Vec<&str> {
        let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        k.sort_unstable();
        k
    }

    fn ident(id: &str) -> JobIdentity {
        JobIdentity {
            job_id: id.into(),
            title: format!("Engineer {id}"),
            company_name: "Acme".into(),
            posting_url: format!("https://example.com/jobs/{id}"),
        }
    }

    fn event() -> RunProgressEvent {
        RunProgressEvent {
            version: PROGRESS_CONTRACT_VERSION,
            run_id: "run-1".into(),
            run_type: RunType::PostingCheck,
            run_status: RunStatus::Active,
            previous_run_status: None,
            seq: BoundedCount::new(2),
            emitted_at: "2026-01-01T00:00:01Z".into(),
            stage: LegacyStage::Postings,
            message: "Checking job postings".into(),
            current: BoundedCount::new(1),
            total: BoundedCount::new(3),
            done: false,
            started_at: "2026-01-01T00:00:00Z".into(),
            elapsed_ms: BoundedCount::new(1000),
            stages: None,
            posting_counts: PostingCounts {
                queued: 1,
                active: 1,
                completed: 1,
                ..Default::default()
            },
            posting_total: BoundedCount::new(3),
            postings: None,
            error_reason: None,
            summary: None,
        }
    }

    fn summary() -> RunSummary {
        RunSummary {
            run_id: "run-1".into(),
            run_type: RunType::JobsCycle,
            status: RunStatus::Completed,
            started_at: "t0".into(),
            finished_at: "t1".into(),
            duration_ms: BoundedCount::new(5),
            posting_outcomes: PostingOutcomes::default(),
            state_changes: BoundedCount::ZERO,
            stages: None,
            source_run_id: None,
        }
    }

    #[test]
    fn bounded_keeps_short_strings() {
        assert_eq!(bounded("abc", 3), "abc");
        assert_eq!(bounded("", 0), "");
        assert_eq!(bounded("héllo", 64), "héllo");
    }

    #[test]
    fn bounded_truncates_ascii_with_ellipsis_within_limit() {
        let out = bounded("a".repeat(600), MAX_MESSAGE_BYTES);
        assert_eq!(out.len(), MAX_MESSAGE_BYTES);
        assert!(out.ends_with('…'));
        assert_eq!(out, format!("{}…", "a".repeat(MAX_MESSAGE_BYTES - 3)));
    }

    #[test]
    fn bounded_cuts_multibyte_text_on_char_boundary() {
        // "é" is 2 bytes, "🦀" is 4 bytes.
        let out = bounded("éééé", 7); // budget 4 → "éé…" (7 bytes)
        assert_eq!(out, "éé…");
        let out = bounded("éééé", 6); // budget 3 → cut inside 2nd é → "é…"
        assert_eq!(out, "é…");
        assert_eq!(out.len(), 5);
        let out = bounded("🦀🦀🦀", 9); // budget 6 → "🦀…"
        assert_eq!(out, "🦀…");
        for max in 0..20 {
            let out = bounded("a🦀é日本語🦀", max);
            assert!(out.len() <= max, "max {max}: {out:?}");
        }
    }

    #[test]
    fn bounded_without_room_for_ellipsis_only_cuts() {
        assert_eq!(bounded("abcdef", 2), "ab");
        assert_eq!(bounded("🦀", 2), "");
        assert_eq!(bounded("abc", 0), "");
    }

    #[test]
    fn bounded_count_saturates_and_rejects_out_of_range_on_deserialize() {
        assert_eq!(BoundedCount::MAX, 9_007_199_254_740_991);
        assert_eq!(BoundedCount::new(u64::MAX).get(), BoundedCount::MAX);
        assert_eq!(
            BoundedCount::new(BoundedCount::MAX + 1).get(),
            BoundedCount::MAX
        );
        assert_eq!(BoundedCount::new(7).get(), 7);
        assert_eq!(
            BoundedCount::from_usize(usize::MAX).get(),
            BoundedCount::MAX
        );

        assert_eq!(
            serde_json::to_value(BoundedCount::new(42)).unwrap(),
            json!(42)
        );
        let max: BoundedCount = serde_json::from_value(json!(BoundedCount::MAX)).unwrap();
        assert_eq!(max.get(), BoundedCount::MAX);
        assert!(serde_json::from_value::<BoundedCount>(json!(BoundedCount::MAX + 1)).is_err());
        assert!(serde_json::from_value::<BoundedCount>(json!(-1)).is_err());
    }

    #[test]
    fn event_omits_absent_optionals_and_keeps_legacy_fields() {
        let v = serde_json::to_value(event()).unwrap();
        assert!(!has_null(&v));
        assert_eq!(
            keys(&v),
            [
                "current",
                "done",
                "elapsedMs",
                "emittedAt",
                "message",
                "postingCounts",
                "postingTotal",
                "runId",
                "runStatus",
                "runType",
                "seq",
                "stage",
                "startedAt",
                "total",
                "version",
            ]
        );
        assert_eq!(v["version"], json!(1));
        assert_eq!(v["runType"], json!("postingCheck"));
        assert_eq!(v["runStatus"], json!("active"));
        assert_eq!(v["stage"], json!("postings"));
        assert_eq!(v["message"], json!("Checking job postings"));
        assert_eq!(v["current"], json!(1));
        assert_eq!(v["total"], json!(3));
        assert_eq!(v["done"], json!(false));
        let back: RunProgressEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, event());
    }

    #[test]
    fn final_cycle_event_uses_cycle_stage_and_carries_summary() {
        let mut e = event();
        e.run_type = RunType::JobsCycle;
        e.previous_run_status = Some(RunStatus::Active);
        e.run_status = RunStatus::Completed;
        e.stage = LegacyStage::Cycle;
        e.message = "Jobs cycle complete".into();
        e.current = BoundedCount::new(1);
        e.total = BoundedCount::new(1);
        e.done = true;
        e.stages = Some(
            StageName::ORDER
                .into_iter()
                .map(StageProgress::not_started)
                .collect(),
        );
        e.summary = Some(summary());

        let v = serde_json::to_value(&e).unwrap();
        assert!(!has_null(&v));
        assert_eq!(v["stage"], json!("cycle"));
        assert_eq!(v["previousRunStatus"], json!("active"));
        assert_eq!(
            v["stages"][0],
            json!({"name": "postings", "outcome": "not_started", "current": 0, "total": 0})
        );
        assert_eq!(
            v["summary"]["postingOutcomes"],
            json!({"active": 0, "closed": 0, "unknown": 0, "error": 0, "canceled": 0})
        );
        assert!(v["summary"].get("sourceRunId").is_none());
        assert_eq!(
            e.log_line(),
            "[jobs-runner] cycle (1/1) Jobs cycle complete"
        );
        let back: RunProgressEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn legacy_stage_mirrors_stage_names() {
        for name in StageName::ORDER {
            assert_eq!(LegacyStage::from(name).as_str(), name.as_str());
        }
        assert_eq!(
            serde_json::to_value(LegacyStage::Cycle).unwrap(),
            json!("cycle")
        );
    }

    #[test]
    fn enforce_bounds_truncates_strings_and_clamps_counts() {
        let mut e = event();
        e.run_id = "r".repeat(100);
        e.message = "日".repeat(400);
        e.current = BoundedCount::new(10);
        e.total = BoundedCount::new(3);
        e.posting_counts.completed = u64::MAX;
        e.error_reason = Some("x".repeat(1000));
        e.postings = Some(vec![PostingProgress {
            title: "t".repeat(400),
            ..PostingProgress::from(&RunLedger::new(vec![ident("a")]).unwrap().entries()[0])
        }]);
        assert!(!e.is_within_bounds());

        e.enforce_bounds();
        assert!(e.is_within_bounds());
        assert!(e.run_id.len() <= MAX_ID_BYTES);
        assert!(e.message.len() <= MAX_MESSAGE_BYTES && e.message.ends_with('…'));
        assert_eq!(e.current.get(), 3);
        assert_eq!(e.posting_counts.completed, BoundedCount::MAX);
        assert!(e.error_reason.as_ref().unwrap().len() <= MAX_REASON_BYTES);
        assert!(e.postings.as_ref().unwrap()[0].title.len() <= MAX_TITLE_BYTES);
    }

    #[test]
    fn posting_progress_from_entry_follows_status_optionality() {
        let mut l = RunLedger::new(vec![ident("q"), ident("c"), ident("e"), ident("x")]).unwrap();
        l.transition("c", PostingStatus::Active, TransitionDetail::started("t1"))
            .unwrap();
        l.transition(
            "c",
            PostingStatus::Completed,
            TransitionDetail::completed(
                PostingState::Inactive,
                "http_404",
                "Closed: posting returned HTTP 404",
                "t2",
            ),
        )
        .unwrap();
        l.transition(
            "e",
            PostingStatus::Error,
            TransitionDetail::error("internal", "worker panicked", "t3"),
        )
        .unwrap();
        l.transition(
            "x",
            PostingStatus::Canceled,
            TransitionDetail::canceled("t4"),
        )
        .unwrap();
        let rows: Vec<PostingProgress> = l.entries().iter().map(PostingProgress::from).collect();

        let q = serde_json::to_value(&rows[0]).unwrap();
        assert_eq!(
            keys(&q),
            ["companyName", "jobId", "postingUrl", "status", "title"]
        );
        assert_eq!(q["status"], json!("queued"));
        assert_eq!(rows[0].identity(), ident("q"));

        let c = serde_json::to_value(&rows[1]).unwrap();
        assert_eq!(c["status"], json!("completed"));
        assert_eq!(c["postingState"], json!("inactive"));
        assert_eq!(c["reasonCode"], json!("http_404"));
        assert_eq!(c["reason"], json!("Closed: posting returned HTTP 404"));

        let e = serde_json::to_value(&rows[2]).unwrap();
        assert_eq!(e["status"], json!("error"));
        assert!(e.get("postingState").is_none());
        assert_eq!(e["reason"], json!("worker panicked"));
        assert_eq!(e["failureCategory"], json!("internal"));

        let x = serde_json::to_value(&rows[3]).unwrap();
        assert_eq!(x["status"], json!("canceled"));
        assert!(x.get("reason").is_none() && x.get("postingState").is_none());

        for v in [&q, &c, &e, &x] {
            assert!(!has_null(v));
        }
    }

    #[test]
    fn posting_progress_from_entry_applies_identity_bounds() {
        let long = JobIdentity {
            job_id: "j".repeat(80),
            title: "🦀".repeat(100),
            company_name: "c".repeat(250),
            posting_url: format!("https://example.com/{}", "p".repeat(3000)),
        };
        let l = RunLedger::new(vec![long]).unwrap();
        let p = PostingProgress::from(&l.entries()[0]);
        assert!(p.is_within_bounds());
        assert!(p.job_id.len() <= MAX_ID_BYTES);
        assert!(p.title.len() <= MAX_TITLE_BYTES && p.title.ends_with('…'));
        assert!(p.company_name.len() <= MAX_COMPANY_BYTES);
        assert!(p.posting_url.len() <= MAX_URL_BYTES);
    }

    #[test]
    fn evidence_attaches_only_to_completed_and_omits_empty_fields() {
        let ev = EvidenceView {
            evidence_version: 1,
            attempted_at: "t1".into(),
            requested_url: "https://example.com/jobs/1".into(),
            final_url: None,
            http_status: Some(404),
            redirect_statuses: vec![],
            provider: None,
            content: vec![],
            failure_category: None,
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            keys(&v),
            [
                "attemptedAt",
                "evidenceVersion",
                "httpStatus",
                "requestedUrl"
            ]
        );

        let mut l = RunLedger::new(vec![ident("a"), ident("b")]).unwrap();
        l.transition("a", PostingStatus::Active, TransitionDetail::started("t"))
            .unwrap();
        l.transition(
            "a",
            PostingStatus::Completed,
            TransitionDetail::completed(PostingState::Inactive, "c", "r", "t"),
        )
        .unwrap();
        let done = PostingProgress::from(l.get("a").unwrap()).with_evidence(ev.clone());
        assert_eq!(done.evidence.as_ref(), Some(&ev));
        let queued = PostingProgress::from(l.get("b").unwrap()).with_evidence(ev);
        assert!(queued.evidence.is_none());

        let full = EvidenceView {
            final_url: Some("https://example.com/careers".into()),
            redirect_statuses: vec![301, 302],
            provider: Some(EvidenceProviderView {
                provider: "greenhouse".into(),
                signal: "absent_from_listing".into(),
                posting_id: "127817".into(),
                http_status: None,
            }),
            content: vec!["generic_careers".into()],
            failure_category: Some("timeout".into()),
            ..done.evidence.clone().unwrap()
        };
        let v = serde_json::to_value(&full).unwrap();
        assert!(!has_null(&v));
        assert_eq!(v["redirectStatuses"], json!([301, 302]));
        assert_eq!(
            v["provider"],
            json!({"provider": "greenhouse", "signal": "absent_from_listing", "postingId": "127817"})
        );
        let back: EvidenceView = serde_json::from_value(v).unwrap();
        assert_eq!(back, full);
    }

    #[test]
    fn snapshot_and_accepted_serialize_required_fields() {
        let l = RunLedger::new(vec![ident("a")]).unwrap();
        let snapshot = RunSnapshot {
            version: PROGRESS_CONTRACT_VERSION,
            run_id: "run-1".into(),
            run_type: RunType::PostingCheck,
            run_status: RunStatus::Queued,
            seq: BoundedCount::new(1),
            stage: LegacyStage::Postings,
            message: "Queued".into(),
            current: BoundedCount::ZERO,
            total: BoundedCount::new(1),
            done: false,
            started_at: "t0".into(),
            elapsed_ms: BoundedCount::ZERO,
            stages: None,
            posting_counts: l.counts(),
            posting_total: BoundedCount::new(l.total()),
            error_reason: None,
            summary: None,
            postings: l.entries().iter().map(PostingProgress::from).collect(),
            live: true,
            source_run_id: None,
            dismissed: false,
        };
        let accepted = RunAccepted {
            run_id: "run-1".into(),
            snapshot: snapshot.clone(),
        };
        let v = serde_json::to_value(&accepted).unwrap();
        assert!(!has_null(&v));
        assert_eq!(keys(&v), ["runId", "snapshot"]);
        let s = &v["snapshot"];
        assert!(s.get("previousRunStatus").is_none() && s.get("emittedAt").is_none());
        assert_eq!(s["live"], json!(true));
        assert_eq!(s["dismissed"], json!(false));
        assert_eq!(s["postings"].as_array().unwrap().len(), 1);
        assert_eq!(s["postingCounts"]["queued"], json!(1));
        let back: RunAccepted = serde_json::from_value(v).unwrap();
        assert_eq!(back, accepted);
        assert!(snapshot.is_within_bounds());
    }

    #[test]
    fn recording_sink_keeps_publish_order() {
        let sink = RecordingSink::new();
        assert!(sink.is_empty());
        let mut first = event();
        first.seq = BoundedCount::new(1);
        let second = event();
        sink.publish(&first);
        sink.publish(&second);
        assert_eq!(sink.len(), 2);
        let seqs: Vec<u64> = sink.events().iter().map(|e| e.seq.get()).collect();
        assert_eq!(seqs, [1, 2]);
        assert_eq!(sink.take().len(), 2);
        assert!(sink.is_empty());

        // LogSink never panics and uses the legacy line format.
        LogSink.publish(&second);
        assert_eq!(
            second.log_line(),
            "[jobs-runner] postings (1/3) Checking job postings"
        );
    }

    #[test]
    fn sinks_are_object_safe_and_thread_safe() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RecordingSink>();
        assert_send_sync::<LogSink>();
        assert_send_sync::<TauriSink>();
        let sinks: Vec<Box<dyn RunEventSink>> =
            vec![Box::new(LogSink), Box::new(RecordingSink::new())];
        for s in &sinks {
            s.publish(&event());
        }
    }

    // ---- build_run_summary ----

    fn header(run_type: RunType, status: RunStatus) -> RunSummaryHeader {
        RunSummaryHeader {
            run_id: "run-9".into(),
            run_type,
            status,
            source_run_id: None,
        }
    }

    fn timing() -> RunTiming {
        RunTiming {
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: "2026-01-01T00:00:05Z".into(),
            duration_ms: 5000,
        }
    }

    fn starts(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn complete(l: &mut RunLedger, id: &str, state: PostingState) {
        l.transition(id, PostingStatus::Active, TransitionDetail::started("t1"))
            .unwrap();
        l.transition(
            id,
            PostingStatus::Completed,
            TransitionDetail::completed(state, "c", "reason", "t2"),
        )
        .unwrap();
    }

    #[test]
    fn summary_of_empty_run_reports_all_zero_keys() {
        let l = RunLedger::new(vec![]).unwrap();
        let s = build_run_summary(
            &header(RunType::PostingCheck, RunStatus::Completed),
            &l,
            None,
            &HashMap::new(),
            &timing(),
        )
        .unwrap();
        assert_eq!(s.posting_outcomes, PostingOutcomes::default());
        assert_eq!(s.state_changes, BoundedCount::ZERO);
        assert_eq!(s.duration_ms.get(), 5000);
        assert_eq!(s.started_at, "2026-01-01T00:00:00Z");
        assert_eq!(s.finished_at, "2026-01-01T00:00:05Z");

        let v = serde_json::to_value(&s).unwrap();
        assert!(!has_null(&v));
        assert_eq!(
            v["postingOutcomes"],
            json!({"active": 0, "closed": 0, "unknown": 0, "error": 0, "canceled": 0})
        );
        assert_eq!(v["stateChanges"], json!(0));
        assert!(v.get("stages").is_none() && v.get("sourceRunId").is_none());
    }

    #[test]
    fn summary_counts_each_entry_in_exactly_one_outcome() {
        let ids = ["a", "b", "c", "d", "e", "f"];
        let mut l = RunLedger::new(ids.iter().map(|id| ident(id)).collect()).unwrap();
        complete(&mut l, "a", PostingState::Active);
        complete(&mut l, "b", PostingState::Inactive);
        complete(&mut l, "c", PostingState::Unknown);
        complete(&mut l, "d", PostingState::Active);
        l.transition(
            "e",
            PostingStatus::Error,
            TransitionDetail::error("internal", "boom", "t"),
        )
        .unwrap();
        l.transition(
            "f",
            PostingStatus::Canceled,
            TransitionDetail::canceled("t"),
        )
        .unwrap();

        let mut h = header(RunType::PostingCheck, RunStatus::CompletedWithErrors);
        h.source_run_id = Some("run-1".into());
        let start = starts(&[
            ("a", "active"),
            ("b", "active"),
            ("c", "unknown"),
            ("d", "active"),
            ("e", "active"),
            ("f", "active"),
        ]);
        let s = build_run_summary(&h, &l, None, &start, &timing()).unwrap();

        assert_eq!(
            s.posting_outcomes,
            PostingOutcomes {
                active: 2,
                closed: 1,
                unknown: 1,
                error: 1,
                canceled: 1
            }
        );
        assert_eq!(s.posting_outcomes.total(), l.total());
        assert_eq!(
            s.state_changes.get(),
            1,
            "only b changed (active -> inactive)"
        );
        assert_eq!(s.status, RunStatus::CompletedWithErrors);
        assert_eq!(s.run_id, "run-9");
        assert_eq!(
            serde_json::to_value(&s).unwrap()["sourceRunId"],
            json!("run-1")
        );
        assert!(s.is_within_bounds());
    }

    #[test]
    fn state_changes_include_unknown_after_active_and_skip_unfinished_entries() {
        let ids = ["u", "same", "new", "missing", "err", "can"];
        let mut l = RunLedger::new(ids.iter().map(|id| ident(id)).collect()).unwrap();
        complete(&mut l, "u", PostingState::Unknown); // active -> unknown: a change
        complete(&mut l, "same", PostingState::Inactive); // inactive -> inactive: no change
        complete(&mut l, "new", PostingState::Active); // legacy text -> active: a change
        complete(&mut l, "missing", PostingState::Active); // no start state recorded: a change
        l.transition("err", PostingStatus::Active, TransitionDetail::started("t"))
            .unwrap();
        l.transition(
            "err",
            PostingStatus::Error,
            TransitionDetail::error("persistence", "db locked", "t"),
        )
        .unwrap();
        l.transition(
            "can",
            PostingStatus::Canceled,
            TransitionDetail::canceled("t"),
        )
        .unwrap();

        // Error and Canceled entries never count as changes, whatever their start state.
        let start = starts(&[
            ("u", "active"),
            ("same", "inactive"),
            ("new", "open"),
            ("err", "active"),
            ("can", "active"),
        ]);
        let s = build_run_summary(
            &header(RunType::PostingCheck, RunStatus::Canceled),
            &l,
            None,
            &start,
            &timing(),
        )
        .unwrap();
        assert_eq!(s.state_changes.get(), 3);
    }

    #[test]
    fn jobs_cycle_summary_passes_stages_through() {
        let mut l = RunLedger::new(vec![ident("a")]).unwrap();
        complete(&mut l, "a", PostingState::Active);
        let stages = vec![
            StageProgress {
                name: StageName::Postings,
                outcome: StageOutcome::Succeeded,
                current: BoundedCount::new(1),
                total: BoundedCount::new(1),
                error: None,
            },
            StageProgress {
                name: StageName::Watches,
                outcome: StageOutcome::Failed,
                current: BoundedCount::new(2),
                total: BoundedCount::new(3),
                error: Some("e".repeat(900)),
            },
            StageProgress {
                outcome: StageOutcome::Succeeded,
                ..StageProgress::not_started(StageName::Careers)
            },
            StageProgress {
                outcome: StageOutcome::Skipped,
                ..StageProgress::not_started(StageName::Csv)
            },
        ];
        let s = build_run_summary(
            &header(RunType::JobsCycle, RunStatus::CompletedWithErrors),
            &l,
            Some(&stages),
            &starts(&[("a", "active")]),
            &timing(),
        )
        .unwrap();

        let out = s.stages.as_ref().unwrap();
        assert_eq!(out.len(), 4);
        let names: Vec<StageName> = out.iter().map(|st| st.name).collect();
        assert_eq!(names, StageName::ORDER);
        let outcomes: Vec<StageOutcome> = out.iter().map(|st| st.outcome).collect();
        assert_eq!(
            outcomes,
            [
                StageOutcome::Succeeded,
                StageOutcome::Failed,
                StageOutcome::Succeeded,
                StageOutcome::Skipped
            ]
        );
        // Bounds applied to passed-through stages.
        assert!(out[1].error.as_ref().unwrap().len() <= MAX_REASON_BYTES);
        assert_eq!(
            s.posting_outcomes,
            PostingOutcomes {
                active: 1,
                ..Default::default()
            }
        );
        assert_eq!(s.state_changes, BoundedCount::ZERO);
        assert!(s.is_within_bounds());
    }

    #[test]
    fn summary_rejects_non_terminal_status_and_unsettled_ledger() {
        let mut l = RunLedger::new(vec![ident("a"), ident("b")]).unwrap();
        l.transition("a", PostingStatus::Active, TransitionDetail::started("t"))
            .unwrap();
        let none = HashMap::new();

        for status in [RunStatus::Queued, RunStatus::Active, RunStatus::Canceling] {
            let err = build_run_summary(
                &header(RunType::PostingCheck, status),
                &l,
                None,
                &none,
                &timing(),
            )
            .unwrap_err();
            assert_eq!(err, SummaryError::NotTerminal(status));
        }
        let err = build_run_summary(
            &header(RunType::PostingCheck, RunStatus::Error),
            &l,
            None,
            &none,
            &timing(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            SummaryError::Unsettled {
                queued: 1,
                active: 1
            }
        );
    }

    #[test]
    fn summary_bounds_run_ids_and_saturates_duration() {
        let l = RunLedger::new(vec![]).unwrap();
        let h = RunSummaryHeader {
            run_id: "r".repeat(100),
            run_type: RunType::PostingCheck,
            status: RunStatus::Completed,
            source_run_id: Some("s".repeat(100)),
        };
        let t = RunTiming {
            duration_ms: u64::MAX,
            ..timing()
        };
        let s = build_run_summary(&h, &l, None, &HashMap::new(), &t).unwrap();
        assert!(s.is_within_bounds());
        assert!(s.run_id.len() <= MAX_ID_BYTES);
        assert!(s.source_run_id.as_ref().unwrap().len() <= MAX_ID_BYTES);
        assert_eq!(s.duration_ms.get(), BoundedCount::MAX);
    }

    #[test]
    fn golden_contract_corpus_is_deterministic_and_matches_checked_in_fixture() {
        let corpus = json!({
            "version": 1,
            "bounds": {
                "MAX_ID_BYTES": MAX_ID_BYTES,
                "MAX_TITLE_BYTES": MAX_TITLE_BYTES,
                "MAX_COMPANY_BYTES": MAX_COMPANY_BYTES,
                "MAX_URL_BYTES": MAX_URL_BYTES,
                "MAX_MESSAGE_BYTES": MAX_MESSAGE_BYTES,
                "MAX_REASON_BYTES": MAX_REASON_BYTES,
                "MAX_CATEGORY_BYTES": MAX_CATEGORY_BYTES,
                "MAX_COUNT": BoundedCount::MAX,
            },
            "events": [serde_json::to_value(event()).unwrap(), {
                "version": 1, "runId": "cycle-1", "runType": "jobsCycle", "runStatus": "completed",
                "previousRunStatus": "active", "seq": 4, "emittedAt": "2026-01-01T00:00:05Z",
                "stage": "cycle", "message": "Jobs cycle complete", "current": 1, "total": 1,
                "done": true, "startedAt": "2026-01-01T00:00:00Z", "elapsedMs": 5000,
                "stages": [
                    {"name": "postings", "outcome": "succeeded", "current": 1, "total": 1},
                    {"name": "watches", "outcome": "skipped", "current": 0, "total": 0},
                    {"name": "careers", "outcome": "not_started", "current": 0, "total": 0},
                    {"name": "csv", "outcome": "succeeded", "current": 1, "total": 1}
                ],
                "postingCounts": {"queued": 0, "active": 0, "completed": 1, "error": 0, "canceled": 0},
                "postingTotal": 1,
                "summary": {"runId": "cycle-1", "runType": "jobsCycle", "status": "completed",
                    "startedAt": "2026-01-01T00:00:00Z", "finishedAt": "2026-01-01T00:00:05Z", "durationMs": 5000,
                    "postingOutcomes": {"active": 1, "closed": 0, "unknown": 0, "error": 0, "canceled": 0}, "stateChanges": 1}
            }],
            "snapshots": [{
                "version": 1, "runId": "run-1", "runType": "postingCheck", "runStatus": "active", "seq": 2,
                "stage": "postings", "message": "Checking job postings", "current": 1, "total": 3, "done": false,
                "startedAt": "2026-01-01T00:00:00Z", "elapsedMs": 1000,
                "postingCounts": {"queued": 1, "active": 1, "completed": 1, "error": 0, "canceled": 0},
                "postingTotal": 3, "postings": [], "live": false, "dismissed": false
            }],
            "eligibility": [
                {"status": "queued", "retry": false, "attention": false},
                {"status": "active", "retry": false, "attention": false},
                {"status": "completed", "postingState": "active", "retry": false, "attention": false},
                {"status": "completed", "postingState": "inactive", "retry": false, "attention": false},
                {"status": "completed", "postingState": "unknown", "retry": true, "attention": true},
                {"status": "error", "retry": true, "attention": true},
                {"status": "canceled", "retry": true, "attention": false}
            ]
        });
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("../desktop/src/lib/__fixtures__/run-contract-corpus.json");
        let checked_in: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        if std::env::var_os("UPDATE_CONTRACT_CORPUS").is_some() {
            std::fs::write(&path, serde_json::to_string_pretty(&corpus).unwrap() + "\n").unwrap();
        } else {
            assert_eq!(
                checked_in, corpus,
                "run-contract corpus is stale; run with UPDATE_CONTRACT_CORPUS=1"
            );
        }
    }
}
