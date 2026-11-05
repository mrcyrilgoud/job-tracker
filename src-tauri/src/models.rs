use serde::{Deserialize, Serialize};

/// The two company-owned scores and their optional supporting metadata.
///
/// Citation lists are represented as JSON arrays (or `null`) at every
/// serialization boundary. SQLite stores the list as JSON text, but that
/// encoding detail belongs to the persistence layer rather than this model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompanyScores {
    pub company_stability: Option<i64>,
    pub company_stability_rationale: Option<String>,
    pub company_stability_citations: Option<Vec<String>>,
    pub company_stability_scored_at: Option<String>,
    pub company_stability_scored_by: Option<String>,
    pub employment_stability: Option<i64>,
    pub employment_stability_rationale: Option<String>,
    pub employment_stability_citations: Option<Vec<String>>,
    pub employment_stability_scored_at: Option<String>,
    pub employment_stability_scored_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    pub id: String,
    pub name: String,
    pub careers_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(flatten)]
    pub scores: CompanyScores,
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
    /// How easy this posting is to apply to, from 1 (hardest) to 5 (easiest).
    /// `None` means the job has not been scored.
    pub application_ease: Option<i64>,
    /// How stable the company looks, from 1 (least stable) to 5 (most stable).
    /// `None` means the job's company has not been scored.
    pub company_stability: Option<i64>,
    /// How secure employment looks, from 1 (least secure) to 5 (most secure).
    /// `None` means the job's company has not been scored.
    pub employment_stability: Option<i64>,
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
    /// How easy this posting is to apply to, from 1 (hardest) to 5 (easiest).
    /// `None` means the job has not been scored.
    pub application_ease: Option<i64>,
    /// How stable the company looks, from 1 (least stable) to 5 (most stable).
    /// `None` means the job has not been scored.
    pub company_stability: Option<i64>,
    /// How secure employment at the company looks, from 1 (least secure) to 5 (most secure).
    /// `None` means the company has not been scored.
    pub employment_stability: Option<i64>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobSort {
    NewestPosting,
    Oldest,
    AppealHighToLow,
    AppealLowToHigh,
    CompanyAz,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobListSortField {
    AppliedAt,
    PostingDate,
    Appeal,
    CompanyName,
    CompanyStability,
    EmploymentStability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobListSortDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobListSort {
    pub field: JobListSortField,
    pub direction: JobListSortDirection,
}

impl Default for JobListSort {
    fn default() -> Self {
        Self {
            field: JobListSortField::AppliedAt,
            direction: JobListSortDirection::Descending,
        }
    }
}

impl Default for JobSort {
    fn default() -> Self {
        Self::NewestPosting
    }
}

/// Sort-aware cursor used only by the main Jobs page. Watch-role paging keeps
/// using `JobPageCursor` and its updated-at ordering.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobListPageCursor {
    pub sort: JobSort,
    pub sort_spec: JobListSort,
    pub sort_value: Option<String>,
    pub applied_tab: bool,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobListPage {
    pub jobs: Vec<JobListSummary>,
    pub next_cursor: Option<JobListPageCursor>,
    /// Number of jobs matching the filters, ignoring the page cursor.
    pub total_count: i64,
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

pub const APPLICATION_EASE_RANGE_MESSAGE: &str =
    "Application ease must be an integer from 1 to 5 (5 = easiest to apply)";

pub fn checked_application_ease(score: i64) -> Result<i64, &'static str> {
    if (1..=5).contains(&score) {
        Ok(score)
    } else {
        Err(APPLICATION_EASE_RANGE_MESSAGE)
    }
}

pub const COMPANY_STABILITY_RANGE_MESSAGE: &str =
    "Company stability must be an integer from 1 to 5 (5 = most stable)";

pub fn checked_company_stability(score: i64) -> Result<i64, &'static str> {
    if (1..=5).contains(&score) {
        Ok(score)
    } else {
        Err(COMPANY_STABILITY_RANGE_MESSAGE)
    }
}

pub const EMPLOYMENT_STABILITY_RANGE_MESSAGE: &str =
    "Employment stability must be an integer from 1 to 5 (5 = most secure)";

pub fn checked_employment_stability(score: i64) -> Result<i64, &'static str> {
    if (1..=5).contains(&score) {
        Ok(score)
    } else {
        Err(EMPLOYMENT_STABILITY_RANGE_MESSAGE)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        checked_company_stability, checked_employment_stability, CompanyScores, JobListSort,
        JobListSortDirection, JobListSortField, JobSort, COMPANY_STABILITY_RANGE_MESSAGE,
        EMPLOYMENT_STABILITY_RANGE_MESSAGE,
    };

    #[test]
    fn job_sort_uses_the_frontend_camel_case_values() {
        for (sort, wire_value) in [
            (JobSort::NewestPosting, "newestPosting"),
            (JobSort::Oldest, "oldest"),
            (JobSort::AppealHighToLow, "appealHighToLow"),
            (JobSort::AppealLowToHigh, "appealLowToHigh"),
            (JobSort::CompanyAz, "companyAz"),
        ] {
            let encoded = serde_json::to_string(&sort).unwrap();
            assert_eq!(encoded, format!("\"{wire_value}\""));
            assert_eq!(serde_json::from_str::<JobSort>(&encoded).unwrap(), sort);
        }
    }

    #[test]
    fn applied_job_sort_uses_the_frontend_camel_case_values() {
        let sort = JobListSort {
            field: JobListSortField::CompanyName,
            direction: JobListSortDirection::Ascending,
        };
        let encoded = serde_json::to_value(sort).unwrap();
        assert_eq!(encoded["field"], "companyName");
        assert_eq!(encoded["direction"], "ascending");
        assert_eq!(
            serde_json::from_value::<JobListSort>(encoded).unwrap(),
            sort
        );
    }

    #[test]
    fn company_stability_sort_field_uses_camel_case_wire_value() {
        let encoded = serde_json::to_value(JobListSortField::CompanyStability).unwrap();
        assert_eq!(encoded, "companyStability");
        assert_eq!(
            serde_json::from_value::<JobListSortField>(encoded).unwrap(),
            JobListSortField::CompanyStability
        );
    }

    #[test]
    fn company_scores_model_defaults_all_ten_fields_to_null() {
        let scores = CompanyScores::default();
        let value = serde_json::to_value(scores).unwrap();
        for key in [
            "companyStability",
            "companyStabilityRationale",
            "companyStabilityCitations",
            "companyStabilityScoredAt",
            "companyStabilityScoredBy",
            "employmentStability",
            "employmentStabilityRationale",
            "employmentStabilityCitations",
            "employmentStabilityScoredAt",
            "employmentStabilityScoredBy",
        ] {
            assert_eq!(value[key], serde_json::Value::Null, "{key}");
        }
    }

    #[test]
    fn checked_employment_stability_uses_the_exact_five_point_range() {
        for score in 1..=5 {
            assert_eq!(checked_employment_stability(score), Ok(score));
        }
        for score in [i64::MIN, -1, 0, 6, i64::MAX] {
            assert_eq!(
                checked_employment_stability(score),
                Err(EMPLOYMENT_STABILITY_RANGE_MESSAGE)
            );
        }
    }

    #[test]
    fn checked_company_stability_uses_the_exact_five_point_range() {
        for score in 1..=5 {
            assert_eq!(checked_company_stability(score), Ok(score));
        }
        for score in [i64::MIN, -1, 0, 6, i64::MAX] {
            assert_eq!(
                checked_company_stability(score),
                Err(COMPANY_STABILITY_RANGE_MESSAGE)
            );
        }
    }
}
