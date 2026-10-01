//! Run domain types shared by the coordinator, store, progress contract, and commands.
//!
//! Serde casing follows the Progress_Contract v1 (design.md): `RunType` is
//! camelCase on the wire (`jobsCycle`, `postingCheck`, `careerCheck`); status/state/stage/trigger
//! enums are snake_case; structs use camelCase field names. `as_str`/`parse`
//! give the canonical persisted SQLite text for each enum.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Opaque, unique Run_Identifier (uuid v4 text).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    /// Generate a fresh uuid v4 run identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Wrap an existing identifier (for example one loaded from SQLite).
    pub fn from_existing(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for RunId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Run_Type. Wire form is camelCase; persisted form is snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunType {
    JobsCycle,
    PostingCheck,
    CareerCheck,
}

impl RunType {
    pub const ALL: [RunType; 3] = [
        RunType::JobsCycle,
        RunType::PostingCheck,
        RunType::CareerCheck,
    ];

    pub fn has_stages(self) -> bool {
        matches!(self, Self::JobsCycle | Self::CareerCheck)
    }

    pub fn stage_order(self) -> &'static [StageName] {
        match self {
            Self::JobsCycle => &StageName::ORDER,
            Self::CareerCheck => &StageName::CAREER_CHECK_ORDER,
            Self::PostingCheck => &[],
        }
    }

    /// Persisted `runs.run_type` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JobsCycle => "jobs_cycle",
            Self::PostingCheck => "posting_check",
            Self::CareerCheck => "career_check",
        }
    }

    /// Parse a persisted `runs.run_type` value.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// Run_Status. Wire and persisted forms are both snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Active,
    Canceling,
    Canceled,
    Completed,
    CompletedWithErrors,
    Error,
}

impl RunStatus {
    pub const ALL: [RunStatus; 7] = [
        RunStatus::Queued,
        RunStatus::Active,
        RunStatus::Canceling,
        RunStatus::Canceled,
        RunStatus::Completed,
        RunStatus::CompletedWithErrors,
        RunStatus::Error,
    ];

    /// Canceled, Completed, Completed_With_Errors, and Error are terminal (Req 1.4).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Canceled | Self::Completed | Self::CompletedWithErrors | Self::Error
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Active => "active",
            Self::Canceling => "canceling",
            Self::Canceled => "canceled",
            Self::Completed => "completed",
            Self::CompletedWithErrors => "completed_with_errors",
            Self::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// Posting_Status of one Posting_Check within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostingStatus {
    Queued,
    Active,
    Completed,
    Error,
    Canceled,
}

impl PostingStatus {
    pub const ALL: [PostingStatus; 5] = [
        PostingStatus::Queued,
        PostingStatus::Active,
        PostingStatus::Completed,
        PostingStatus::Error,
        PostingStatus::Canceled,
    ];

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Error | Self::Canceled)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Canceled => "canceled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// Posting_State. The UI label "Closed" corresponds to `Inactive`, persisted as `"inactive"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostingState {
    Active,
    Inactive,
    Unknown,
}

impl PostingState {
    pub const ALL: [PostingState; 3] = [
        PostingState::Active,
        PostingState::Inactive,
        PostingState::Unknown,
    ];

    /// Persisted `jobs.posting_state` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// Jobs_Cycle stage names, in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageName {
    Postings,
    Watches,
    Careers,
    Csv,
}

impl StageName {
    /// Fixed execution order: postings → watches → careers → csv.
    pub const ORDER: [StageName; 4] = [
        StageName::Postings,
        StageName::Watches,
        StageName::Careers,
        StageName::Csv,
    ];

    pub const CAREER_CHECK_ORDER: [StageName; 2] = [StageName::Watches, StageName::Careers];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Postings => "postings",
            Self::Watches => "watches",
            Self::Careers => "careers",
            Self::Csv => "csv",
        }
    }
}

/// Outcome of one Jobs_Cycle stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageOutcome {
    NotStarted,
    InProgress,
    Succeeded,
    Failed,
    Skipped,
}

/// Per-status posting counters. Their sum equals the run's posting total.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostingCounts {
    pub queued: u64,
    pub active: u64,
    pub completed: u64,
    pub error: u64,
    pub canceled: u64,
}

impl PostingCounts {
    pub fn total(&self) -> u64 {
        self.queued + self.active + self.completed + self.error + self.canceled
    }
}

/// Job_Identity frozen for the lifetime of a run (Req 3.10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobIdentity {
    pub job_id: String,
    pub title: String,
    pub company_name: String,
    pub posting_url: String,
}

/// What initiated a run. Persisted in `runs.trigger`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Desktop,
    LegacyCommand,
    Cli,
    Launchd,
    Retry,
}

impl Trigger {
    pub const ALL: [Trigger; 5] = [
        Trigger::Desktop,
        Trigger::LegacyCommand,
        Trigger::Cli,
        Trigger::Launchd,
        Trigger::Retry,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::LegacyCommand => "legacy_command",
            Self::Cli => "cli",
            Self::Launchd => "launchd",
            Self::Retry => "retry",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    fn wire<T: Serialize>(v: T) -> String {
        serde_json::to_value(v)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn run_id_is_uuid_v4_and_unique() {
        let ids: HashSet<String> = (0..256).map(|_| RunId::new().into_string()).collect();
        assert_eq!(ids.len(), 256);
        for id in &ids {
            let parsed = Uuid::parse_str(id).expect("valid uuid");
            assert_eq!(parsed.get_version_num(), 4);
            assert!(!id.is_empty() && id.len() <= 64);
        }
    }

    #[test]
    fn run_id_serializes_as_plain_string() {
        let id = RunId::from_existing("abc-123");
        assert_eq!(serde_json::to_value(&id).unwrap(), json!("abc-123"));
        let back: RunId = serde_json::from_value(json!("abc-123")).unwrap();
        assert_eq!(back, id);
        assert_eq!(id.to_string(), "abc-123");
    }

    #[test]
    fn run_type_wire_is_camel_case_and_db_is_snake_case() {
        assert_eq!(wire(RunType::JobsCycle), "jobsCycle");
        assert_eq!(wire(RunType::PostingCheck), "postingCheck");
        assert_eq!(wire(RunType::CareerCheck), "careerCheck");
        assert_eq!(RunType::JobsCycle.as_str(), "jobs_cycle");
        assert_eq!(RunType::PostingCheck.as_str(), "posting_check");
        assert_eq!(RunType::CareerCheck.as_str(), "career_check");
        for t in RunType::ALL {
            assert_eq!(RunType::parse(t.as_str()), Some(t));
        }
        assert_eq!(RunType::parse("jobsCycle"), None);
    }

    #[test]
    fn run_status_casing_and_terminality() {
        let expected = [
            (RunStatus::Queued, "queued", false),
            (RunStatus::Active, "active", false),
            (RunStatus::Canceling, "canceling", false),
            (RunStatus::Canceled, "canceled", true),
            (RunStatus::Completed, "completed", true),
            (
                RunStatus::CompletedWithErrors,
                "completed_with_errors",
                true,
            ),
            (RunStatus::Error, "error", true),
        ];
        for (status, text, terminal) in expected {
            assert_eq!(wire(status), text);
            assert_eq!(status.as_str(), text);
            assert_eq!(RunStatus::parse(text), Some(status));
            assert_eq!(status.is_terminal(), terminal, "{text}");
            let back: RunStatus = serde_json::from_value(json!(text)).unwrap();
            assert_eq!(back, status);
        }
    }

    #[test]
    fn posting_status_casing_and_terminality() {
        for s in PostingStatus::ALL {
            assert_eq!(wire(s), s.as_str());
            assert_eq!(PostingStatus::parse(s.as_str()), Some(s));
        }
        assert!(!PostingStatus::Queued.is_terminal());
        assert!(!PostingStatus::Active.is_terminal());
        assert!(PostingStatus::Completed.is_terminal());
        assert!(PostingStatus::Error.is_terminal());
        assert!(PostingStatus::Canceled.is_terminal());
    }

    #[test]
    fn posting_state_inactive_is_persisted_form_of_closed() {
        assert_eq!(wire(PostingState::Inactive), "inactive");
        assert_eq!(PostingState::Inactive.as_str(), "inactive");
        assert_eq!(
            PostingState::parse("inactive"),
            Some(PostingState::Inactive)
        );
        assert_eq!(PostingState::parse("closed"), None);
        for s in PostingState::ALL {
            assert_eq!(wire(s), s.as_str());
        }
    }

    #[test]
    fn stage_enums_are_snake_case() {
        let names: Vec<String> = StageName::ORDER.into_iter().map(wire).collect();
        assert_eq!(names, ["postings", "watches", "careers", "csv"]);
        for n in StageName::ORDER {
            assert_eq!(wire(n), n.as_str());
        }
        assert_eq!(wire(StageOutcome::NotStarted), "not_started");
        assert_eq!(wire(StageOutcome::InProgress), "in_progress");
        assert_eq!(wire(StageOutcome::Succeeded), "succeeded");
        assert_eq!(wire(StageOutcome::Failed), "failed");
        assert_eq!(wire(StageOutcome::Skipped), "skipped");
    }

    #[test]
    fn trigger_values_match_persisted_form() {
        let texts: Vec<String> = Trigger::ALL.into_iter().map(wire).collect();
        assert_eq!(
            texts,
            ["desktop", "legacy_command", "cli", "launchd", "retry"]
        );
        for t in Trigger::ALL {
            assert_eq!(Trigger::parse(t.as_str()), Some(t));
        }
    }

    #[test]
    fn job_identity_uses_camel_case_fields() {
        let id = JobIdentity {
            job_id: "j1".into(),
            title: "Engineer".into(),
            company_name: "Acme".into(),
            posting_url: "https://example.com/jobs/1".into(),
        };
        let v = serde_json::to_value(&id).unwrap();
        assert_eq!(
            v,
            json!({
                "jobId": "j1",
                "title": "Engineer",
                "companyName": "Acme",
                "postingUrl": "https://example.com/jobs/1"
            })
        );
        let back: JobIdentity = serde_json::from_value(v).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn posting_counts_serialize_and_total() {
        let c = PostingCounts {
            queued: 1,
            active: 2,
            completed: 3,
            error: 4,
            canceled: 5,
        };
        assert_eq!(c.total(), 15);
        assert_eq!(
            serde_json::to_value(c).unwrap(),
            json!({"queued": 1, "active": 2, "completed": 3, "error": 4, "canceled": 5})
        );
        assert_eq!(PostingCounts::default().total(), 0);
    }
}
