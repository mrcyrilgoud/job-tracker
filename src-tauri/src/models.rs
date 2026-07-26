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
    pub location: Option<String>,
    pub is_new_from_watch: bool,
    pub missing_from_sync_count: i64,
    pub created_at: String,
    pub updated_at: String,
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
pub struct EmailMatch {
    pub id: String,
    pub job_id: Option<String>,
    pub gmail_message_id: String,
    pub thread_id: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub from_address: Option<String>,
    pub received_at: Option<String>,
    pub confidence: String,
    pub triage_status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobListItem {
    pub job: Job,
    pub company_name: String,
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
];

pub fn is_job_status(value: &str) -> bool {
    JOB_STATUSES.contains(&value)
}
