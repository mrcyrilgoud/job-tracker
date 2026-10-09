use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    pub id: String,
    pub name: String,
    pub careers_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub company_id: String,
    pub title: String,
    pub url: String,
    pub canonical_url: String,
    pub source_external_id: Option<String>,
    pub status: String,
    pub applied_at: Option<String>,
    pub posting_state: String,
    pub last_checked_at: Option<String>,
    pub last_check_result: Option<String>,
    pub source: String,
    pub notes: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub is_new_from_watch: bool,
    pub watch_disposition: Option<String>,
    pub missing_from_sync_count: i64,
    pub is_favorite: bool,
    pub created_at: String,
    pub updated_at: String,
    /// Overall appeal from 1 (least appealing) to 5 (most appealing).
    /// `None` means the job has not been scored.
    pub appeal: Option<i64>,
    /// Optional annual USD salary bounds. A missing bound is open-ended.
    pub salary_min: Option<i64>,
    pub salary_max: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub id: String,
    pub job_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub note: Option<String>,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: String,
    pub original_filename: String,
    pub stored_filename: String,
    pub mime_type: String,
    pub checksum: String,
    pub size_bytes: i64,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDocument {
    pub id: String,
    pub job_id: String,
    pub document_id: String,
    pub kind: String,
    pub used_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanyWatch {
    pub id: String,
    pub company_id: String,
    pub provider: String,
    pub board_slug: String,
    pub last_synced_at: Option<String>,
    pub consecutive_sync_failures: i64,
    pub last_sync_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CareersPageReview {
    pub id: String,
    pub company_id: String,
    pub previous_hash: Option<String>,
    pub current_hash: String,
    pub summary: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobListItem {
    pub job: Job,
    pub company_name: String,
}

/// Compact projection used by the paged Jobs screen. Keep long detail fields
/// such as description, notes, and check results on the detail query only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    pub applied_at: Option<String>,
    pub posting_state: String,
    pub last_checked_at: Option<String>,
    pub source: String,
    pub is_new_from_watch: bool,
    pub is_favorite: bool,
    pub updated_at: String,
    pub appeal: Option<i64>,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobListSummary {
    pub job: JobSummary,
    pub company_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobPageCursor {
    pub updated_at: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobListPage {
    pub jobs: Vec<JobListSummary>,
    pub next_cursor: Option<JobPageCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentListItem {
    #[serde(flatten)]
    pub document: Document,
    pub kinds: Vec<String>,
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedDocument {
    pub attachment: JobDocument,
    pub document: Document,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDetail {
    pub job: Job,
    pub company: Company,
    pub events: Vec<JobEvent>,
    pub attached: Vec<AttachedDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeeklyDay {
    pub key: String,
    pub label: String,
    pub count: i64,
    pub is_today: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeeklyActivity {
    pub total: i64,
    pub days: Vec<WeeklyDay>,
}

pub const JOB_STATUSES: &[&str] = &[
    "wishlist",
    "applied",
    "interviewing",
    "offer",
    "rejected",
    "withdrawn",
    "closed",
    "archived",
];

pub fn is_job_status(value: &str) -> bool {
    JOB_STATUSES.contains(&value)
}

pub const APPEAL_RANGE_MESSAGE: &str = "Appeal must be an integer from 1 to 5 (5 = most appealing)";

pub fn checked_appeal(score: i64) -> Result<i64, &'static str> {
    if (1..=5).contains(&score) {
        Ok(score)
    } else {
        Err(APPEAL_RANGE_MESSAGE)
    }
}
