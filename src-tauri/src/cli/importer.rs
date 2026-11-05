//! Score-file validation and planning primitives.
//!
//! This module deliberately stops at the database boundary. It parses and
//! validates the complete JSON file, maps the six supported fields into the
//! shared score-service request, and provides the stable summary envelope used
//! by the CLI. Database matching and per-entry application are layered on top
//! by the score import command.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::company_scores::{
    normalize_citations, normalize_rationale, OptionalField, ScoreSource, ScoreWriteInput,
    MAX_CITATIONS,
};
use crate::error::{AppError, AppResult};
use crate::util::now_iso;
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const MAX_SCORE_FILE_ENTRIES: usize = 5_000;

const SUPPORTED_KEYS: [&str; 6] = [
    "score",
    "rationale",
    "citations",
    "employment_stability",
    "employment_rationale",
    "employment_citations",
];

/// One normalized, validated score entry. `position` is intentionally the
/// source array index rather than a one-based display row number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedScoreEntry {
    pub position: usize,
    pub name: Option<String>,
    pub tracker_names: Vec<String>,
    pub write: ScoreWriteInput,
}

impl ValidatedScoreEntry {
    pub fn matching_names(&self) -> impl Iterator<Item = &str> {
        self.tracker_names
            .iter()
            .map(String::as_str)
            .chain(self.name.as_deref())
    }
}

/// The file-level result passed to the database-aware planner. Invalid entries
/// are retained in source order and are never converted into write requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedScoreFile {
    pub as_of: String,
    pub entries: Vec<ValidatedScoreEntry>,
    pub invalid: Vec<ImportItem>,
    pub warnings: Vec<ImportItem>,
}

/// A summary item shared by all eight output groups. A file-level warning has
/// no entry position; entry-level items always carry the zero-based position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItem {
    pub position: Option<usize>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ImportItem {
    pub fn entry(position: usize, name: impl Into<String>) -> Self {
        Self {
            position: Some(position),
            name: name.into(),
            company_id: None,
            company_name: None,
            reason: None,
        }
    }

    pub fn file_warning(reason: impl Into<String>) -> Self {
        Self {
            position: None,
            name: "<file>".to_string(),
            company_id: None,
            company_name: None,
            reason: Some(reason.into()),
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// The exact machine-readable importer envelope. Keep these eight top-level
/// keys stable: consumers use an empty array to distinguish no results from a
/// missing result group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub dry_run: bool,
    pub applied: Vec<ImportItem>,
    pub unchanged: Vec<ImportItem>,
    pub unmatched: Vec<ImportItem>,
    pub conflicts: Vec<ImportItem>,
    pub invalid: Vec<ImportItem>,
    pub warnings: Vec<ImportItem>,
    pub failed: Vec<ImportItem>,
}

impl ImportSummary {
    pub fn new(dry_run: bool) -> Self {
        Self {
            dry_run,
            ..Self::default()
        }
    }

    /// A failed write or mirror refresh is the only nonzero classification.
    /// Unmatched, conflicting, invalid, and warning entries are successful
    /// imports with actionable output, as required by the CLI contract.
    pub fn exit_success(&self) -> bool {
        self.failed.is_empty()
    }

    pub fn from_file_validation(file: &ValidatedScoreFile, dry_run: bool) -> Self {
        let mut summary = Self::new(dry_run);
        summary.invalid = file.invalid.clone();
        summary.warnings = file.warnings.clone();
        summary
    }
}

/// The caller prints this classification as its process exit status after it
/// has rendered the complete summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportExit {
    Success,
    Failed,
}

impl From<&ImportSummary> for ImportExit {
    fn from(summary: &ImportSummary) -> Self {
        if summary.exit_success() {
            Self::Success
        } else {
            Self::Failed
        }
    }
}

/// Render the summary without writing to stdout. JSON and quiet mode both
/// return only one JSON object, making quiet mode safe for shell pipelines.
pub fn format_summary(summary: &ImportSummary, json: bool, quiet: bool) -> AppResult<String> {
    if json || quiet {
        return serde_json::to_string(summary).map_err(|error| {
            AppError::from(format!("Failed to serialize score import summary: {error}"))
        });
    }

    let mut lines = Vec::new();
    lines.push(if summary.dry_run {
        "Score import plan (dry run):".to_string()
    } else {
        "Score import summary:".to_string()
    });
    append_human_group(&mut lines, "Applied", &summary.applied);
    append_human_group(&mut lines, "Unchanged", &summary.unchanged);
    append_human_group(&mut lines, "Unmatched", &summary.unmatched);
    append_human_group(&mut lines, "Conflicts", &summary.conflicts);
    append_human_group(&mut lines, "Invalid", &summary.invalid);
    append_human_group(&mut lines, "Warnings", &summary.warnings);
    append_human_group(&mut lines, "Failed", &summary.failed);
    Ok(lines.join("\n"))
}

fn append_human_group(lines: &mut Vec<String>, label: &str, items: &[ImportItem]) {
    lines.push(format!("{label} ({}):", items.len()));
    if items.is_empty() {
        lines.push("  (none)".to_string());
        return;
    }
    for item in items {
        let position = item
            .position
            .map_or_else(|| "file".to_string(), |position| position.to_string());
        let company = item
            .company_name
            .as_deref()
            .or(item.company_id.as_deref())
            .unwrap_or("");
        let reason = item.reason.as_deref().unwrap_or("");
        let suffix = [company, reason]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(": ");
        if suffix.is_empty() {
            lines.push(format!("  [{position}] {}", item.name));
        } else {
            lines.push(format!("  [{position}] {}: {suffix}", item.name));
        }
    }
}

/// Read and validate a score file. All file-level failures happen here,
/// before a caller opens SQLite or touches the CSV mirror.
pub fn read_score_file(path: &Path) -> AppResult<ValidatedScoreFile> {
    let text = fs::read_to_string(path).map_err(|error| {
        AppError::from(format!(
            "Unable to read score file {}: {error}",
            path.display()
        ))
    })?;
    parse_score_file(&text)
}

/// Parse and validate a score file from its complete JSON text.
pub fn parse_score_file(text: &str) -> AppResult<ValidatedScoreFile> {
    let root: Value = serde_json::from_str(text)
        .map_err(|error| AppError::from(format!("Score file is not valid JSON: {error}")))?;
    let object = root
        .as_object()
        .ok_or_else(|| AppError::from("Score file top level must be a JSON object"))?;
    let companies = object
        .get("companies")
        .ok_or_else(|| AppError::from("Score file is missing the companies key"))?
        .as_array()
        .ok_or_else(|| AppError::from("Score file companies must be a JSON array"))?;
    if companies.len() > MAX_SCORE_FILE_ENTRIES {
        return Err(AppError::from(format!(
            "Score file companies exceeds the maximum of {MAX_SCORE_FILE_ENTRIES} entries"
        )));
    }

    let (as_of, as_of_warning) = normalize_as_of(object.get("as_of"));
    let mut invalid = Vec::new();
    let mut entries = Vec::with_capacity(companies.len());
    for (position, value) in companies.iter().enumerate() {
        match validate_entry(position, value, &as_of) {
            Ok(entry) => entries.push(entry),
            Err(item) => invalid.push(item),
        }
    }

    let mut warnings = Vec::new();
    if let Some(reason) = as_of_warning {
        warnings.push(ImportItem::file_warning(reason));
    }
    Ok(ValidatedScoreFile {
        as_of,
        entries,
        invalid,
        warnings,
    })
}

/// Normalize a valid date or RFC3339 timestamp to millisecond UTC RFC3339.
/// Invalid, absent, null, and non-string values intentionally fall back to a
/// single import-time value and one file-level warning.
pub fn normalize_as_of(value: Option<&Value>) -> (String, Option<String>) {
    let Some(Value::String(raw)) = value else {
        return (
            now_iso(),
            Some("Score file `as_of` was not used; import time was used instead".to_string()),
        );
    };

    if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
        return (
            parsed
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Millis, true),
            None,
        );
    }
    if let Ok(parsed) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let timestamp = parsed
            .and_hms_opt(0, 0, 0)
            .expect("midnight is valid")
            .and_utc()
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        return (timestamp, None);
    }

    (
        now_iso(),
        Some("Score file `as_of` was not used; import time was used instead".to_string()),
    )
}

fn validate_entry(
    position: usize,
    value: &Value,
    as_of: &str,
) -> Result<ValidatedScoreEntry, ImportItem> {
    let Some(object) = value.as_object() else {
        return Err(ImportItem::entry(position, "").with_reason("entry must be a JSON object"));
    };

    let display_name = entry_display_name(object);
    let name = match object.get("name") {
        None => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(
                ImportItem::entry(position, display_name).with_reason("name must be a string")
            )
        }
    };

    let tracker_names = match object.get("tracker_names") {
        None => Vec::new(),
        Some(Value::Array(values)) => {
            let mut names = Vec::with_capacity(values.len());
            for value in values {
                let Some(name) = value.as_str() else {
                    return Err(ImportItem::entry(position, display_name)
                        .with_reason("tracker_names must be an array of strings"));
                };
                names.push(name.to_string());
            }
            names
        }
        Some(_) => {
            return Err(ImportItem::entry(position, display_name)
                .with_reason("tracker_names must be an array of strings"));
        }
    };
    let name_is_empty = match name.as_deref() {
        None => true,
        Some(name) => name.trim().is_empty(),
    };
    if name_is_empty && tracker_names.is_empty() {
        return Err(ImportItem::entry(position, display_name)
            .with_reason("entry needs a non-empty name or tracker_names"));
    }

    let score = validate_score_field(
        object,
        "score",
        "Company stability",
        position,
        &display_name,
    )?;
    let employment = validate_score_field(
        object,
        "employment_stability",
        "Employment stability",
        position,
        &display_name,
    )?;
    let company_rationale = validate_rationale_field(object, "rationale", position, &display_name)?;
    let employment_rationale =
        validate_rationale_field(object, "employment_rationale", position, &display_name)?;
    let company_citations = validate_citations_field(object, "citations", position, &display_name)?;
    let employment_citations =
        validate_citations_field(object, "employment_citations", position, &display_name)?;

    Ok(ValidatedScoreEntry {
        position,
        name,
        tracker_names,
        write: ScoreWriteInput {
            company_stability: score,
            employment_stability: employment,
            company_stability_rationale: company_rationale,
            company_stability_citations: company_citations,
            employment_stability_rationale: employment_rationale,
            employment_stability_citations: employment_citations,
            source: ScoreSource::Import,
            scored_at_override: Some(as_of.to_string()),
        },
    })
}

fn entry_display_name(object: &Map<String, Value>) -> String {
    object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn validate_score_field(
    object: &Map<String, Value>,
    key: &str,
    label: &str,
    position: usize,
    name: &str,
) -> Result<OptionalField<Option<i64>>, ImportItem> {
    let Some(value) = object.get(key) else {
        return Ok(OptionalField::Omitted);
    };
    if value.is_null() {
        return Ok(OptionalField::Present(None));
    }
    let Some(number) = value.as_number() else {
        return Err(ImportItem::entry(position, name)
            .with_reason(format!("{key}: {label} must be an integer from 1 to 5")));
    };
    let Some(score) = number.as_i64().filter(|score| (1..=5).contains(score)) else {
        return Err(ImportItem::entry(position, name)
            .with_reason(format!("{key}: {label} must be an integer from 1 to 5")));
    };
    Ok(OptionalField::Present(Some(score)))
}

fn validate_rationale_field(
    object: &Map<String, Value>,
    key: &str,
    position: usize,
    name: &str,
) -> Result<OptionalField<Option<String>>, ImportItem> {
    let Some(value) = object.get(key) else {
        return Ok(OptionalField::Omitted);
    };
    if value.is_null() {
        return Ok(OptionalField::Present(None));
    }
    let Some(text) = value.as_str() else {
        return Err(ImportItem::entry(position, name)
            .with_reason(format!("{key}: rationale must be a string or null")));
    };
    let normalized = normalize_rationale(text).map_err(|reason| {
        ImportItem::entry(position, name).with_reason(format!("{key}: {reason}"))
    })?;
    Ok(OptionalField::Present(normalized))
}

fn validate_citations_field(
    object: &Map<String, Value>,
    key: &str,
    position: usize,
    name: &str,
) -> Result<OptionalField<Option<Vec<String>>>, ImportItem> {
    let Some(value) = object.get(key) else {
        return Ok(OptionalField::Omitted);
    };
    if value.is_null() {
        return Ok(OptionalField::Present(None));
    }
    let Some(values) = value.as_array() else {
        return Err(ImportItem::entry(position, name).with_reason(format!(
            "{key}: citations must be an array of strings or null"
        )));
    };
    if values.len() > MAX_CITATIONS {
        return Err(ImportItem::entry(position, name).with_reason(format!(
            "{key}: citation list must contain at most {MAX_CITATIONS} entries"
        )));
    }
    let mut citations = Vec::with_capacity(values.len());
    for value in values {
        let Some(citation) = value.as_str() else {
            return Err(ImportItem::entry(position, name)
                .with_reason(format!("{key}: every citation must be a string")));
        };
        let trimmed = citation.trim();
        citations.push(trimmed.to_string());
    }
    let normalized = normalize_citations(&citations).map_err(|reason| {
        ImportItem::entry(position, name).with_reason(format!("{key}: {reason}"))
    })?;
    Ok(OptionalField::Present(normalized))
}

/// Keep this helper private to the module but expose the supported mapping for
/// tests and future callers that need to filter unknown research fields.
pub fn supported_score_keys() -> &'static [&'static str] {
    &SUPPORTED_KEYS
}

/// A deterministic, database-independent plan containing all entries that
/// survived file validation. The database-aware layer fills applied,
/// unchanged, unmatched, and conflict groups without reparsing the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreImportPlan {
    pub file: ValidatedScoreFile,
    pub summary: ImportSummary,
}

impl ScoreImportPlan {
    pub fn new(file: ValidatedScoreFile, dry_run: bool) -> Self {
        let summary = ImportSummary::from_file_validation(&file, dry_run);
        Self { file, summary }
    }

    pub fn entries_by_position(&self) -> BTreeMap<usize, &ValidatedScoreEntry> {
        self.file
            .entries
            .iter()
            .map(|entry| (entry.position, entry))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> ValidatedScoreFile {
        parse_score_file(text).unwrap()
    }

    #[test]
    fn validates_top_level_shape_before_entries() {
        for (input, message) in [
            ("[]", "top level"),
            (r#"{}"#, "missing the companies"),
            (r#"{"companies":{}}"#, "companies must be a JSON array"),
            (r#"{"companies":null}"#, "companies must be a JSON array"),
        ] {
            let error = parse_score_file(input).unwrap_err().to_string();
            assert!(error.contains(message), "{input}: {error}");
        }
    }

    #[test]
    fn rejects_more_than_five_thousand_entries_without_building_a_plan() {
        let entries = (0..=MAX_SCORE_FILE_ENTRIES)
            .map(|_| "{\"name\":\"Acme\"}")
            .collect::<Vec<_>>()
            .join(",");
        let error = parse_score_file(&format!("{{\"companies\":[{entries}]}}"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("5000"));
    }

    #[test]
    fn preserves_zero_based_positions_when_invalid_entries_are_interleaved() {
        let file = parse(
            r#"{
                "companies": [
                    {"name":"valid", "score": 3},
                    {"name":"bad", "score": 6},
                    {"name":"valid-two", "employment_stability": 4},
                    12,
                    {"name":"valid-three", "unknown": true}
                ]
            }"#,
        );
        assert_eq!(
            file.entries
                .iter()
                .map(|entry| entry.position)
                .collect::<Vec<_>>(),
            vec![0, 2, 4]
        );
        assert_eq!(file.invalid[0].position, Some(1));
        assert_eq!(file.invalid[1].position, Some(3));
    }

    #[test]
    fn maps_six_supported_keys_and_ignores_unmapped_fields() {
        let file = parse(
            r#"{
                "as_of": "2026-11-02",
                "companies": [{
                    "name":" Acme ",
                    "tracker_names":["Acme Inc"],
                    "score": 4,
                    "rationale": "  funded  ",
                    "citations": [" https://example.com/a "],
                    "employment_stability": 2,
                    "employment_rationale": "cuts",
                    "employment_citations": ["https://example.com/b"],
                    "signals": {"funding":"ignored"},
                    "rubric": "ignored",
                    "other": true
                }]
            }"#,
        );
        let entry = &file.entries[0];
        assert_eq!(entry.position, 0);
        assert_eq!(entry.name.as_deref(), Some(" Acme "));
        assert_eq!(entry.tracker_names, vec!["Acme Inc"]);
        assert_eq!(
            entry.write.company_stability,
            OptionalField::Present(Some(4))
        );
        assert_eq!(
            entry.write.company_stability_rationale,
            OptionalField::Present(Some("funded".to_string()))
        );
        assert_eq!(
            entry.write.company_stability_citations,
            OptionalField::Present(Some(vec!["https://example.com/a".to_string()]))
        );
        assert_eq!(
            entry.write.employment_stability,
            OptionalField::Present(Some(2))
        );
        assert_eq!(file.as_of, "2026-11-02T00:00:00.000Z");
        assert!(file.warnings.is_empty());
    }

    #[test]
    fn invalid_as_of_uses_one_import_time_warning() {
        let file = parse(r#"{"as_of":"not-a-date","companies":[{"name":"Acme"}]}"#);
        assert_eq!(file.warnings.len(), 1);
        assert_eq!(file.warnings[0].position, None);
        assert!(file.warnings[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("as_of"));
        assert!(DateTime::parse_from_rfc3339(&file.as_of).is_ok());
    }

    #[test]
    fn invalid_entry_reports_first_key_and_continues() {
        let file = parse(
            r#"{"companies":[
                {"name":"bad","score":3.0,"rationale":42},
                {"name":"good","score":3,"employment_stability":null}
            ]}"#,
        );
        assert_eq!(file.entries.len(), 1);
        assert_eq!(file.invalid.len(), 1);
        assert!(file.invalid[0].reason.as_deref().unwrap().contains("score"));
    }

    #[test]
    fn nulls_and_omitted_fields_keep_distinct_write_semantics() {
        let file = parse(
            r#"{"companies":[{"name":"Acme","score":null,"rationale":null,"citations":null}]}"#,
        );
        let entry = &file.entries[0];
        assert_eq!(entry.write.company_stability, OptionalField::Present(None));
        assert_eq!(
            entry.write.company_stability_rationale,
            OptionalField::Present(None)
        );
        assert_eq!(
            entry.write.company_stability_citations,
            OptionalField::Present(None)
        );
        assert_eq!(entry.write.employment_stability, OptionalField::Omitted);
    }

    #[test]
    fn summary_has_exact_eight_top_level_keys_and_quiet_is_payload_only() {
        let summary = ImportSummary::new(true);
        let value: Value =
            serde_json::from_str(&format_summary(&summary, true, false).unwrap()).unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "applied",
                "conflicts",
                "dryRun",
                "failed",
                "invalid",
                "unchanged",
                "unmatched",
                "warnings"
            ]
        );
        assert_eq!(
            format_summary(&summary, false, true).unwrap(),
            format_summary(&summary, true, false).unwrap()
        );
    }
}

// Database-aware matching and application are kept below the file-only
// planner above. The aliases preserve a small, useful API for CLI handlers
// without changing the stable validation types used by existing callers.
pub type ScoreFile = ValidatedScoreFile;

#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub file: ScoreFile,
    pub summary: ImportSummary,
    operations: Vec<DatabaseOperation>,
}

#[derive(Debug, Clone)]
struct DatabaseCompany {
    id: String,
    name: String,
}

#[derive(Debug, Clone)]
struct DatabaseOperation {
    entry: ValidatedScoreEntry,
    company: DatabaseCompany,
    item: ImportItem,
}

/// Match validated entries against existing companies and classify all
/// conflicts before any write. Tracker names are authoritative whenever any
/// tracker name matches; `name` is used only when tracker names are empty or
/// all tracker names are unmatched.
pub fn plan_import(
    conn: &rusqlite::Connection,
    file: &ScoreFile,
    dry_run: bool,
) -> AppResult<ImportPlan> {
    let companies = load_database_companies(conn)?;
    let mut summary = ImportSummary::from_file_validation(file, dry_run);
    let mut matched = Vec::<(ValidatedScoreEntry, Vec<DatabaseCompany>)>::new();

    for entry in &file.entries {
        let candidates = resolve_database_match(entry, &companies);
        if candidates.is_empty() {
            summary.unmatched.push(
                ImportItem::entry(entry.position, entry.name.clone().unwrap_or_default())
                    .with_reason("no company matched tracker_names or name"),
            );
        } else {
            matched.push((entry.clone(), candidates));
        }
    }

    let conflict_positions = merge_database_conflicts(&mut summary, &matched);
    let mut operations = Vec::new();
    for (entry, candidates) in matched {
        if conflict_positions.contains(&entry.position) {
            continue;
        }
        let company = candidates.into_iter().next().expect("non-empty candidates");
        let item = ImportItem {
            position: Some(entry.position),
            name: entry.name.clone().unwrap_or_default(),
            company_id: Some(company.id.clone()),
            company_name: Some(company.name.clone()),
            reason: None,
        };
        let (changed, warnings) = preview_import_write(conn, &company.id, &entry.write)?;
        if changed {
            summary.applied.push(item.clone());
        } else {
            summary.unchanged.push(item.clone());
        }
        for warning in warnings {
            summary.warnings.push(ImportItem {
                reason: Some(warning),
                ..item.clone()
            });
        }
        operations.push(DatabaseOperation {
            entry,
            company,
            item,
        });
    }

    Ok(ImportPlan {
        file: file.clone(),
        summary,
        operations,
    })
}

/// Apply the non-conflicting portion of a database plan. Each operation gets
/// its own transaction and later operations continue after a rollback.
pub fn apply_import(
    conn: &rusqlite::Connection,
    paths: &crate::db::paths::DataPaths,
    mut plan: ImportPlan,
) -> AppResult<ImportSummary> {
    plan.summary.dry_run = false;
    plan.summary.applied.clear();
    plan.summary.unchanged.clear();
    plan.summary.failed.clear();

    for operation in plan.operations {
        let tx = conn
            .unchecked_transaction()
            .map_err(crate::error::map_sqlite)?;
        let result = crate::company_scores::ScoreService::write(
            &tx,
            &operation.company.id,
            &operation.entry.write,
        );
        match result {
            Ok(write) => {
                tx.commit().map_err(crate::error::map_sqlite)?;
                if write.changed {
                    plan.summary.applied.push(operation.item.clone());
                } else {
                    plan.summary.unchanged.push(operation.item.clone());
                }
                for warning in write.warnings {
                    let warning_item = ImportItem {
                        reason: Some(warning),
                        ..operation.item.clone()
                    };
                    if !plan.summary.warnings.contains(&warning_item) {
                        plan.summary.warnings.push(warning_item);
                    }
                }
            }
            Err(error) => {
                plan.summary.failed.push(ImportItem {
                    reason: Some(error.to_string()),
                    ..operation.item.clone()
                });
            }
        }
    }

    if !plan.summary.applied.is_empty() {
        let csv_path = crate::jobs::csv_config::active_csv_path(conn, &paths.jobs_csv_path)?;
        if let Err(error) = crate::jobs::csv::export_jobs_csv(conn, &csv_path, None) {
            plan.summary.failed.push(ImportItem::file_warning(format!(
                "CSV mirror refresh failed: {error}"
            )));
        }
    }
    Ok(plan.summary)
}

fn load_database_companies(conn: &rusqlite::Connection) -> AppResult<Vec<DatabaseCompany>> {
    let mut statement = conn
        .prepare("SELECT id, name FROM companies ORDER BY name COLLATE NOCASE, id")
        .map_err(crate::error::map_sqlite)?;
    let rows = statement
        .query_map([], |row| {
            Ok(DatabaseCompany {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>();
    rows.map_err(crate::error::map_sqlite)
}

fn resolve_database_match(
    entry: &ValidatedScoreEntry,
    companies: &[DatabaseCompany],
) -> Vec<DatabaseCompany> {
    let tracker_matches = matching_database_names(&entry.tracker_names, companies);
    if !entry.tracker_names.is_empty() && !tracker_matches.is_empty() {
        return tracker_matches;
    }
    entry
        .name
        .as_deref()
        .map(|name| matching_database_names(&[name.to_string()], companies))
        .unwrap_or_default()
}

fn matching_database_names(
    names: &[String],
    companies: &[DatabaseCompany],
) -> Vec<DatabaseCompany> {
    let mut by_id = BTreeMap::new();
    for name in names {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        for company in companies {
            if company.name.trim().eq_ignore_ascii_case(name) {
                by_id.insert(company.id.clone(), company.clone());
            }
        }
    }
    by_id.into_values().collect()
}

fn merge_database_conflicts(
    summary: &mut ImportSummary,
    matched: &[(ValidatedScoreEntry, Vec<DatabaseCompany>)],
) -> std::collections::BTreeSet<usize> {
    let mut parent: Vec<usize> = (0..matched.len()).collect();
    let candidates: Vec<std::collections::BTreeSet<String>> = matched
        .iter()
        .map(|(_, companies)| companies.iter().map(|company| company.id.clone()).collect())
        .collect();
    fn find(parent: &mut [usize], index: usize) -> usize {
        if parent[index] != index {
            let root = find(parent, parent[index]);
            parent[index] = root;
        }
        parent[index]
    }
    fn union(parent: &mut [usize], left: usize, right: usize) {
        let left = find(parent, left);
        let right = find(parent, right);
        if left != right {
            parent[right] = left;
        }
    }
    for left in 0..matched.len() {
        for right in (left + 1)..matched.len() {
            if !candidates[left].is_disjoint(&candidates[right]) {
                union(&mut parent, left, right);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..matched.len() {
        groups
            .entry(find(&mut parent, index))
            .or_default()
            .push(index);
    }
    let mut conflicted = std::collections::BTreeSet::new();
    for indices in groups.values() {
        let ambiguous = indices.iter().any(|index| matched[*index].1.len() > 1);
        let duplicate = indices.len() > 1;
        if !ambiguous && !duplicate {
            continue;
        }
        let mut companies = BTreeMap::<String, String>::new();
        let mut names = Vec::new();
        for index in indices {
            let (entry, candidates) = &matched[*index];
            conflicted.insert(entry.position);
            names.push(format!(
                "[{}] {}",
                entry.position,
                entry.name.as_deref().unwrap_or("")
            ));
            for company in candidates {
                companies.insert(company.id.clone(), company.name.clone());
            }
        }
        let company_id = (companies.len() == 1).then(|| companies.keys().next().unwrap().clone());
        let company_name = company_id
            .as_ref()
            .and_then(|id| companies.get(id).cloned());
        let reason = if ambiguous && duplicate {
            format!(
                "ambiguous matching and duplicate entries merged; entries: {}",
                names.join(", ")
            )
        } else if ambiguous {
            format!(
                "matching names resolve to multiple companies ({}); entries: {}",
                companies.values().cloned().collect::<Vec<_>>().join(", "),
                names.join(", ")
            )
        } else {
            format!(
                "multiple entries match the same company; entries: {}",
                names.join(", ")
            )
        };
        summary.conflicts.push(ImportItem {
            position: indices.first().map(|index| matched[*index].0.position),
            name: names.join("; "),
            company_id,
            company_name,
            reason: Some(reason),
        });
    }
    conflicted
}

fn preview_import_write(
    conn: &rusqlite::Connection,
    company_id: &str,
    input: &ScoreWriteInput,
) -> AppResult<(bool, Vec<String>)> {
    let current = crate::company_scores::ScoreService::read(conn, company_id)?;
    let company_score = match &input.company_stability {
        OptionalField::Omitted => current.company_stability,
        OptionalField::Present(value) => *value,
    };
    let employment_score = match &input.employment_stability {
        OptionalField::Omitted => current.employment_stability,
        OptionalField::Present(value) => *value,
    };
    let mut warnings = Vec::new();
    if company_score.is_none()
        && metadata_present(
            &input.company_stability_rationale,
            &input.company_stability_citations,
        )
    {
        warnings.push(
            "Discarded company stability rationale/citations because the imported score is null"
                .to_string(),
        );
    }
    if employment_score.is_none()
        && metadata_present(
            &input.employment_stability_rationale,
            &input.employment_stability_citations,
        )
    {
        warnings.push(
            "Discarded employment stability rationale/citations because the imported score is null"
                .to_string(),
        );
    }
    let changed = score_changed(input.company_stability.clone(), current.company_stability)
        || score_changed(
            input.employment_stability.clone(),
            current.employment_stability,
        )
        || metadata_changed(
            &input.company_stability_rationale,
            &input.company_stability_citations,
            current.company_stability_rationale.as_ref(),
            current.company_stability_citations.as_ref(),
            company_score,
        )
        || metadata_changed(
            &input.employment_stability_rationale,
            &input.employment_stability_citations,
            current.employment_stability_rationale.as_ref(),
            current.employment_stability_citations.as_ref(),
            employment_score,
        );
    Ok((changed, warnings))
}

fn score_changed(field: OptionalField<Option<i64>>, current: Option<i64>) -> bool {
    match field {
        OptionalField::Omitted => false,
        OptionalField::Present(value) => value != current,
    }
}

fn metadata_present<T>(
    rationale: &OptionalField<Option<String>>,
    citations: &OptionalField<Option<Vec<T>>>,
) -> bool {
    matches!(rationale, OptionalField::Present(Some(value)) if !value.trim().is_empty())
        || matches!(citations, OptionalField::Present(Some(values)) if !values.is_empty())
}

fn metadata_changed<T: PartialEq>(
    rationale: &OptionalField<Option<String>>,
    citations: &OptionalField<Option<Vec<T>>>,
    current_rationale: Option<&String>,
    current_citations: Option<&Vec<T>>,
    score: Option<i64>,
) -> bool {
    if score.is_none() {
        return false;
    }
    let rationale_changed = match rationale {
        OptionalField::Omitted => false,
        OptionalField::Present(value) => value.as_ref() != current_rationale,
    };
    let citations_changed = match citations {
        OptionalField::Omitted => false,
        OptionalField::Present(value) => value.as_ref() != current_citations,
    };
    rationale_changed || citations_changed
}

#[cfg(test)]
mod database_import_tests {
    use super::*;
    use crate::db::migrate::migrate;
    use crate::util::now_iso;
    use proptest::prelude::*;
    use rusqlite::Connection;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn company(conn: &Connection, id: &str, name: &str) {
        conn.execute(
            "INSERT INTO companies (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            rusqlite::params![id, name, now_iso()],
        )
        .unwrap();
    }

    #[test]
    fn tracker_name_precedence_and_name_fallback_match_existing_companies() {
        let conn = connection();
        company(&conn, "acme", "Acme");
        company(&conn, "beta", "Beta");
        let file = parse_score_file(
            r#"{"companies":[
                {"name":"","tracker_names":["Acme"],"score":4},
                {"name":"Beta","tracker_names":[],"score":3}
            ]}"#,
        )
        .unwrap();
        let plan = plan_import(&conn, &file, true).unwrap();
        assert_eq!(plan.summary.applied.len(), 2);
        assert_eq!(
            plan.summary.applied[0].company_name.as_deref(),
            Some("Acme")
        );
        assert_eq!(
            plan.summary.applied[1].company_name.as_deref(),
            Some("Beta")
        );
    }

    #[test]
    fn ambiguity_and_duplicate_entries_form_one_group_and_write_nothing() {
        let conn = connection();
        company(&conn, "a", "Acme");
        company(&conn, "b", "Beta");
        let file = parse_score_file(
            r#"{"companies":[
                {"name":"ignored","tracker_names":["Acme","Beta"],"score":4},
                {"name":"Acme","score":3}
            ]}"#,
        )
        .unwrap();
        let plan = plan_import(&conn, &file, true).unwrap();
        assert_eq!(plan.summary.conflicts.len(), 1);
        assert!(plan.summary.conflicts[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("merged"));
        assert!(plan.summary.applied.is_empty());
        assert_eq!(
            conn.query_row(
                "SELECT company_stability FROM companies WHERE id = 'a'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn null_and_omitted_fields_apply_with_import_metadata_and_as_of() {
        let conn = connection();
        company(&conn, "a", "Acme");
        let first = parse_score_file(
            r#"{"as_of":"2026-11-02","companies":[{"name":"Acme","score":4,"rationale":" reason "}]}"#,
        )
        .unwrap();
        let first_plan = plan_import(&conn, &first, true).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::db::paths::DataPaths::from_data_dir(dir.path().to_path_buf());
        let result = apply_import(&conn, &paths, first_plan).unwrap();
        assert_eq!(result.applied.len(), 1);
        let stored = crate::company_scores::ScoreService::read(&conn, "a").unwrap();
        assert_eq!(stored.company_stability, Some(4));
        assert_eq!(
            stored.company_stability_scored_by.as_deref(),
            Some("import")
        );
        assert_eq!(
            stored.company_stability_scored_at.as_deref(),
            Some("2026-11-02T00:00:00.000Z")
        );
        assert_eq!(
            stored.company_stability_rationale.as_deref(),
            Some("reason")
        );

        let second = parse_score_file(
            r#"{"as_of":"2027-01-01","companies":[{"name":"Acme","score":null,"employment_stability":3}]}"#,
        )
        .unwrap();
        let second_plan = plan_import(&conn, &second, true).unwrap();
        let result = apply_import(&conn, &paths, second_plan).unwrap();
        assert_eq!(result.applied.len(), 1);
        let stored = crate::company_scores::ScoreService::read(&conn, "a").unwrap();
        assert_eq!(stored.company_stability, None);
        assert_eq!(stored.company_stability_rationale, None);
        assert_eq!(stored.employment_stability, Some(3));
    }

    #[test]
    fn importer_preserves_omitted_score_metadata_and_reports_discarded_null_metadata() {
        let conn = connection();
        company(&conn, "a", "Acme");
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::db::paths::DataPaths::from_data_dir(dir.path().to_path_buf());

        let first = parse_score_file(
            r#"{"as_of":"2026-11-02","companies":[{"name":"Acme","score":4,"rationale":"cash flow","citations":["https://example.com/source"],"employment_stability":3,"employment_rationale":"steady"}]}"#,
        )
        .unwrap();
        apply_import(&conn, &paths, plan_import(&conn, &first, false).unwrap()).unwrap();
        let before = crate::company_scores::ScoreService::read(&conn, "a").unwrap();

        let second = parse_score_file(
            r#"{"as_of":"2027-01-01","companies":[{"name":"Acme","employment_rationale":"updated"}]}"#,
        )
        .unwrap();
        apply_import(&conn, &paths, plan_import(&conn, &second, false).unwrap()).unwrap();
        let after = crate::company_scores::ScoreService::read(&conn, "a").unwrap();
        assert_eq!(after.company_stability, before.company_stability);
        assert_eq!(
            after.company_stability_rationale,
            before.company_stability_rationale
        );
        assert_eq!(
            after.company_stability_citations,
            before.company_stability_citations
        );
        assert_eq!(
            after.company_stability_scored_at,
            before.company_stability_scored_at
        );
        assert_eq!(
            after.company_stability_scored_by,
            before.company_stability_scored_by
        );
        assert_eq!(after.employment_stability, Some(3));
        assert_eq!(
            after.employment_stability_rationale.as_deref(),
            Some("updated")
        );

        let warning_file = parse_score_file(
            r#"{"companies":[{"name":"Acme","score":null,"rationale":"discard me"}]}"#,
        )
        .unwrap();
        let warning_plan = plan_import(&conn, &warning_file, true).unwrap();
        assert!(warning_plan.summary.warnings.iter().any(|item| item
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("Discarded company stability")));
    }

    #[test]
    fn importer_citation_count_precedes_invalid_url_validation() {
        let citations = (0..=MAX_CITATIONS)
            .map(|index| {
                if index == MAX_CITATIONS {
                    "not a URL".to_string()
                } else {
                    format!("https://example.com/{index}")
                }
            })
            .collect::<Vec<_>>();
        let input = serde_json::json!({
            "companies": [{"name": "Acme", "citations": citations}]
        });
        let file = parse_score_file(&input.to_string()).unwrap();
        assert_eq!(file.entries.len(), 0);
        let reason = file.invalid[0].reason.as_deref().unwrap();
        assert!(reason.contains("at most 20"), "{reason}");
        assert!(!reason.contains("invalid URL"), "{reason}");
    }

    #[test]
    fn mirror_refresh_failure_keeps_committed_entry_and_reports_failure() {
        let conn = connection();
        company(&conn, "a", "Acme");
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::db::paths::DataPaths::from_data_dir(dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        std::fs::create_dir(&paths.jobs_csv_path).unwrap();
        let file = parse_score_file(r#"{"companies":[{"name":"Acme","score":4}]}"#).unwrap();

        let result =
            apply_import(&conn, &paths, plan_import(&conn, &file, false).unwrap()).unwrap();
        assert_eq!(result.applied.len(), 1);
        assert_eq!(result.failed.len(), 1);
        assert!(result.failed[0]
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("CSV mirror refresh failed"));
        assert_eq!(
            crate::company_scores::ScoreService::read(&conn, "a")
                .unwrap()
                .company_stability,
            Some(4)
        );
    }

    // Feature: company-employment-stability, Property 12: Score-file planning is deterministic and idempotent
    // Validates: Requirements 10.1, 10.2, 10.3, 10.6, 10.7, 10.8, 10.10, 10.11, 10.13, 10.14, 10.15, 10.16, 10.17, 10.18, 10.21, 10.24, 10.25
    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(100))]

        #[test]
        fn prop_score_file_plan_apply_and_repeat_are_deterministic(scores in proptest::collection::vec(1i64..=5, 1..=8)) {
            let conn = connection();
            let companies = scores.iter().enumerate().map(|(index, score)| {
                let name = format!("Company {index}");
                company(&conn, &format!("company-{index}"), &name);
                serde_json::json!({"name": name, "score": score, "employment_stability": score})
            }).collect::<Vec<_>>();
            let file = parse_score_file(&serde_json::json!({
                "as_of": "2026-11-02",
                "companies": companies,
            }).to_string()).unwrap();

            let dry = plan_import(&conn, &file, true).unwrap();
            let apply_plan = plan_import(&conn, &file, false).unwrap();
            prop_assert_eq!(&dry.summary.applied, &apply_plan.summary.applied);
            prop_assert_eq!(&dry.summary.unchanged, &apply_plan.summary.unchanged);
            prop_assert_eq!(&dry.summary.unmatched, &apply_plan.summary.unmatched);
            prop_assert_eq!(&dry.summary.conflicts, &apply_plan.summary.conflicts);
            prop_assert_eq!(&dry.summary.invalid, &apply_plan.summary.invalid);
            prop_assert_eq!(&dry.summary.warnings, &apply_plan.summary.warnings);

            let dir = tempfile::tempdir().unwrap();
            let paths = crate::db::paths::DataPaths::from_data_dir(dir.path().to_path_buf());
            let first = apply_import(&conn, &paths, apply_plan).unwrap();
            prop_assert_eq!(first.applied.len(), scores.len());
            prop_assert!(first.failed.is_empty());

            let second_plan = plan_import(&conn, &file, true).unwrap();
            prop_assert!(second_plan.summary.applied.is_empty());
            prop_assert_eq!(second_plan.summary.unchanged.len(), scores.len());
            let second = apply_import(&conn, &paths, second_plan).unwrap();
            prop_assert!(second.applied.is_empty());
            prop_assert_eq!(second.unchanged.len(), scores.len());
            prop_assert!(second.failed.is_empty());
        }
    }

    // Feature: company-employment-stability, Property 13: Import matching conflicts are merged per company
    // Validates: Requirements 10.11, 10.12, 10.13, 10.14
    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(100))]

        #[test]
        fn prop_ambiguous_and_duplicate_entries_merge_into_one_conflict(scores in proptest::collection::vec(1i64..=5, 1..=6)) {
            let conn = connection();
            company(&conn, "a", "Acme");
            company(&conn, "b", "Beta");
            let mut entries = vec![serde_json::json!({
                "name": "Acme",
                "tracker_names": ["Acme", "Beta"],
                "score": scores[0],
            })];
            entries.extend(scores.iter().enumerate().map(|(index, score)| serde_json::json!({
                "name": "Acme",
                "score": score,
                "rationale": format!("entry {index}"),
            })));
            let file = parse_score_file(&serde_json::json!({"companies": entries}).to_string()).unwrap();
            let plan = plan_import(&conn, &file, true).unwrap();

            prop_assert_eq!(plan.summary.conflicts.len(), 1);
            let conflict = &plan.summary.conflicts[0];
            prop_assert!(conflict.reason.as_deref().unwrap_or_default().contains("merged"));
            prop_assert!(conflict.name.contains("[0]"));
            for position in 0..=scores.len() {
                let marker = format!("[{position}]");
                prop_assert!(conflict.name.contains(&marker), "missing conflict position");
            }
            prop_assert!(plan.summary.applied.is_empty());
            prop_assert!(plan.summary.unchanged.is_empty());
            prop_assert_eq!(crate::company_scores::ScoreService::read(&conn, "a").unwrap().company_stability, None);
            prop_assert_eq!(crate::company_scores::ScoreService::read(&conn, "b").unwrap().company_stability, None);
        }
    }

    // Feature: company-employment-stability, Property 15: Score scope never changes posting scores or favorites
    // Validates: Requirements 2.3, 3.14, 3.16, 3.17, 6.18, 10.19, 12.5, 12.6, 12.7
    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(100))]

        #[test]
        fn prop_import_scores_preserves_posting_fields_and_events(company_score in 1i64..=5, employment_score in 1i64..=5) {
            let conn = connection();
            let (first, _) = crate::jobs::service::create_job_from_url(
                &conn, "https://example.com/jobs/one", "One", Some("Acme"), Some("wishlist"), None, None, None,
            ).unwrap();
            let (second, _) = crate::jobs::service::create_job_from_url(
                &conn, "https://example.com/jobs/two", "Two", Some("Acme"), Some("wishlist"), None, None, None,
            ).unwrap();
            conn.execute(
                "UPDATE jobs SET appeal = 2, application_ease = 5, is_favorite = 1, status = 'applied', notes = 'keep these fields' WHERE id IN (?1, ?2)",
                rusqlite::params![first.id, second.id],
            ).unwrap();
            let snapshot = |id: &str| -> (Option<i64>, Option<i64>, i64, String, Option<String>, String) {
                conn.query_row(
                    "SELECT appeal, application_ease, is_favorite, status, notes, updated_at FROM jobs WHERE id = ?1",
                    rusqlite::params![id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
                ).unwrap()
            };
            let before = vec![snapshot(&first.id), snapshot(&second.id)];
            let events_before: i64 = conn.query_row("SELECT COUNT(*) FROM job_events", [], |row| row.get(0)).unwrap();

            let file = parse_score_file(&serde_json::json!({
                "companies": [{"name": "Acme", "score": company_score, "employment_stability": employment_score}],
            }).to_string()).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let paths = crate::db::paths::DataPaths::from_data_dir(dir.path().to_path_buf());
            apply_import(&conn, &paths, plan_import(&conn, &file, false).unwrap()).unwrap();

            prop_assert_eq!(before, vec![snapshot(&first.id), snapshot(&second.id)]);
            let events_after: i64 = conn.query_row("SELECT COUNT(*) FROM job_events", [], |row| row.get(0)).unwrap();
            prop_assert_eq!(events_before, events_after);
        }
    }

    #[test]
    fn failed_entry_rolls_back_and_later_entry_continues() {
        let conn = connection();
        company(&conn, "a", "Acme");
        company(&conn, "b", "Beta");
        conn.execute_batch(
            "CREATE TRIGGER fail_acme_score BEFORE UPDATE OF company_stability ON companies
             WHEN OLD.id = 'a' BEGIN SELECT RAISE(ABORT, 'blocked company'); END;",
        )
        .unwrap();
        let file = parse_score_file(
            r#"{"companies":[{"name":"Acme","score":4},{"name":"Beta","score":3}]}"#,
        )
        .unwrap();
        let plan = plan_import(&conn, &file, true).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::db::paths::DataPaths::from_data_dir(dir.path().to_path_buf());
        let result = apply_import(&conn, &paths, plan).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.applied.len(), 1);
        assert_eq!(
            conn.query_row(
                "SELECT company_stability FROM companies WHERE id = 'a'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .unwrap(),
            None
        );
        assert_eq!(
            conn.query_row(
                "SELECT company_stability FROM companies WHERE id = 'b'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .unwrap(),
            Some(3)
        );
    }
}
