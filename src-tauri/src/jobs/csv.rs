use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::service::{
    add_job_event, create_job_from_url, get_job_by_id, job_cols, map_job, update_job,
    UpdateJobInput, JOB_COL_COUNT,
};
use crate::models::{is_job_status, Job};
use crate::util::{normalize_canonical_url, now_iso};

pub const CSV_HEADERS: &[&str] = &[
    "id",
    "company",
    "csv_rev",
    "url",
    "canonical_url",
    "title",
    "status",
    "applied_at",
    "notes",
    "location",
    "latest_note",
    "source",
    "posting_state",
    "updated_at",
    "appeal",
    "application_ease",
    "company_stability",
    "employment_stability",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EditableFields {
    pub url: String,
    pub title: String,
    pub company: String,
    pub status: String,
    pub applied_at: Option<String>,
    pub notes: Option<String>,
    pub location: Option<String>,
    pub latest_note: Option<String>,
    /// Overall appeal, 1–5 (5 = most appealing). `None` is unscored.
    #[serde(default)]
    pub appeal: Option<i64>,
    /// True when the CSV file has an `appeal` column. Absent columns must not
    /// clear a score that was set in the app. Not stored in the sync sidecar.
    #[serde(skip)]
    pub appeal_in_file: bool,
    /// How easy the posting is to apply to, 1–5 (5 = easiest). `None` is unscored.
    #[serde(default)]
    pub application_ease: Option<i64>,
    /// True when the CSV file has an `application_ease` column. Absent columns
    /// must not clear a score that was set in the app. Not stored in the sync sidecar.
    #[serde(skip)]
    pub application_ease_in_file: bool,
    /// Company stability, 1–5 (5 = most stable). `None` is unscored.
    #[serde(default)]
    pub company_stability: Option<i64>,
    /// True when the CSV file has a `company_stability` column. Absent columns
    /// must not clear a score that was set in the app. Not stored in the sync sidecar.
    #[serde(skip)]
    pub company_stability_in_file: bool,
    /// Employment stability, 1–5 (5 = most secure). `None` is unscored.
    #[serde(default)]
    pub employment_stability: Option<i64>,
    /// True when the CSV file has an `employment_stability` column. Absent
    /// columns must not clear a score set in the app.
    #[serde(skip)]
    pub employment_stability_in_file: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncRowState {
    csv_rev: i64,
    fields: EditableFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncState {
    path: String,
    last_export_at: Option<String>,
    last_import_at: Option<String>,
    rows: HashMap<String, SyncRowState>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub exported_at: String,
    pub row_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    #[default]
    Merge,
    OverwriteEditable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub conflicts: usize,
    pub skipped: usize,
    pub notes_added: usize,
    pub missing_from_csv: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportConflict {
    pub key: String,
    pub fields: Vec<String>,
    pub db: serde_json::Value,
    pub csv: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub dry_run: bool,
    pub mode: String,
    pub path: String,
    pub summary: ImportSummary,
    pub conflicts: Vec<ImportConflict>,
    pub errors: Vec<serde_json::Value>,
    pub changes: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvStatus {
    pub path: String,
    pub exists: bool,
    pub file_mtime: Option<String>,
    pub last_export_at: Option<String>,
    pub last_import_at: Option<String>,
    pub row_count_db: usize,
    pub row_count_csv: Option<usize>,
    pub drift: bool,
}

fn normalize_nullable(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn values_equal(a: Option<&str>, b: Option<&str>) -> bool {
    normalize_nullable(a) == normalize_nullable(b)
}

fn escape_csv_field(value: &str) -> String {
    if value.contains(['"', ',', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn serialize_csv(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for row in rows {
        let line = row
            .iter()
            .map(|c| escape_csv_field(c))
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let char = chars[i];
        let next = chars.get(i + 1).copied();
        if in_quotes {
            if char == '"' && next == Some('"') {
                field.push('"');
                i += 2;
                continue;
            } else if char == '"' {
                in_quotes = false;
            } else {
                field.push(char);
            }
            i += 1;
            continue;
        }
        if char == '"' {
            in_quotes = true;
            i += 1;
            continue;
        }
        if char == ',' {
            row.push(std::mem::take(&mut field));
            i += 1;
            continue;
        }
        if char == '\n' {
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
            i += 1;
            continue;
        }
        if char == '\r' {
            i += 1;
            continue;
        }
        field.push(char);
        i += 1;
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.into_iter()
        .filter(|r| r.iter().any(|c| !c.trim().is_empty()))
        .collect()
}

fn empty_sync_state(csv_path: &Path) -> SyncState {
    SyncState {
        path: csv_path.display().to_string(),
        last_export_at: None,
        last_import_at: None,
        rows: HashMap::new(),
    }
}

fn sync_path_for(csv_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.sync.json", csv_path.display()))
}

fn read_sync_state(csv_path: &Path) -> SyncState {
    let sync_path = sync_path_for(csv_path);
    if !sync_path.exists() {
        return empty_sync_state(csv_path);
    }
    match fs::read_to_string(&sync_path) {
        Ok(text) => {
            serde_json::from_str::<SyncState>(&text).unwrap_or_else(|_| empty_sync_state(csv_path))
        }
        Err(_) => empty_sync_state(csv_path),
    }
}

fn write_sync_state(state: &SyncState) -> AppResult<()> {
    let sync_path = sync_path_for(Path::new(&state.path));
    let temp_path = format!("{}.{}.tmp", sync_path.display(), std::process::id());
    let json = serde_json::to_string_pretty(state).map_err(|e| AppError::from(e.to_string()))?;
    {
        let mut f = fs::File::create(&temp_path)?;
        f.write_all(json.as_bytes())?;
        f.write_all(b"\n")?;
    }
    fs::rename(&temp_path, &sync_path)?;
    Ok(())
}

fn load_job_rows(conn: &Connection) -> AppResult<Vec<(Job, String)>> {
    // NON-DESTRUCTIVE CSV CONTRACT (Requirements 18.2, 18.3, 18.4):
    // CSV export row selection is intentionally INDEPENDENT of watchlist
    // Filter_Criteria and the `jobs.watch_filtered` hint. This SELECT chooses
    // rows solely by the existing pipeline-tracking columns
    // (`is_new_from_watch`, `watch_disposition`) — never by filter match
    // results. Filtering only affects on-screen watch listings; it must never
    // change what is stored or exported here. Do NOT add references to
    // `filter_criteria`, `watch_filtered`, or the filter engine to this query.
    //
    // Pending watch discoveries and dismissed watch roles stay out of jobs.csv —
    // only pipeline jobs the user is tracking are exported.
    let mut stmt = conn.prepare(&format!(
        "SELECT {}, c.name
         FROM jobs j INNER JOIN companies c ON j.company_id = c.id
         WHERE j.is_new_from_watch = 0
           AND (j.watch_disposition IS NULL OR j.watch_disposition != 'dismissed')",
        job_cols(Some("j"))
    ))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((map_job(row)?, row.get::<_, String>(JOB_COL_COUNT)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn editable_from_db(job: &Job, company_name: &str, latest_note: Option<String>) -> EditableFields {
    EditableFields {
        url: job.url.clone(),
        title: job.title.clone(),
        company: company_name.to_string(),
        status: job.status.clone(),
        applied_at: job.applied_at.clone(),
        notes: job.notes.clone(),
        location: job.location.clone(),
        latest_note,
        appeal: job.appeal,
        appeal_in_file: false,
        application_ease: job.application_ease,
        application_ease_in_file: false,
        company_stability: job.company_stability,
        company_stability_in_file: false,
        employment_stability: job.employment_stability,
        employment_stability_in_file: false,
    }
}

pub fn export_jobs_csv(
    conn: &Connection,
    csv_path: &Path,
    latest_note_overrides: Option<&HashMap<String, Option<String>>>,
) -> AppResult<ExportResult> {
    export_jobs_csv_with_last_import_at(conn, csv_path, latest_note_overrides, None)
}

fn export_jobs_csv_with_last_import_at(
    conn: &Connection,
    csv_path: &Path,
    latest_note_overrides: Option<&HashMap<String, Option<String>>>,
    last_import_at: Option<String>,
) -> AppResult<ExportResult> {
    let sync = read_sync_state(csv_path);
    let rows = load_job_rows(conn)?;
    let mut next_rows = HashMap::new();
    let mut csv_rows = vec![CSV_HEADERS
        .iter()
        .map(|s| (*s).to_string())
        .collect::<Vec<_>>()];

    for (job, company_name) in &rows {
        let previous = sync.rows.get(&job.id);
        let latest_note = if let Some(overrides) = latest_note_overrides {
            if overrides.contains_key(&job.id) {
                overrides.get(&job.id).cloned().flatten()
            } else {
                previous.and_then(|p| p.fields.latest_note.clone())
            }
        } else {
            previous.and_then(|p| p.fields.latest_note.clone())
        };
        let fields = editable_from_db(job, company_name, latest_note);
        let csv_rev = previous.map(|p| p.csv_rev).unwrap_or(0) + 1;
        next_rows.insert(
            job.id.clone(),
            SyncRowState {
                csv_rev,
                fields: fields.clone(),
            },
        );
        csv_rows.push(vec![
            job.id.clone(),
            fields.company,
            csv_rev.to_string(),
            fields.url,
            job.canonical_url.clone(),
            fields.title,
            fields.status,
            fields.applied_at.unwrap_or_default(),
            fields.notes.unwrap_or_default(),
            fields.location.unwrap_or_default(),
            fields.latest_note.unwrap_or_default(),
            job.source.clone(),
            job.posting_state.clone(),
            job.updated_at.clone(),
            fields
                .appeal
                .map(|score| score.to_string())
                .unwrap_or_default(),
            fields
                .application_ease
                .map(|score| score.to_string())
                .unwrap_or_default(),
            fields
                .company_stability
                .map(|score| score.to_string())
                .unwrap_or_default(),
            fields
                .employment_stability
                .map(|score| score.to_string())
                .unwrap_or_default(),
        ]);
    }

    if let Some(parent) = csv_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = format!("{}.{}.tmp", csv_path.display(), std::process::id());
    fs::write(&temp_path, serialize_csv(&csv_rows))?;
    fs::rename(&temp_path, csv_path)?;

    let exported_at = fs::metadata(csv_path)?
        .modified()
        .ok()
        .map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339()
        })
        .unwrap_or_else(now_iso);

    write_sync_state(&SyncState {
        path: csv_path.display().to_string(),
        last_export_at: Some(exported_at.clone()),
        last_import_at: last_import_at.or(sync.last_import_at),
        rows: next_rows,
    })?;

    Ok(ExportResult {
        path: csv_path.display().to_string(),
        exported_at,
        row_count: rows.len(),
    })
}

pub fn get_jobs_csv_status(conn: &Connection, csv_path: &Path) -> AppResult<CsvStatus> {
    let sync = read_sync_state(csv_path);
    let exists = csv_path.exists();
    let mut file_mtime = None;
    let mut row_count_csv = None;
    if exists {
        if let Ok(meta) = fs::metadata(csv_path) {
            if let Ok(modified) = meta.modified() {
                let dt: chrono::DateTime<chrono::Utc> = modified.into();
                file_mtime = Some(dt.to_rfc3339());
            }
        }
        if let Ok(text) = fs::read_to_string(csv_path) {
            let parsed = parse_csv(&text);
            row_count_csv = Some(parsed.len().saturating_sub(1));
        }
    }
    let row_count_db = load_job_rows(conn)?.len();
    let drift = exists
        && (sync.last_export_at.is_none()
            || file_mtime
                .as_ref()
                .zip(sync.last_export_at.as_ref())
                .map(|(m, e)| m > e)
                .unwrap_or(false)
            || row_count_csv.map(|c| c != row_count_db).unwrap_or(false));

    Ok(CsvStatus {
        path: csv_path.display().to_string(),
        exists,
        file_mtime,
        last_export_at: sync.last_export_at,
        last_import_at: sync.last_import_at,
        row_count_db,
        row_count_csv,
        drift,
    })
}

fn company_name_for_job(conn: &Connection, company_id: &str) -> AppResult<String> {
    let name: Option<String> = conn
        .query_row(
            "SELECT name FROM companies WHERE id = ?1",
            params![company_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    Ok(name.unwrap_or_else(|| "Unknown company".into()))
}

fn find_job_by_id_or_canonical(
    conn: &Connection,
    id: Option<&str>,
    url: &str,
) -> AppResult<Option<Job>> {
    if let Some(id) = id {
        if let Some(job) = get_job_by_id(conn, id)? {
            return Ok(Some(job));
        }
    }
    if url.trim().is_empty() {
        return Ok(None);
    }
    let canonical = normalize_canonical_url(url).map_err(AppError::from)?;
    let job = conn
        .query_row(
            &format!(
                "SELECT {} FROM jobs j INNER JOIN companies c ON j.company_id = c.id WHERE j.canonical_url = ?1",
                job_cols(Some("j"))
            ),
            params![canonical],
            map_job,
        )
        .optional()
        .map_err(map_sqlite)?;
    Ok(job)
}

const EDITABLE_KEYS: &[&str] = &[
    "url",
    "title",
    "company",
    "status",
    "appliedAt",
    "notes",
    "location",
    "latestNote",
    "appeal",
    "applicationEase",
    "companyStability",
    "employmentStability",
];

fn parse_score_cell(raw: &str, label: &str, hint: &str) -> Result<Option<i64>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed.parse::<i64>() {
        Ok(score) if (1..=5).contains(&score) => Ok(Some(score)),
        _ => Err(format!(
            "Invalid {label} '{trimmed}'. Use an integer from 1 to 5 ({hint}), or leave blank"
        )),
    }
}

fn parse_appeal_cell(raw: &str) -> Result<Option<i64>, String> {
    parse_score_cell(raw, "appeal", "5 = most appealing")
}

fn parse_application_ease_cell(raw: &str) -> Result<Option<i64>, String> {
    parse_score_cell(raw, "application ease", "5 = easiest to apply")
}

fn parse_company_stability_cell(raw: &str) -> Result<Option<i64>, String> {
    parse_score_cell(raw, "company stability", "5 = most stable")
}

fn parse_employment_stability_cell(raw: &str) -> Result<Option<i64>, String> {
    parse_score_cell(raw, "employment stability", "5 = most secure")
}

fn field_norm(fields: &EditableFields, key: &str) -> Option<String> {
    let raw = match key {
        "url" => Some(fields.url.as_str()),
        "title" => Some(fields.title.as_str()),
        "company" => Some(fields.company.as_str()),
        "status" => Some(fields.status.as_str()),
        "appliedAt" => fields.applied_at.as_deref(),
        "notes" => fields.notes.as_deref(),
        "location" => fields.location.as_deref(),
        "latestNote" => fields.latest_note.as_deref(),
        "appeal" => return fields.appeal.map(|score| score.to_string()),
        "applicationEase" => return fields.application_ease.map(|score| score.to_string()),
        "companyStability" => return fields.company_stability.map(|score| score.to_string()),
        "employmentStability" => return fields.employment_stability.map(|score| score.to_string()),
        _ => None,
    };
    normalize_nullable(raw)
}

fn open_csv_conn(db_path: &Path) -> AppResult<Connection> {
    let conn = Connection::open(db_path).map_err(map_sqlite)?;
    conn.pragma_update(None, "busy_timeout", 5000i32)
        .map_err(map_sqlite)?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(map_sqlite)?;
    Ok(conn)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScoreCellPlan {
    Apply,
    Block,
}

#[derive(Debug, Clone)]
struct ParsedCsvRow {
    row_number: usize,
    key: String,
    id: Option<String>,
    fields: EditableFields,
}

#[derive(Debug, Clone)]
struct ScoreCandidate {
    row_number: usize,
    key: String,
    value: Option<i64>,
    stored_conflict: bool,
}

fn destination_company_key(
    conn: &Connection,
    existing: Option<&Job>,
    company_name: &str,
) -> AppResult<String> {
    let trimmed = company_name.trim();
    let id: Option<String> = conn
        .query_row(
            "SELECT id FROM companies WHERE name = ?1 COLLATE NOCASE",
            params![trimmed],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    if let Some(id) = id {
        return Ok(id);
    }
    if existing.is_some_and(|job| job.company_id == trimmed) {
        return Ok(trimmed.to_string());
    }
    Ok(format!("name:{}", trimmed.to_ascii_lowercase()))
}

fn company_score_for_name(
    conn: &Connection,
    company_name: &str,
    field: &str,
) -> AppResult<Option<i64>> {
    let sql = match field {
        "company_stability" => {
            "SELECT company_stability FROM companies WHERE name = ?1 COLLATE NOCASE"
        }
        "employment_stability" => {
            "SELECT employment_stability FROM companies WHERE name = ?1 COLLATE NOCASE"
        }
        _ => return Err(AppError::from("Unsupported company score field")),
    };
    let value = conn
        .query_row(sql, params![company_name.trim()], |row| row.get(0))
        .optional()
        .map_err(map_sqlite)?;
    Ok(value)
}

fn add_score_conflict(
    result: &mut ImportResult,
    destination: &str,
    field: &str,
    candidates: &[ScoreCandidate],
    reason: &str,
) {
    let rows: Vec<usize> = candidates
        .iter()
        .map(|candidate| candidate.row_number)
        .collect();
    let values: Vec<Option<i64>> = candidates.iter().map(|candidate| candidate.value).collect();
    result.summary.conflicts += 1;
    result.conflicts.push(ImportConflict {
        key: format!("company:{destination}"),
        fields: vec![field.to_string()],
        db: serde_json::json!({"rows": rows, "reason": reason}),
        csv: serde_json::json!({"rows": rows, "values": values}),
    });
}

fn plan_score_cells(
    conn: &Connection,
    sync: &SyncState,
    rows: &[ParsedCsvRow],
    mode: &ImportMode,
    result: &mut ImportResult,
) -> AppResult<HashMap<String, HashMap<String, ScoreCellPlan>>> {
    let mut groups: HashMap<(String, String), Vec<ScoreCandidate>> = HashMap::new();
    for row in rows {
        let existing = find_job_by_id_or_canonical(conn, row.id.as_deref(), &row.fields.url)?;
        let Some(existing) = existing else {
            for (field, present, value) in [
                (
                    "companyStability",
                    row.fields.company_stability_in_file,
                    row.fields.company_stability,
                ),
                (
                    "employmentStability",
                    row.fields.employment_stability_in_file,
                    row.fields.employment_stability,
                ),
            ] {
                if present && value.is_some() {
                    let destination = destination_company_key(conn, None, &row.fields.company)?;
                    groups
                        .entry((destination.clone(), field.to_string()))
                        .or_default()
                        .push(ScoreCandidate {
                            row_number: row.row_number,
                            key: row.key.clone(),
                            value,
                            stored_conflict: false,
                        });
                }
            }
            continue;
        };

        let company_name = company_name_for_job(conn, &existing.company_id)?;
        let baseline = sync
            .rows
            .get(&existing.id)
            .map(|state| state.fields.clone())
            .unwrap_or_else(|| editable_from_db(&existing, &company_name, None));
        let destination_name = row.fields.company.trim();
        let destination_key = destination_company_key(conn, Some(&existing), destination_name)?;
        let moved = !values_equal(Some(destination_name), Some(company_name.as_str()));
        for (field, present, value, baseline_value, target_value) in [
            (
                "companyStability",
                row.fields.company_stability_in_file,
                row.fields.company_stability,
                baseline.company_stability,
                if moved {
                    company_score_for_name(conn, destination_name, "company_stability")?
                } else {
                    existing.company_stability
                },
            ),
            (
                "employmentStability",
                row.fields.employment_stability_in_file,
                row.fields.employment_stability,
                baseline.employment_stability,
                if moved {
                    company_score_for_name(conn, destination_name, "employment_stability")?
                } else {
                    existing.employment_stability
                },
            ),
        ] {
            if !present {
                continue;
            }
            let csv_changed = match mode {
                ImportMode::Merge => value != baseline_value,
                ImportMode::OverwriteEditable => true,
            };
            if !csv_changed {
                continue;
            }
            let db_changed = !moved && target_value != baseline_value;
            groups
                .entry((destination_key.clone(), field.to_string()))
                .or_default()
                .push(ScoreCandidate {
                    row_number: row.row_number,
                    key: row.key.clone(),
                    value,
                    stored_conflict: db_changed,
                });
        }
    }

    let mut plans: HashMap<String, HashMap<String, ScoreCellPlan>> = HashMap::new();
    for ((destination, field), candidates) in groups {
        let disagreement = candidates
            .iter()
            .any(|candidate| candidate.value != candidates[0].value);
        let stored_conflict = candidates.iter().any(|candidate| candidate.stored_conflict);
        if disagreement || stored_conflict {
            add_score_conflict(
                result,
                &destination,
                &field,
                &candidates,
                if disagreement {
                    "changed score cells disagree"
                } else {
                    "stored company value wins"
                },
            );
            for candidate in candidates {
                plans
                    .entry(candidate.key)
                    .or_default()
                    .insert(field.clone(), ScoreCellPlan::Block);
            }
        } else if let Some(first) = candidates.first() {
            for candidate in &candidates {
                plans.entry(candidate.key.clone()).or_default().insert(
                    field.clone(),
                    if candidate.key == first.key {
                        ScoreCellPlan::Apply
                    } else {
                        ScoreCellPlan::Block
                    },
                );
            }
        }
    }
    Ok(plans)
}

pub fn import_jobs_csv(
    db_path: &Path,
    csv_path: &Path,
    content: Option<&str>,
    dry_run: bool,
    mode: ImportMode,
) -> AppResult<ImportResult> {
    let sync = read_sync_state(csv_path);
    let mode_str = match mode {
        ImportMode::Merge => "merge",
        ImportMode::OverwriteEditable => "overwrite_editable",
    };
    let mut result = ImportResult {
        dry_run,
        mode: mode_str.to_string(),
        path: csv_path.display().to_string(),
        summary: ImportSummary {
            created: 0,
            updated: 0,
            unchanged: 0,
            conflicts: 0,
            skipped: 0,
            notes_added: 0,
            missing_from_csv: 0,
        },
        conflicts: Vec::new(),
        errors: Vec::new(),
        changes: Vec::new(),
    };

    let text = if let Some(c) = content {
        c.to_string()
    } else {
        if !csv_path.exists() {
            return Err(AppError::from(format!(
                "CSV file not found: {}",
                csv_path.display()
            )));
        }
        fs::read_to_string(csv_path)?
    };

    let parsed = parse_csv(&text);
    if parsed.is_empty() {
        return Err(AppError::from("CSV is empty"));
    }
    let header = &parsed[0];
    let header_map: HashMap<String, usize> = header
        .iter()
        .enumerate()
        .map(|(i, h)| (h.trim().to_lowercase(), i))
        .collect();
    if !header_map.contains_key("url")
        || !header_map.contains_key("title")
        || !header_map.contains_key("company")
    {
        return Err(AppError::from(
            "CSV must include url, title, and company columns",
        ));
    }

    let mut seen_job_ids = HashSet::new();
    let mut imported_notes: HashMap<String, Option<String>> = HashMap::new();
    let conn = open_csv_conn(db_path)?;

    let mut csv_rows = Vec::new();
    for (i, row) in parsed.iter().skip(1).enumerate() {
        let row_number = i + 2;
        let get = |name: &str| -> String {
            header_map
                .get(name)
                .and_then(|idx| row.get(*idx))
                .cloned()
                .unwrap_or_default()
        };
        let appeal_in_file = header_map.contains_key("appeal");
        let appeal = if appeal_in_file {
            match parse_appeal_cell(&get("appeal")) {
                Ok(value) => value,
                Err(message) => {
                    result.summary.skipped += 1;
                    result.errors.push(serde_json::json!({
                        "row": row_number,
                        "message": message
                    }));
                    continue;
                }
            }
        } else {
            None
        };
        let application_ease_in_file = header_map.contains_key("application_ease");
        let application_ease = if application_ease_in_file {
            match parse_application_ease_cell(&get("application_ease")) {
                Ok(value) => value,
                Err(message) => {
                    result.summary.skipped += 1;
                    result.errors.push(serde_json::json!({
                        "row": row_number,
                        "message": message
                    }));
                    continue;
                }
            }
        } else {
            None
        };
        let company_stability_in_file = header_map.contains_key("company_stability");
        let company_stability = if company_stability_in_file {
            match parse_company_stability_cell(&get("company_stability")) {
                Ok(value) => value,
                Err(message) => {
                    result.summary.skipped += 1;
                    result.errors.push(serde_json::json!({
                        "row": row_number,
                        "message": message
                    }));
                    continue;
                }
            }
        } else {
            None
        };
        let employment_stability_in_file = header_map.contains_key("employment_stability");
        let employment_stability = if employment_stability_in_file {
            match parse_employment_stability_cell(&get("employment_stability")) {
                Ok(value) => value,
                Err(message) => {
                    result.summary.skipped += 1;
                    result.errors.push(serde_json::json!({
                        "row": row_number,
                        "message": message
                    }));
                    continue;
                }
            }
        } else {
            None
        };
        let csv_fields = EditableFields {
            url: get("url").trim().to_string(),
            title: get("title").trim().to_string(),
            company: get("company").trim().to_string(),
            status: get("status").trim().to_string(),
            applied_at: normalize_nullable(Some(&get("applied_at"))),
            notes: normalize_nullable(Some(&get("notes"))),
            location: normalize_nullable(Some(&get("location"))),
            latest_note: normalize_nullable(Some(&get("latest_note"))),
            appeal,
            appeal_in_file,
            application_ease,
            application_ease_in_file,
            company_stability,
            company_stability_in_file,
            employment_stability,
            employment_stability_in_file,
        };
        let id = normalize_nullable(Some(&get("id")));
        let key = id.clone().unwrap_or_else(|| {
            if csv_fields.url.is_empty() {
                format!("row-{row_number}")
            } else {
                csv_fields.url.clone()
            }
        });

        csv_rows.push(ParsedCsvRow {
            row_number,
            key,
            id,
            fields: csv_fields,
        });
    }

    let score_plans = plan_score_cells(&conn, &sync, &csv_rows, &mode, &mut result)?;
    for row in &csv_rows {
        match process_row(
            &conn,
            &sync,
            &mut result,
            &mut seen_job_ids,
            &mut imported_notes,
            row.row_number,
            &row.key,
            row.id.as_deref(),
            &row.fields,
            dry_run,
            &mode,
            score_plans.get(&row.key),
        ) {
            Ok(()) => {}
            Err(e) => {
                result.summary.skipped += 1;
                result.errors.push(serde_json::json!({
                    "row": row.row_number,
                    "message": e.to_string()
                }));
            }
        }
    }

    for job_id in sync.rows.keys() {
        if !seen_job_ids.contains(job_id) && get_job_by_id(&conn, job_id)?.is_some() {
            result.summary.missing_from_csv += 1;
        }
    }

    if !dry_run {
        export_jobs_csv_with_last_import_at(
            &conn,
            csv_path,
            Some(&imported_notes),
            Some(now_iso()),
        )?;
    }

    Ok(result)
}

fn process_row(
    conn: &Connection,
    sync: &SyncState,
    result: &mut ImportResult,
    seen_job_ids: &mut HashSet<String>,
    imported_notes: &mut HashMap<String, Option<String>>,
    row_number: usize,
    key: &str,
    id: Option<&str>,
    csv_fields: &EditableFields,
    dry_run: bool,
    mode: &ImportMode,
    score_plans: Option<&HashMap<String, ScoreCellPlan>>,
) -> AppResult<()> {
    let score_is_apply = |field: &str| {
        matches!(
            score_plans.and_then(|plans| plans.get(field)),
            Some(ScoreCellPlan::Apply)
        )
    };
    let existing = find_job_by_id_or_canonical(conn, id, &csv_fields.url)?;

    if existing.is_none() {
        if csv_fields.url.is_empty() || csv_fields.title.is_empty() || csv_fields.company.is_empty()
        {
            result.summary.skipped += 1;
            result.errors.push(serde_json::json!({
                "row": row_number,
                "message": "New jobs require url, title, and company"
            }));
            return Ok(());
        }
        if !csv_fields.status.is_empty() && !is_job_status(&csv_fields.status) {
            result.summary.skipped += 1;
            result.errors.push(serde_json::json!({
                "row": row_number,
                "message": format!("Invalid status: {}", csv_fields.status)
            }));
            return Ok(());
        }
        if !dry_run {
            let title = csv_fields.title.clone();
            let created = create_job_from_url(
                conn,
                &csv_fields.url,
                &title,
                Some(&csv_fields.company),
                if csv_fields.status.is_empty() {
                    None
                } else {
                    Some(&csv_fields.status)
                },
                csv_fields.applied_at.as_deref(),
                csv_fields.notes.as_deref(),
                csv_fields.location.as_deref(),
            )?;
            if let Some(note) = &csv_fields.latest_note {
                add_job_event(conn, &created.0.id, "csv_note", Some(note))?;
                imported_notes.insert(created.0.id.clone(), Some(note.clone()));
                result.summary.notes_added += 1;
                result.changes.push(serde_json::json!({
                    "action": "note",
                    "jobId": created.0.id,
                    "fields": ["latestNote"]
                }));
            }
            if csv_fields.appeal.is_some()
                || score_is_apply("companyStability")
                || score_is_apply("employmentStability")
                || csv_fields.application_ease.is_some()
            {
                let mut updates = UpdateJobInput::default();
                if csv_fields.appeal.is_some() {
                    updates.appeal = Some(csv_fields.appeal);
                }
                if csv_fields.application_ease.is_some() {
                    updates.application_ease = Some(csv_fields.application_ease);
                }
                if score_is_apply("companyStability") {
                    updates.company_stability = Some(csv_fields.company_stability);
                }
                if score_is_apply("employmentStability") {
                    updates.employment_stability = Some(csv_fields.employment_stability);
                }
                update_job(conn, &created.0.id, updates)?;
            }
            seen_job_ids.insert(created.0.id.clone());
            result.changes.push(serde_json::json!({
                "action": "create",
                "jobId": created.0.id
            }));
        } else {
            result.changes.push(serde_json::json!({"action": "create"}));
        }
        result.summary.created += 1;
        return Ok(());
    }

    let existing = existing.unwrap();
    seen_job_ids.insert(existing.id.clone());

    let company_name = company_name_for_job(conn, &existing.company_id)?;
    let baseline = sync
        .rows
        .get(&existing.id)
        .map(|r| r.fields.clone())
        .unwrap_or_else(|| editable_from_db(&existing, &company_name, None));
    let db_fields = editable_from_db(&existing, &company_name, baseline.latest_note.clone());

    if !csv_fields.status.is_empty() && !is_job_status(&csv_fields.status) {
        result.summary.skipped += 1;
        result.errors.push(serde_json::json!({
            "row": row_number,
            "message": format!("Invalid status: {}", csv_fields.status)
        }));
        return Ok(());
    }

    let mut apply_fields = Vec::new();
    let mut conflict_fields = Vec::new();

    for field in EDITABLE_KEYS {
        if *field == "latestNote" {
            continue;
        }
        if *field == "appeal" && !csv_fields.appeal_in_file {
            continue;
        }
        if *field == "applicationEase" && !csv_fields.application_ease_in_file {
            continue;
        }
        if *field == "companyStability" && !csv_fields.company_stability_in_file {
            continue;
        }
        if *field == "employmentStability" && !csv_fields.employment_stability_in_file {
            continue;
        }
        if let Some(score_plan) = score_plans.and_then(|plans| plans.get(*field)) {
            if *score_plan == ScoreCellPlan::Apply {
                apply_fields.push((*field).to_string());
            }
            continue;
        }
        let csv_changed = field_norm(csv_fields, field) != field_norm(&baseline, field);
        let db_changed = field_norm(&db_fields, field) != field_norm(&baseline, field);
        match mode {
            ImportMode::OverwriteEditable => {
                if field_norm(csv_fields, field) != field_norm(&db_fields, field) {
                    apply_fields.push((*field).to_string());
                }
            }
            ImportMode::Merge => {
                if csv_changed && db_changed {
                    conflict_fields.push((*field).to_string());
                } else if csv_changed && !db_changed {
                    apply_fields.push((*field).to_string());
                }
            }
        }
    }

    if !conflict_fields.is_empty() {
        result.summary.conflicts += 1;
        result.conflicts.push(ImportConflict {
            key: key.to_string(),
            fields: conflict_fields.clone(),
            db: serde_json::to_value(&db_fields).unwrap_or_default(),
            csv: serde_json::to_value(csv_fields).unwrap_or_default(),
        });
    }

    let note_changed = match mode {
        ImportMode::OverwriteEditable => {
            !values_equal(
                csv_fields.latest_note.as_deref(),
                db_fields.latest_note.as_deref(),
            ) && csv_fields.latest_note.is_some()
        }
        ImportMode::Merge => {
            !values_equal(
                csv_fields.latest_note.as_deref(),
                baseline.latest_note.as_deref(),
            ) && csv_fields.latest_note.is_some()
                && values_equal(
                    db_fields.latest_note.as_deref(),
                    baseline.latest_note.as_deref(),
                )
        }
    };

    let note_conflict = matches!(mode, ImportMode::Merge)
        && !values_equal(
            csv_fields.latest_note.as_deref(),
            baseline.latest_note.as_deref(),
        )
        && !values_equal(
            db_fields.latest_note.as_deref(),
            baseline.latest_note.as_deref(),
        )
        && !values_equal(
            csv_fields.latest_note.as_deref(),
            db_fields.latest_note.as_deref(),
        );

    if note_conflict {
        result.summary.conflicts += 1;
        result.conflicts.push(ImportConflict {
            key: key.to_string(),
            fields: vec!["latestNote".into()],
            db: serde_json::to_value(&db_fields).unwrap_or_default(),
            csv: serde_json::to_value(csv_fields).unwrap_or_default(),
        });
    }

    if apply_fields.is_empty() && !note_changed {
        result.summary.unchanged += 1;
        return Ok(());
    }

    if !apply_fields.is_empty() {
        if !dry_run {
            let mut updates = UpdateJobInput::default();
            if apply_fields.iter().any(|f| f == "url") {
                updates.url = Some(csv_fields.url.clone());
            }
            if apply_fields.iter().any(|f| f == "title") {
                updates.title = Some(csv_fields.title.clone());
            }
            if apply_fields.iter().any(|f| f == "company") {
                updates.company_name = Some(csv_fields.company.clone());
            }
            if apply_fields.iter().any(|f| f == "status") {
                updates.status = Some(csv_fields.status.clone());
            }
            if apply_fields.iter().any(|f| f == "appliedAt") {
                updates.applied_at = Some(csv_fields.applied_at.clone());
            }
            if apply_fields.iter().any(|f| f == "notes") {
                updates.notes = Some(csv_fields.notes.clone());
            }
            if apply_fields.iter().any(|f| f == "location") {
                updates.location = Some(csv_fields.location.clone());
            }
            if apply_fields.iter().any(|f| f == "appeal") {
                updates.appeal = Some(csv_fields.appeal);
            }
            if apply_fields.iter().any(|f| f == "applicationEase") {
                updates.application_ease = Some(csv_fields.application_ease);
            }
            if apply_fields.iter().any(|f| f == "companyStability") {
                updates.company_stability = Some(csv_fields.company_stability);
            }
            if apply_fields.iter().any(|f| f == "employmentStability") {
                updates.employment_stability = Some(csv_fields.employment_stability);
            }
            update_job(conn, &existing.id, updates)?;
        }
        result.summary.updated += 1;
        result.changes.push(serde_json::json!({
            "action": "update",
            "jobId": existing.id,
            "fields": apply_fields
        }));
    }

    if note_changed {
        if let Some(note) = &csv_fields.latest_note {
            if !dry_run {
                add_job_event(conn, &existing.id, "csv_note", Some(note))?;
                imported_notes.insert(existing.id.clone(), Some(note.clone()));
            }
            result.summary.notes_added += 1;
            result.changes.push(serde_json::json!({
                "action": "note",
                "jobId": existing.id,
                "fields": ["latestNote"]
            }));
        }
    }

    Ok(())
}

pub fn sync_jobs_csv_with_disk(
    db_path: &Path,
    csv_path: &Path,
) -> AppResult<(Option<ImportResult>, ExportResult)> {
    let conn = open_csv_conn(db_path)?;
    let sync = read_sync_state(csv_path);
    let externally_changed = csv_path.exists()
        && (sync.last_export_at.is_none()
            || fs::metadata(csv_path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .map(|modified| {
                    let modified: chrono::DateTime<chrono::Utc> = modified.into();
                    sync.last_export_at
                        .as_ref()
                        .map(|exported| modified.to_rfc3339() > *exported)
                        .unwrap_or(true)
                })
                .unwrap_or(false));
    if externally_changed {
        drop(conn);
        let imported = import_jobs_csv(db_path, csv_path, None, false, ImportMode::Merge)?;
        let conn = open_csv_conn(db_path)?;
        let exported = ExportResult {
            path: csv_path.display().to_string(),
            exported_at: read_sync_state(csv_path)
                .last_export_at
                .unwrap_or_else(now_iso),
            row_count: load_job_rows(&conn)?.len(),
        };
        return Ok((Some(imported), exported));
    }
    let exported = export_jobs_csv(&conn, csv_path, None)?;
    Ok((None, exported))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;
    use crate::filtering::model::{
        FilterCriteria, LocationCriteria, MatchMode, RemoteMode, TitleCriteria,
    };
    use crate::filtering::resolver::set_global_criteria;
    use crate::jobs::service::{archive_job, create_job_from_url, delete_job, unarchive_job};
    use tempfile::tempdir;

    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        connection
    }

    #[test]
    fn parse_csv_quotes() {
        let rows = parse_csv("a,b\n\"x,\"\"y\"\",z\",1\n");
        assert_eq!(rows[1][0], "x,\"y\",z");
    }

    #[test]
    fn delete_job_and_export_removes_from_csv_and_sync_state() {
        let conn = test_connection();
        let dir = tempdir().unwrap();
        let csv_path = dir.path().join("jobs.csv");

        let (job1, _) = create_job_from_url(
            &conn,
            "https://example.com/job/1",
            "wishlist",
            Some("Acme"),
            Some("Job 1"),
            None,
            None,
            None,
        )
        .unwrap();

        let (job2, _) = create_job_from_url(
            &conn,
            "https://example.com/job/2",
            "applied",
            Some("Acme"),
            Some("Job 2"),
            None,
            None,
            None,
        )
        .unwrap();

        // First export: should contain both jobs
        let exp1 = export_jobs_csv(&conn, &csv_path, None).unwrap();
        assert_eq!(exp1.row_count, 2);

        let parsed = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        assert_eq!(parsed.len(), 3); // header + 2 rows

        let sync1 = read_sync_state(&csv_path);
        assert_eq!(sync1.rows.len(), 2);
        assert!(sync1.rows.contains_key(&job1.id));
        assert!(sync1.rows.contains_key(&job2.id));

        // Delete job1
        delete_job(&conn, &job1.id).unwrap();

        // Second export: should only contain job2
        let exp2 = export_jobs_csv(&conn, &csv_path, None).unwrap();
        assert_eq!(exp2.row_count, 1);

        let parsed2 = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        assert_eq!(parsed2.len(), 2); // header + 1 row
        assert_eq!(parsed2[1][0], job2.id);

        let sync2 = read_sync_state(&csv_path);
        assert_eq!(sync2.rows.len(), 1);
        assert!(!sync2.rows.contains_key(&job1.id));
        assert!(sync2.rows.contains_key(&job2.id));
    }

    #[test]
    fn archive_and_unarchive_job_updates_csv_export() {
        let conn = test_connection();
        let dir = tempdir().unwrap();
        let csv_path = dir.path().join("jobs.csv");

        let (job, _) = create_job_from_url(
            &conn,
            "https://example.com/job/archivable",
            "wishlist",
            Some("Startup"),
            Some("DevOps Engineer"),
            None,
            None,
            None,
        )
        .unwrap();

        // Archive job
        archive_job(&conn, &job.id).unwrap();
        export_jobs_csv(&conn, &csv_path, None).unwrap();

        let parsed = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        assert_eq!(parsed[1][6], "archived");

        // Unarchive job
        unarchive_job(&conn, &job.id, Some("applied")).unwrap();
        export_jobs_csv(&conn, &csv_path, None).unwrap();

        let parsed_restored = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        assert_eq!(parsed_restored[1][6], "applied");
    }

    #[test]
    fn export_keeps_closed_jobs_in_database_and_csv() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("job-tracker.db");
        let csv_path = dir.path().join("jobs.csv");
        let conn = Connection::open(&db_path).unwrap();
        migrate(&conn).unwrap();
        let (job, _) = create_job_from_url(
            &conn,
            "https://example.com/job/legacy-closed",
            "Legacy closed role",
            Some("Acme"),
            Some("applied"),
            None,
            None,
            None,
        )
        .unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'closed' WHERE id = ?1",
            params![job.id],
        )
        .unwrap();

        let exported = export_jobs_csv(&conn, &csv_path, None).unwrap();
        assert_eq!(exported.row_count, 1);
        assert_eq!(
            get_job_by_id(&conn, &job.id).unwrap().unwrap().status,
            "closed"
        );
        let parsed = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1][6], "closed");
        drop(conn);

        // Round-trip: re-importing the exported CSV keeps the closed row.
        let imported =
            import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert!(imported.errors.is_empty());
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(
            get_job_by_id(&conn, &job.id).unwrap().unwrap().status,
            "closed"
        );
        export_jobs_csv(&conn, &csv_path, None).unwrap();
        let parsed = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1][6], "closed");
    }

    #[test]
    fn import_closed_status_row_keeps_job() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("job-tracker.db");
        let csv_path = dir.path().join("jobs.csv");
        let job_id = {
            let conn = Connection::open(&db_path).unwrap();
            migrate(&conn).unwrap();
            let (job, _) = create_job_from_url(
                &conn,
                "https://example.com/job/import-closed",
                "Import closed role",
                Some("Acme"),
                Some("applied"),
                None,
                None,
                None,
            )
            .unwrap();
            export_jobs_csv(&conn, &csv_path, None).unwrap();
            job.id
        };
        let mut rows = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        rows[1][6] = "closed".to_string();
        fs::write(&csv_path, serialize_csv(&rows)).unwrap();

        let imported =
            import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert!(imported.errors.is_empty());
        assert_eq!(imported.summary.updated, 1);
        let conn = Connection::open(&db_path).unwrap();
        let stored = get_job_by_id(&conn, &job_id)
            .unwrap()
            .expect("closed job kept");
        assert_eq!(stored.status, "closed");
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM job_events WHERE job_id = ?1",
                params![job_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(events >= 2);
    }

    /// Read the set of job ids from an exported CSV file (excluding the header).
    fn exported_ids(csv_path: &Path) -> HashSet<String> {
        let parsed = parse_csv(&fs::read_to_string(csv_path).unwrap());
        // Row 0 is the header; column 0 is the job id.
        parsed
            .into_iter()
            .skip(1)
            .map(|row| row[0].clone())
            .collect()
    }

    /// Regression: CSV export row selection is INDEPENDENT of filter criteria.
    ///
    /// A restrictive global `FilterCriteria` that would hide these jobs from a
    /// watch listing must not change which tracked pipeline jobs are exported.
    /// The exported row set (job ids and count) must be identical to the
    /// match-all run, and tracked jobs that a watch listing would filter out
    /// must still be present in the CSV.
    ///
    /// **Validates: Requirements 18.2, 18.3, 18.4**
    #[test]
    fn csv_export_unaffected_by_restrictive_filter_criteria() {
        let conn = test_connection();
        let dir = tempdir().unwrap();
        let csv_path = dir.path().join("jobs.csv");

        // Several TRACKED pipeline jobs: create_job_from_url sets
        // is_new_from_watch = 0 and watch_disposition = NULL, so they qualify
        // for CSV export regardless of any filter criteria.
        let (job1, _) = create_job_from_url(
            &conn,
            "https://example.com/job/pipeline-1",
            "Software Engineer",
            Some("Acme"),
            Some("applied"),
            None,
            None,
            Some("San Francisco, CA"),
        )
        .unwrap();
        let (job2, _) = create_job_from_url(
            &conn,
            "https://example.com/job/pipeline-2",
            "Product Manager",
            Some("Globex"),
            Some("wishlist"),
            None,
            None,
            Some("New York, NY"),
        )
        .unwrap();
        let (job3, _) = create_job_from_url(
            &conn,
            "https://example.com/job/pipeline-3",
            "Data Scientist",
            Some("Initech"),
            Some("interview"),
            None,
            None,
            Some("Remote"),
        )
        .unwrap();

        let expected_ids: HashSet<String> = [job1.id.clone(), job2.id.clone(), job3.id.clone()]
            .into_iter()
            .collect();

        // Baseline: export under match-all / no global criteria.
        set_global_criteria(&conn, &FilterCriteria::match_all()).unwrap();
        let baseline_export = export_jobs_csv(&conn, &csv_path, None).unwrap();
        let baseline_ids = exported_ids(&csv_path);
        assert_eq!(baseline_export.row_count, 3);
        assert_eq!(baseline_ids, expected_ids);

        // Now set a RESTRICTIVE global filter that would hide every one of
        // these jobs from a watch listing: a title include no job matches, plus
        // a non-matching location include, plus a remote-only gate.
        let restrictive = FilterCriteria {
            version: 1,
            title: TitleCriteria {
                include: vec!["zzz-nonmatching-title".to_string()],
                exclude: Vec::new(),
                match_mode: MatchMode::Word,
            },
            location: LocationCriteria {
                country: Some("Antarctica".to_string()),
                include: vec!["zzz-nonmatching-location".to_string()],
                exclude: Vec::new(),
                match_mode: MatchMode::Word,
            },
            remote: RemoteMode::RemoteOnly,
        };
        set_global_criteria(&conn, &restrictive).unwrap();

        // Export again with the restrictive criteria in place.
        let filtered_export = export_jobs_csv(&conn, &csv_path, None).unwrap();
        let filtered_ids = exported_ids(&csv_path);

        // Req 18.2 / 18.3: the exported row set is IDENTICAL to the match-all
        // run — CSV export is unaffected by filter criteria.
        assert_eq!(filtered_export.row_count, baseline_export.row_count);
        assert_eq!(filtered_ids, baseline_ids);
        assert_eq!(filtered_ids, expected_ids);

        // Req 18.4: jobs that a watch listing would filter out (e.g. the
        // onsite-only San Francisco role under a RemoteOnly gate) are STILL
        // present in the CSV export because they are tracked pipeline jobs.
        assert!(filtered_ids.contains(&job1.id));
        assert!(filtered_ids.contains(&job2.id));
        assert!(filtered_ids.contains(&job3.id));

        // Also confirm the underlying selection is stable directly.
        let rows = load_job_rows(&conn).unwrap();
        let load_ids: HashSet<String> = rows.into_iter().map(|(job, _)| job.id).collect();
        assert_eq!(load_ids, expected_ids);
    }

    #[test]
    fn appeal_csv_round_trip_preserves_blank_and_favorite() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("job-tracker.db");
        let csv_path = dir.path().join("jobs.csv");
        let job_id = {
            let conn = Connection::open(&db_path).unwrap();
            migrate(&conn).unwrap();
            let (job, _) = create_job_from_url(
                &conn,
                "https://example.com/job/appeal",
                "Appealing role",
                Some("Acme"),
                Some("wishlist"),
                None,
                None,
                None,
            )
            .unwrap();
            assert_eq!(job.appeal, None);
            crate::jobs::service::set_job_favorite(&conn, &job.id, true).unwrap();
            export_jobs_csv(&conn, &csv_path, None).unwrap();
            job.id
        };

        let old_sync = r#"{"url":"https://example.com/job/appeal","title":"Appealing role","company":"Acme","status":"wishlist","appliedAt":null,"notes":null,"location":null,"latestNote":null}"#;
        let legacy: EditableFields = serde_json::from_str(old_sync).unwrap();
        assert_eq!(legacy.appeal, None);
        assert!(!legacy.appeal_in_file);

        let parsed = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        assert_eq!(
            parsed[0].last().map(String::as_str),
            Some("employment_stability")
        );
        assert_eq!(parsed[0].len(), parsed[1].len());
        let appeal_idx = parsed[0]
            .iter()
            .position(|header| header == "appeal")
            .unwrap();
        assert_eq!(parsed[1][appeal_idx], "");

        let mut scored = parsed.clone();
        scored[1][appeal_idx] = "5".to_string();
        fs::write(&csv_path, serialize_csv(&scored)).unwrap();
        let imported =
            import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert_eq!(imported.summary.updated, 1);
        assert!(imported.errors.is_empty());

        let conn = Connection::open(&db_path).unwrap();
        let job = get_job_by_id(&conn, &job_id).unwrap().unwrap();
        assert_eq!(job.appeal, Some(5));
        assert!(job.is_favorite);
        drop(conn);

        let without_column = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        let mut stripped = without_column.clone();
        let drop_appeal = stripped[0]
            .iter()
            .position(|header| header == "appeal")
            .unwrap();
        for row in &mut stripped {
            row.remove(drop_appeal);
        }
        assert!(!stripped[0].iter().any(|header| header == "appeal"));
        fs::write(&csv_path, serialize_csv(&stripped)).unwrap();
        let kept = import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert_eq!(kept.summary.updated, 0);
        let conn = Connection::open(&db_path).unwrap();
        let job = get_job_by_id(&conn, &job_id).unwrap().unwrap();
        assert_eq!(job.appeal, Some(5));
        assert!(job.is_favorite);
        drop(conn);

        let mut cleared = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        let clear_idx = cleared[0]
            .iter()
            .position(|header| header == "appeal")
            .unwrap();
        cleared[1][clear_idx].clear();
        fs::write(&csv_path, serialize_csv(&cleared)).unwrap();
        import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(get_job_by_id(&conn, &job_id).unwrap().unwrap().appeal, None);
        drop(conn);

        let mut invalid = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        invalid[1][clear_idx] = "9".to_string();
        fs::write(&csv_path, serialize_csv(&invalid)).unwrap();
        let rejected =
            import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert_eq!(rejected.summary.skipped, 1);
        assert!(rejected.errors[0]["message"]
            .as_str()
            .unwrap()
            .contains("Invalid appeal"));
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(get_job_by_id(&conn, &job_id).unwrap().unwrap().appeal, None);

        let create_csv = "\
url,title,company,status,appeal
https://example.com/job/new-appeal,New role,Globex,wishlist,2
";
        let created = import_jobs_csv(
            &db_path,
            &csv_path,
            Some(create_csv),
            false,
            ImportMode::Merge,
        )
        .unwrap();
        assert_eq!(created.summary.created, 1);
        let jobs = load_job_rows(&conn).unwrap();
        let new_job = jobs
            .iter()
            .find(|(job, _)| job.url.contains("new-appeal"))
            .unwrap();
        assert_eq!(new_job.0.appeal, Some(2));
        assert_eq!(new_job.0.application_ease, None);
        assert!(!new_job.0.is_favorite);
    }

    #[test]
    fn application_ease_csv_round_trip_preserves_blank_and_other_scores() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("job-tracker.db");
        let csv_path = dir.path().join("jobs.csv");
        let job_id = {
            let conn = Connection::open(&db_path).unwrap();
            migrate(&conn).unwrap();
            let (job, _) = create_job_from_url(
                &conn,
                "https://example.com/job/easy",
                "Easy role",
                Some("Acme"),
                Some("wishlist"),
                None,
                None,
                None,
            )
            .unwrap();
            assert_eq!(job.application_ease, None);
            update_job(
                &conn,
                &job.id,
                UpdateJobInput {
                    appeal: Some(Some(4)),
                    ..UpdateJobInput::default()
                },
            )
            .unwrap();
            crate::jobs::service::set_job_favorite(&conn, &job.id, true).unwrap();
            export_jobs_csv(&conn, &csv_path, None).unwrap();
            job.id
        };

        let old_sync = r#"{"url":"https://example.com/job/easy","title":"Easy role","company":"Acme","status":"wishlist","appliedAt":null,"notes":null,"location":null,"latestNote":null,"appeal":4}"#;
        let legacy: EditableFields = serde_json::from_str(old_sync).unwrap();
        assert_eq!(legacy.application_ease, None);
        assert!(!legacy.application_ease_in_file);
        assert_eq!(legacy.appeal, Some(4));

        let parsed = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        let ease_idx = parsed[0]
            .iter()
            .position(|header| header == "application_ease")
            .unwrap();
        let appeal_idx = parsed[0]
            .iter()
            .position(|header| header == "appeal")
            .unwrap();
        assert_eq!(parsed[1][ease_idx], "");
        assert_eq!(parsed[1][appeal_idx], "4");

        let mut scored = parsed.clone();
        scored[1][ease_idx] = "5".to_string();
        fs::write(&csv_path, serialize_csv(&scored)).unwrap();
        let imported =
            import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert_eq!(imported.summary.updated, 1);
        assert!(imported.errors.is_empty());

        let conn = Connection::open(&db_path).unwrap();
        let job = get_job_by_id(&conn, &job_id).unwrap().unwrap();
        assert_eq!(job.application_ease, Some(5));
        assert_eq!(job.appeal, Some(4));
        assert!(job.is_favorite);
        drop(conn);

        let without_column = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        let mut stripped = without_column.clone();
        let drop_ease = stripped[0]
            .iter()
            .position(|header| header == "application_ease")
            .unwrap();
        for row in &mut stripped {
            row.remove(drop_ease);
        }
        assert!(!stripped[0]
            .iter()
            .any(|header| header == "application_ease"));
        fs::write(&csv_path, serialize_csv(&stripped)).unwrap();
        let kept = import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert_eq!(kept.summary.updated, 0);
        let conn = Connection::open(&db_path).unwrap();
        let job = get_job_by_id(&conn, &job_id).unwrap().unwrap();
        assert_eq!(job.application_ease, Some(5));
        assert_eq!(job.appeal, Some(4));
        assert!(job.is_favorite);
        drop(conn);

        let mut cleared = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        let clear_idx = cleared[0]
            .iter()
            .position(|header| header == "application_ease")
            .unwrap();
        cleared[1][clear_idx].clear();
        fs::write(&csv_path, serialize_csv(&cleared)).unwrap();
        import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        let cleared_job = get_job_by_id(&conn, &job_id).unwrap().unwrap();
        assert_eq!(cleared_job.application_ease, None);
        assert_eq!(cleared_job.appeal, Some(4));
        drop(conn);

        let mut invalid = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        invalid[1][clear_idx] = "9".to_string();
        fs::write(&csv_path, serialize_csv(&invalid)).unwrap();
        let rejected =
            import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert_eq!(rejected.summary.skipped, 1);
        assert!(rejected.errors[0]["message"]
            .as_str()
            .unwrap()
            .contains("Invalid application ease"));
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(
            get_job_by_id(&conn, &job_id)
                .unwrap()
                .unwrap()
                .application_ease,
            None
        );

        let create_csv = "\
url,title,company,status,application_ease
https://example.com/job/new-ease,New role,Globex,wishlist,2
";
        let created = import_jobs_csv(
            &db_path,
            &csv_path,
            Some(create_csv),
            false,
            ImportMode::Merge,
        )
        .unwrap();
        assert_eq!(created.summary.created, 1);
        let jobs = load_job_rows(&conn).unwrap();
        let new_job = jobs
            .iter()
            .find(|(job, _)| job.url.contains("new-ease"))
            .unwrap();
        assert_eq!(new_job.0.application_ease, Some(2));
        assert_eq!(new_job.0.appeal, None);
        assert!(!new_job.0.is_favorite);
    }

    /// Set the company-owned score through the same service used by the UI,
    /// CLI, and CSV mirror. Every job at the company receives the cache value.
    fn set_company_stability(conn: &Connection, id: &str, value: Option<i64>) {
        update_job(
            conn,
            id,
            UpdateJobInput {
                company_stability: Some(value),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();
    }

    fn header_idx(rows: &[Vec<String>], name: &str) -> usize {
        rows[0].iter().position(|h| h == name).unwrap()
    }

    fn cell(value: Option<i64>) -> String {
        value.map(|v| v.to_string()).unwrap_or_default()
    }

    #[test]
    fn csv_field_lists_and_editable_presence_flags_are_stable() {
        assert_eq!(
            CSV_HEADERS,
            &[
                "id",
                "company",
                "csv_rev",
                "url",
                "canonical_url",
                "title",
                "status",
                "applied_at",
                "notes",
                "location",
                "latest_note",
                "source",
                "posting_state",
                "updated_at",
                "appeal",
                "application_ease",
                "company_stability",
                "employment_stability",
            ]
        );
        assert_eq!(
            EDITABLE_KEYS,
            &[
                "url",
                "title",
                "company",
                "status",
                "appliedAt",
                "notes",
                "location",
                "latestNote",
                "appeal",
                "applicationEase",
                "companyStability",
                "employmentStability",
            ]
        );
        assert_eq!(
            CSV_HEADERS
                .windows(2)
                .find(|pair| pair == &["company_stability", "employment_stability"]),
            Some(["company_stability", "employment_stability"].as_slice())
        );
        assert_eq!(
            EDITABLE_KEYS.len(),
            EDITABLE_KEYS.iter().collect::<HashSet<_>>().len()
        );

        let fields = EditableFields {
            url: "https://example.com/job/field-list".into(),
            title: "Role".into(),
            company: "  Acme  ".into(),
            status: "wishlist".into(),
            applied_at: None,
            notes: Some("  notes  ".into()),
            location: None,
            latest_note: None,
            appeal: Some(3),
            appeal_in_file: true,
            application_ease: None,
            application_ease_in_file: false,
            company_stability: Some(4),
            company_stability_in_file: true,
            employment_stability: Some(2),
            employment_stability_in_file: false,
        };
        assert_eq!(field_norm(&fields, "company"), Some("Acme".into()));
        assert_eq!(field_norm(&fields, "notes"), Some("notes".into()));
        assert_eq!(field_norm(&fields, "companyStability"), Some("4".into()));
        assert_eq!(field_norm(&fields, "employmentStability"), Some("2".into()));
        assert_eq!(field_norm(&fields, "unknown"), None);

        let serialized = serde_json::to_value(&fields).unwrap();
        assert!(serialized.get("appealInFile").is_none());
        assert!(serialized.get("companyStabilityInFile").is_none());
        assert!(serialized.get("employmentStabilityInFile").is_none());
        let old_sidecar: EditableFields = serde_json::from_value(serde_json::json!({
            "url": fields.url,
            "title": fields.title,
            "company": fields.company,
            "status": fields.status,
            "appliedAt": null,
            "notes": null,
            "location": null,
            "latestNote": null,
            "appeal": 3,
            "applicationEase": 2
        }))
        .unwrap();
        assert_eq!(old_sidecar.company_stability, None);
        assert_eq!(old_sidecar.employment_stability, None);
        assert!(!old_sidecar.company_stability_in_file);
        assert!(!old_sidecar.employment_stability_in_file);
    }

    #[test]
    fn csv_export_repeats_company_scores_and_rebaselines_after_import() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("job-tracker.db");
        let csv_path = dir.path().join("jobs.csv");
        let (first_id, second_id, other_id) = {
            let conn = Connection::open(&db_path).unwrap();
            migrate(&conn).unwrap();
            let (first, _) = create_job_from_url(
                &conn,
                "https://example.com/job/repeated-1",
                "Repeated role 1",
                Some("Acme"),
                Some("wishlist"),
                None,
                None,
                None,
            )
            .unwrap();
            let (second, _) = create_job_from_url(
                &conn,
                "https://example.com/job/repeated-2",
                "Repeated role 2",
                Some("Acme"),
                Some("wishlist"),
                None,
                None,
                None,
            )
            .unwrap();
            let (other, _) = create_job_from_url(
                &conn,
                "https://example.com/job/repeated-3",
                "Other role",
                Some("Globex"),
                Some("wishlist"),
                None,
                None,
                None,
            )
            .unwrap();
            update_job(
                &conn,
                &first.id,
                UpdateJobInput {
                    company_stability: Some(Some(4)),
                    employment_stability: Some(Some(2)),
                    company_stability_rationale: Some(Some("Stable customer base".into())),
                    employment_stability_rationale: Some(Some("No recent cuts".into())),
                    company_stability_citations: Some(Some(vec![
                        "https://example.com/company".into()
                    ])),
                    employment_stability_citations: Some(Some(vec![
                        "https://example.com/employment".into(),
                    ])),
                    ..UpdateJobInput::default()
                },
            )
            .unwrap();
            export_jobs_csv(&conn, &csv_path, None).unwrap();
            (first.id, second.id, other.id)
        };

        let exported = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        assert_eq!(exported.len(), 4);
        assert_eq!(exported[0], CSV_HEADERS);
        assert!(exported.iter().all(|row| row.len() == CSV_HEADERS.len()));
        let company_idx = header_idx(&exported, "company");
        let company_score_idx = header_idx(&exported, "company_stability");
        let employment_score_idx = header_idx(&exported, "employment_stability");
        let acme_rows: Vec<&Vec<String>> = exported
            .iter()
            .skip(1)
            .filter(|row| row[company_idx] == "Acme")
            .collect();
        assert_eq!(acme_rows.len(), 2);
        assert!(acme_rows
            .iter()
            .all(|row| row[company_score_idx] == "4" && row[employment_score_idx] == "2"));
        let globex = exported.iter().find(|row| row[0] == other_id).unwrap();
        assert_eq!(globex[company_score_idx], "");
        assert_eq!(globex[employment_score_idx], "");

        let mut edited = exported;
        for row in edited.iter_mut().skip(1) {
            if row[0] == first_id || row[0] == second_id {
                row[company_score_idx] = "5".into();
                row[employment_score_idx] = "3".into();
            }
        }
        fs::write(&csv_path, serialize_csv(&edited)).unwrap();
        let imported =
            import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert!(imported.errors.is_empty(), "errors: {:?}", imported.errors);
        assert_eq!(imported.summary.conflicts, 0);

        let conn = Connection::open(&db_path).unwrap();
        for id in [&first_id, &second_id] {
            let job = get_job_by_id(&conn, id).unwrap().unwrap();
            assert_eq!(job.company_stability, Some(5));
            assert_eq!(job.employment_stability, Some(3));
        }
        let reexported = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        assert_eq!(reexported[0], CSV_HEADERS);
        assert!(reexported
            .iter()
            .skip(1)
            .all(|row| row.len() == CSV_HEADERS.len()));
        assert!(reexported
            .iter()
            .skip(1)
            .filter(|row| row[company_idx] == "Acme")
            .all(|row| row[company_score_idx] == "5" && row[employment_score_idx] == "3"));
        let rebased = read_sync_state(&csv_path);
        for id in [&first_id, &second_id] {
            let fields = &rebased.rows.get(id).unwrap().fields;
            assert_eq!(fields.company_stability, Some(5));
            assert_eq!(fields.employment_stability, Some(3));
        }
    }

    #[test]
    fn csv_old_sidecar_and_missing_employment_column_preserve_score() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("job-tracker.db");
        let csv_path = dir.path().join("jobs.csv");
        let job_id = {
            let conn = Connection::open(&db_path).unwrap();
            migrate(&conn).unwrap();
            let (job, _) = create_job_from_url(
                &conn,
                "https://example.com/job/legacy-employment",
                "Legacy employment role",
                Some("Acme"),
                Some("wishlist"),
                None,
                None,
                None,
            )
            .unwrap();
            update_job(
                &conn,
                &job.id,
                UpdateJobInput {
                    company_stability: Some(Some(4)),
                    employment_stability: Some(Some(2)),
                    ..UpdateJobInput::default()
                },
            )
            .unwrap();
            export_jobs_csv(&conn, &csv_path, None).unwrap();
            job.id
        };

        let sync_path = sync_path_for(&csv_path);
        let mut old_sidecar: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&sync_path).unwrap()).unwrap();
        for row in old_sidecar["rows"].as_object_mut().unwrap().values_mut() {
            let fields = row["fields"].as_object_mut().unwrap();
            fields.remove("companyStability");
            fields.remove("employmentStability");
        }
        fs::write(&sync_path, serde_json::to_string(&old_sidecar).unwrap()).unwrap();

        let mut legacy_csv = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        let employment_idx = header_idx(&legacy_csv, "employment_stability");
        for row in &mut legacy_csv {
            row.remove(employment_idx);
        }
        fs::write(&csv_path, serialize_csv(&legacy_csv)).unwrap();

        let imported =
            import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert!(imported.errors.is_empty(), "errors: {:?}", imported.errors);
        let conn = Connection::open(&db_path).unwrap();
        let job = get_job_by_id(&conn, &job_id).unwrap().unwrap();
        assert_eq!(job.company_stability, Some(4));
        assert_eq!(job.employment_stability, Some(2));

        let refreshed = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        assert_eq!(refreshed[0], CSV_HEADERS);
        assert_eq!(refreshed[1].len(), CSV_HEADERS.len());
        let refreshed_employment_idx = header_idx(&refreshed, "employment_stability");
        assert_eq!(refreshed[1][refreshed_employment_idx], "2");
        let rebased = read_sync_state(&csv_path);
        let fields = &rebased.rows.get(&job_id).unwrap().fields;
        assert_eq!(fields.company_stability, Some(4));
        assert_eq!(fields.employment_stability, Some(2));
    }

    mod merge_props {
        use super::*;
        use proptest::prelude::*;

        #[test]
        fn csv_merge_overwrite_and_lifecycle_preserve_company_and_job_state() {
            let dir = tempdir().unwrap();
            let db_path = dir.path().join("job-tracker.db");
            let csv_path = dir.path().join("jobs.csv");
            let (moved_id, kept_id, other_id) = {
                let conn = Connection::open(&db_path).unwrap();
                migrate(&conn).unwrap();
                let (moved, _) = create_job_from_url(
                    &conn,
                    "https://example.com/job/moved",
                    "Moved role",
                    Some("Acme"),
                    Some("wishlist"),
                    None,
                    Some("keep these notes"),
                    Some("Remote"),
                )
                .unwrap();
                let (kept, _) = create_job_from_url(
                    &conn,
                    "https://example.com/job/kept",
                    "Kept role",
                    Some("Acme"),
                    Some("wishlist"),
                    None,
                    None,
                    None,
                )
                .unwrap();
                let (other, _) = create_job_from_url(
                    &conn,
                    "https://example.com/job/other",
                    "Other role",
                    Some("Globex"),
                    Some("wishlist"),
                    None,
                    None,
                    None,
                )
                .unwrap();
                update_job(
                    &conn,
                    &moved.id,
                    UpdateJobInput {
                        company_stability: Some(Some(4)),
                        employment_stability: Some(Some(3)),
                        company_stability_rationale: Some(Some("cash flow".into())),
                        company_stability_citations: Some(Some(vec![
                            "https://example.com/stability".into(),
                        ])),
                        employment_stability_rationale: Some(Some("steady team".into())),
                        employment_stability_citations: Some(Some(vec![
                            "https://example.com/employment".into(),
                        ])),
                        appeal: Some(Some(2)),
                        application_ease: Some(Some(5)),
                        ..UpdateJobInput::default()
                    },
                )
                .unwrap();
                crate::jobs::service::set_job_favorite(&conn, &moved.id, true).unwrap();
                set_company_stability(&conn, &other.id, Some(1));
                export_jobs_csv(&conn, &csv_path, None).unwrap();
                (moved.id, kept.id, other.id)
            };

            let mut rows = parse_csv(&fs::read_to_string(&csv_path).unwrap());
            let id_idx = header_idx(&rows, "id");
            let company_idx = header_idx(&rows, "company");
            let company_score_idx = header_idx(&rows, "company_stability");
            let employment_score_idx = header_idx(&rows, "employment_stability");
            let title_idx = header_idx(&rows, "title");
            let moved_row = rows.iter().position(|row| row[id_idx] == moved_id).unwrap();
            let kept_row = rows.iter().position(|row| row[id_idx] == kept_id).unwrap();
            let other_row = rows.iter().position(|row| row[id_idx] == other_id).unwrap();
            rows[moved_row][company_idx] = "Moved Co".into();
            rows[moved_row][company_score_idx] = "2".into();
            rows[moved_row][employment_score_idx].clear();
            rows[kept_row][company_score_idx] = "5".into();
            rows[kept_row][employment_score_idx].clear();
            rows[other_row][company_score_idx] = "3.5".into();

            let mut new_row = vec![String::new(); CSV_HEADERS.len()];
            new_row[header_idx(&rows, "url")] = "https://example.com/job/new-csv".into();
            new_row[title_idx] = "New CSV role".into();
            new_row[company_idx] = "New Co".into();
            new_row[header_idx(&rows, "status")] = "wishlist".into();
            new_row[company_score_idx] = "3".into();
            new_row[employment_score_idx] = "4".into();
            rows.push(new_row);
            fs::write(&csv_path, serialize_csv(&rows)).unwrap();

            let result =
                import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
            assert_eq!(result.summary.created, 1);
            assert_eq!(result.summary.skipped, 1);
            assert!(result.errors[0]["message"]
                .as_str()
                .unwrap()
                .contains("3.5"));
            assert!(
                result.conflicts.is_empty(),
                "unexpected conflicts: {:?}",
                result.conflicts
            );

            let conn = Connection::open(&db_path).unwrap();
            let moved = get_job_by_id(&conn, &moved_id).unwrap().unwrap();
            let kept = get_job_by_id(&conn, &kept_id).unwrap().unwrap();
            let other = get_job_by_id(&conn, &other_id).unwrap().unwrap();
            assert_eq!(
                company_name_for_job(&conn, &moved.company_id).unwrap(),
                "Moved Co"
            );
            assert_eq!(moved.company_stability, Some(2));
            assert_eq!(moved.employment_stability, None);
            assert_eq!(moved.appeal, Some(2));
            assert_eq!(moved.application_ease, Some(5));
            assert!(moved.is_favorite);
            assert_eq!(moved.notes.as_deref(), Some("keep these notes"));
            assert_eq!(
                company_name_for_job(&conn, &kept.company_id).unwrap(),
                "Acme"
            );
            assert_eq!(kept.company_stability, Some(5));
            assert_eq!(kept.employment_stability, None);
            assert_eq!(other.company_stability, Some(1));

            let new_job = conn
            .query_row(
                &format!("SELECT {} FROM jobs j INNER JOIN companies c ON j.company_id = c.id WHERE j.url = ?1", job_cols(Some("j"))),
                params!["https://example.com/job/new-csv"],
                map_job,
            )
            .unwrap();
            assert_eq!(
                company_name_for_job(&conn, &new_job.company_id).unwrap(),
                "New Co"
            );
            assert_eq!(new_job.company_stability, Some(3));
            assert_eq!(new_job.employment_stability, Some(4));

            let acme_id: String = conn
                .query_row("SELECT id FROM companies WHERE name = 'Acme'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let acme_metadata: (Option<String>, Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT company_stability_rationale, company_stability_citations, employment_stability_rationale, employment_stability_citations FROM companies WHERE id = ?1",
                params![acme_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
            assert_eq!(acme_metadata.0.as_deref(), Some("cash flow"));
            assert_eq!(
                acme_metadata.1.as_deref(),
                Some("[\"https://example.com/stability\"]")
            );
            assert_eq!(acme_metadata.2, None);
            assert_eq!(acme_metadata.3, None);

            let moved_company_id: String = conn
                .query_row(
                    "SELECT company_id FROM jobs WHERE id = ?1",
                    params![moved_id],
                    |row| row.get(0),
                )
                .unwrap();
            let moved_cache: Option<i64> = conn
                .query_row(
                    "SELECT company_stability FROM jobs WHERE id = ?1",
                    params![moved_id],
                    |row| row.get(0),
                )
                .unwrap();
            let moved_company_score: Option<i64> = conn
                .query_row(
                    "SELECT company_stability FROM companies WHERE id = ?1",
                    params![moved_company_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(moved_cache, moved_company_score);

            let exported = parse_csv(&fs::read_to_string(&csv_path).unwrap());
            assert_eq!(exported[0], CSV_HEADERS);
            assert_eq!(
                exported.iter().find(|row| row[id_idx] == kept_id).unwrap()[company_score_idx],
                "5"
            );
            assert_eq!(
                read_sync_state(&csv_path)
                    .rows
                    .get(&kept_id)
                    .unwrap()
                    .fields
                    .company_stability,
                Some(5)
            );
        }

        #[test]
        fn csv_overwrite_and_presence_tracking_keep_database_wins_and_missing_scores() {
            let dir = tempdir().unwrap();
            let db_path = dir.path().join("job-tracker.db");
            let csv_path = dir.path().join("jobs.csv");
            let (first_id, second_id) = {
                let conn = Connection::open(&db_path).unwrap();
                migrate(&conn).unwrap();
                let (first, _) = create_job_from_url(
                    &conn,
                    "https://example.com/job/overwrite-1",
                    "Role 1",
                    Some("Acme"),
                    Some("wishlist"),
                    None,
                    None,
                    None,
                )
                .unwrap();
                let (second, _) = create_job_from_url(
                    &conn,
                    "https://example.com/job/overwrite-2",
                    "Role 2",
                    Some("Acme"),
                    Some("wishlist"),
                    None,
                    None,
                    None,
                )
                .unwrap();
                update_job(
                    &conn,
                    &first.id,
                    UpdateJobInput {
                        company_stability: Some(Some(4)),
                        employment_stability: Some(Some(2)),
                        ..UpdateJobInput::default()
                    },
                )
                .unwrap();
                export_jobs_csv(&conn, &csv_path, None).unwrap();
                (first.id, second.id)
            };

            let mut rows = parse_csv(&fs::read_to_string(&csv_path).unwrap());
            let id_idx = header_idx(&rows, "id");
            let company_score_idx = header_idx(&rows, "company_stability");
            let employment_score_idx = header_idx(&rows, "employment_stability");
            for row in rows.iter_mut().skip(1) {
                if row[id_idx] == first_id || row[id_idx] == second_id {
                    row[company_score_idx] = "5".into();
                    row[employment_score_idx] = "3".into();
                }
            }
            fs::write(&csv_path, serialize_csv(&rows)).unwrap();
            let overwritten = import_jobs_csv(
                &db_path,
                &csv_path,
                None,
                false,
                ImportMode::OverwriteEditable,
            )
            .unwrap();
            assert_eq!(overwritten.summary.conflicts, 0);
            let conn = Connection::open(&db_path).unwrap();
            assert_eq!(
                get_job_by_id(&conn, &first_id)
                    .unwrap()
                    .unwrap()
                    .company_stability,
                Some(5)
            );
            assert_eq!(
                get_job_by_id(&conn, &second_id)
                    .unwrap()
                    .unwrap()
                    .employment_stability,
                Some(3)
            );
            drop(conn);

            // A missing score column is not a present blank cell. It must preserve
            // the stored company score while unrelated editable fields still merge.
            let mut without_company_score = parse_csv(&fs::read_to_string(&csv_path).unwrap());
            for row in &mut without_company_score {
                row.remove(company_score_idx);
            }
            let title_idx = header_idx(&without_company_score, "title");
            let first_row = without_company_score
                .iter()
                .position(|row| row[id_idx] == first_id)
                .unwrap();
            without_company_score[first_row][title_idx] = "Edited title".into();
            fs::write(&csv_path, serialize_csv(&without_company_score)).unwrap();
            let preserved =
                import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
            assert!(
                preserved.errors.is_empty(),
                "errors: {:?}",
                preserved.errors
            );
            let conn = Connection::open(&db_path).unwrap();
            let first = get_job_by_id(&conn, &first_id).unwrap().unwrap();
            assert_eq!(first.company_stability, Some(5));
            assert_eq!(first.title, "Edited title");
        }

        #[test]
        fn csv_score_disagreement_is_isolated_and_dry_run_writes_nothing() {
            let dir = tempdir().unwrap();
            let db_path = dir.path().join("job-tracker.db");
            let csv_path = dir.path().join("jobs.csv");
            let (first_id, second_id, other_id) = {
                let conn = Connection::open(&db_path).unwrap();
                migrate(&conn).unwrap();
                let (first, _) = create_job_from_url(
                    &conn,
                    "https://example.com/job/disagree-1",
                    "Role 1",
                    Some("Acme"),
                    Some("wishlist"),
                    None,
                    None,
                    None,
                )
                .unwrap();
                let (second, _) = create_job_from_url(
                    &conn,
                    "https://example.com/job/disagree-2",
                    "Role 2",
                    Some("Acme"),
                    Some("wishlist"),
                    None,
                    None,
                    None,
                )
                .unwrap();
                let (other, _) = create_job_from_url(
                    &conn,
                    "https://example.com/job/disagree-3",
                    "Role 3",
                    Some("Globex"),
                    Some("wishlist"),
                    None,
                    None,
                    None,
                )
                .unwrap();
                set_company_stability(&conn, &first.id, Some(4));
                export_jobs_csv(&conn, &csv_path, None).unwrap();
                (first.id, second.id, other.id)
            };
            let mut rows = parse_csv(&fs::read_to_string(&csv_path).unwrap());
            let id_idx = header_idx(&rows, "id");
            let score_idx = header_idx(&rows, "company_stability");
            let notes_idx = header_idx(&rows, "notes");
            let first_row = rows.iter().position(|row| row[id_idx] == first_id).unwrap();
            let second_row = rows
                .iter()
                .position(|row| row[id_idx] == second_id)
                .unwrap();
            let other_row = rows.iter().position(|row| row[id_idx] == other_id).unwrap();
            rows[first_row][score_idx] = "5".into();
            rows[first_row][notes_idx] = "CSV note survives disagreement".into();
            rows[second_row][score_idx].clear();
            rows[other_row][score_idx] = "3".into();
            fs::write(&csv_path, serialize_csv(&rows)).unwrap();
            let imported =
                import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
            assert_eq!(imported.summary.conflicts, 1);
            assert_eq!(imported.conflicts[0].fields, vec!["companyStability"]);
            let conn = Connection::open(&db_path).unwrap();
            assert_eq!(
                get_job_by_id(&conn, &first_id)
                    .unwrap()
                    .unwrap()
                    .company_stability,
                Some(4)
            );
            assert_eq!(
                get_job_by_id(&conn, &second_id)
                    .unwrap()
                    .unwrap()
                    .company_stability,
                Some(4)
            );
            assert_eq!(
                get_job_by_id(&conn, &first_id)
                    .unwrap()
                    .unwrap()
                    .notes
                    .as_deref(),
                Some("CSV note survives disagreement")
            );
            assert_eq!(
                get_job_by_id(&conn, &other_id)
                    .unwrap()
                    .unwrap()
                    .company_stability,
                Some(3)
            );
            drop(conn);

            let before_sync = fs::read_to_string(sync_path_for(&csv_path)).unwrap();
            let before_count = {
                let conn = Connection::open(&db_path).unwrap();
                conn.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get::<_, i64>(0))
                    .unwrap()
            };
            let mut dry_rows = parse_csv(&fs::read_to_string(&csv_path).unwrap());
            dry_rows[first_row][score_idx] = "2".into();
            let mut dry_new = vec![String::new(); CSV_HEADERS.len()];
            dry_new[header_idx(&dry_rows, "url")] = "https://example.com/job/dry-new".into();
            dry_new[header_idx(&dry_rows, "title")] = "Dry run role".into();
            dry_new[header_idx(&dry_rows, "company")] = "Dry Co".into();
            dry_new[header_idx(&dry_rows, "status")] = "wishlist".into();
            dry_new[score_idx] = "5".into();
            dry_rows.push(dry_new);
            let dry_content = serialize_csv(&dry_rows);
            let dry = import_jobs_csv(
                &db_path,
                &csv_path,
                Some(&dry_content),
                true,
                ImportMode::Merge,
            )
            .unwrap();
            assert!(dry.dry_run);
            assert!(!dry.changes.is_empty());
            assert_eq!(
                fs::read_to_string(sync_path_for(&csv_path)).unwrap(),
                before_sync
            );
            let conn = Connection::open(&db_path).unwrap();
            assert_eq!(
                conn.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                before_count
            );
            assert_eq!(
                get_job_by_id(&conn, &first_id)
                    .unwrap()
                    .unwrap()
                    .company_stability,
                Some(4)
            );
            assert!(get_job_by_id(&conn, "dry-new").unwrap().is_none());
        }

        #[test]
        fn csv_boundary_has_no_metadata_columns_and_citation_limit_precedes_url_validation() {
            assert!(!CSV_HEADERS.contains(&"company_stability_rationale"));
            assert!(!CSV_HEADERS.contains(&"company_stability_citations"));
            assert!(!EDITABLE_KEYS.contains(&"companyStabilityCitations"));

            let conn = test_connection();
            let (job, _) = create_job_from_url(
                &conn,
                "https://example.com/job/citation-limit",
                "Citation limit role",
                Some("Acme"),
                Some("wishlist"),
                None,
                None,
                None,
            )
            .unwrap();
            let mut citations = vec!["not-a-url".to_string()];
            citations.extend((0..20).map(|i| format!("https://example.com/{i}")));
            let error = update_job(
                &conn,
                &job.id,
                UpdateJobInput {
                    company_stability: Some(Some(3)),
                    company_stability_citations: Some(Some(citations)),
                    ..UpdateJobInput::default()
                },
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("20"),
                "unexpected error: {error}"
            );
            assert_eq!(
                get_job_by_id(&conn, &job.id)
                    .unwrap()
                    .unwrap()
                    .company_stability,
                None
            );
        }

        // Feature: company-employment-stability, Property 11: CSV three-way merge is baseline-correct
        // **Validates: Requirements 9.5, 9.6, 9.7, 9.8, 9.9, 9.10, 9.11, 9.12**
        proptest! {
            #![proptest_config(ProptestConfig::with_cases(100))]
            #[test]
            fn csv_three_way_merge_is_baseline_correct(
                baseline in prop::option::of(1i64..=5),
                csv in prop::option::of(1i64..=5),
                database in prop::option::of(1i64..=5),
            ) {
                let dir = tempdir().unwrap();
                let db_path = dir.path().join("job-tracker.db");
                let csv_path = dir.path().join("jobs.csv");
                let job_id = {
                    let conn = Connection::open(&db_path).unwrap();
                    migrate(&conn).unwrap();
                    let (job, _) = create_job_from_url(
                        &conn,
                        "https://example.com/job/merge-property",
                        "Merge role",
                        Some("Acme"),
                        Some("wishlist"),
                        None,
                        None,
                        None,
                    ).unwrap();
                    set_company_stability(&conn, &job.id, baseline);
                    export_jobs_csv(&conn, &csv_path, None).unwrap();
                    set_company_stability(&conn, &job.id, database);
                    job.id
                };

                let mut rows = parse_csv(&fs::read_to_string(&csv_path).unwrap());
                let stability_idx = header_idx(&rows, "company_stability");
                rows[1][stability_idx] = cell(csv);
                fs::write(&csv_path, serialize_csv(&rows)).unwrap();
                let result = import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
                prop_assert!(result.errors.is_empty(), "errors: {:?}", result.errors);

                let conn = Connection::open(&db_path).unwrap();
                let stored = get_job_by_id(&conn, &job_id).unwrap().unwrap().company_stability;
                let expected = if database != baseline { database } else { csv };
                prop_assert_eq!(stored, expected);
                let expected_conflict = csv != baseline && database != baseline;
                let company_id: String = conn.query_row(
                    "SELECT company_id FROM jobs WHERE id = ?1",
                    params![job_id],
                    |row| row.get(0),
                ).unwrap();
                let conflict_key = format!("company:{company_id}");
                prop_assert_eq!(
                    result.conflicts.iter().any(|conflict| conflict.key == conflict_key),
                    expected_conflict,
                    "csv conflict expectation failed"
                );
            }
        }
    }

    #[test]
    fn company_stability_csv_header_blank_missing_invalid_and_legacy() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("job-tracker.db");
        let csv_path = dir.path().join("jobs.csv");
        let (scored_id, other_id) = {
            let conn = Connection::open(&db_path).unwrap();
            migrate(&conn).unwrap();
            let mut ids = Vec::new();
            for (slug, company) in [("stable-a", "Acme"), ("stable-b", "Other Co")] {
                let (job, _) = create_job_from_url(
                    &conn,
                    &format!("https://example.com/job/{slug}"),
                    "Stable role",
                    Some(company),
                    Some("wishlist"),
                    None,
                    None,
                    None,
                )
                .unwrap();
                assert_eq!(job.company_stability, None);
                ids.push(job.id);
            }
            set_company_stability(&conn, &ids[0], Some(4));
            update_job(
                &conn,
                &ids[0],
                UpdateJobInput {
                    employment_stability: Some(Some(2)),
                    ..UpdateJobInput::default()
                },
            )
            .unwrap();
            update_job(
                &conn,
                &ids[0],
                UpdateJobInput {
                    appeal: Some(Some(3)),
                    application_ease: Some(Some(2)),
                    ..UpdateJobInput::default()
                },
            )
            .unwrap();
            crate::jobs::service::set_job_favorite(&conn, &ids[0], true).unwrap();
            export_jobs_csv(&conn, &csv_path, None).unwrap();
            (ids[0].clone(), ids[1].clone())
        };
        let stability = |id: &str| {
            let conn = Connection::open(&db_path).unwrap();
            get_job_by_id(&conn, id).unwrap().unwrap()
        };

        // Req 4.1: header order and equal cell counts.
        let rows = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        assert_eq!(rows[0], CSV_HEADERS);
        assert_eq!(
            header_idx(&rows, "employment_stability"),
            header_idx(&rows, "company_stability") + 1
        );
        assert_eq!(rows.len(), 3);
        for row in &rows {
            assert_eq!(row.len(), CSV_HEADERS.len());
        }
        let idx = header_idx(&rows, "company_stability");
        let employment_idx = header_idx(&rows, "employment_stability");
        let scored_row = rows.iter().position(|r| r[0] == scored_id).unwrap();
        assert_eq!(rows[scored_row][idx], "4");
        assert_eq!(rows[scored_row][employment_idx], "2");

        // Req 4.6: invalid row skipped with message; later rows still import.
        let mut invalid = rows.clone();
        invalid[scored_row][idx] = " 3.5 ".to_string();
        let other_row = invalid.iter().position(|r| r[0] == other_id).unwrap();
        invalid[other_row][idx] = "2".to_string();
        if other_row < scored_row {
            invalid.swap(other_row, scored_row);
        }
        fs::write(&csv_path, serialize_csv(&invalid)).unwrap();
        let rejected =
            import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert_eq!(rejected.summary.skipped, 1);
        assert_eq!(rejected.errors.len(), 1);
        assert!(rejected.errors[0]["message"]
            .as_str()
            .unwrap()
            .contains("Invalid company stability '3.5'"));
        assert_eq!(stability(&scored_id).company_stability, Some(4));
        assert_eq!(stability(&other_id).company_stability, Some(2));

        // Req 4.4: blank cell clears and leaves other scores and favorite alone.
        let mut cleared = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        let row = cleared.iter().position(|r| r[0] == scored_id).unwrap();
        cleared[row][idx] = "  ".to_string();
        fs::write(&csv_path, serialize_csv(&cleared)).unwrap();
        let result = import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert!(result.errors.is_empty());
        let job = stability(&scored_id);
        assert_eq!(job.company_stability, None);
        assert_eq!(job.appeal, Some(3));
        assert_eq!(job.application_ease, Some(2));
        assert!(job.is_favorite);

        // Req 4.5: a file without the column keeps stored scores.
        let mut stripped = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        for row in &mut stripped {
            row.remove(idx);
        }
        let row = stripped.iter().position(|r| r[0] == other_id).unwrap();
        let notes_idx = header_idx(&stripped, "notes");
        stripped[row][notes_idx] = "edited".to_string();
        fs::write(&csv_path, serialize_csv(&stripped)).unwrap();
        let kept = import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert!(kept.errors.is_empty());
        assert_eq!(kept.summary.updated, 1);
        assert_eq!(stability(&other_id).company_stability, Some(2));

        // Req 1.6, 4.4, 4.5: new rows with a blank cell or no column are unscored.
        let blank_csv = "\
url,title,company,status,company_stability
https://example.com/job/new-blank,New role,Blank Co,wishlist,
https://example.com/job/new-set,New role,Set Co,wishlist,5
";
        let created = import_jobs_csv(
            &db_path,
            &csv_path,
            Some(blank_csv),
            false,
            ImportMode::Merge,
        )
        .unwrap();
        assert_eq!(created.summary.created, 2);
        let no_column_csv = "\
url,title,company,status
https://example.com/job/new-no-column,New role,Globex,wishlist
";
        let created = import_jobs_csv(
            &db_path,
            &csv_path,
            Some(no_column_csv),
            false,
            ImportMode::Merge,
        )
        .unwrap();
        assert_eq!(created.summary.created, 1);
        let conn = Connection::open(&db_path).unwrap();
        let jobs = load_job_rows(&conn).unwrap();
        let find = |slug: &str| {
            jobs.iter()
                .find(|(job, _)| job.url.ends_with(slug))
                .unwrap()
                .0
                .company_stability
        };
        assert_eq!(find("new-blank"), None);
        assert_eq!(find("new-set"), Some(5));
        assert_eq!(find("new-no-column"), None);
        drop(conn);

        // Req 4.8: legacy sidecar JSON without companyStability loads.
        let legacy_fields = r#"{"url":"https://example.com/job/x","title":"X","company":"Acme","status":"wishlist","appliedAt":null,"notes":null,"location":null,"latestNote":null,"appeal":3,"applicationEase":2}"#;
        let legacy: EditableFields = serde_json::from_str(legacy_fields).unwrap();
        assert_eq!(legacy.company_stability, None);
        assert!(!legacy.company_stability_in_file);

        let sync_path = sync_path_for(&csv_path);
        let mut sidecar: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&sync_path).unwrap()).unwrap();
        for row in sidecar["rows"].as_object_mut().unwrap().values_mut() {
            row["fields"]
                .as_object_mut()
                .unwrap()
                .remove("companyStability");
        }
        fs::write(&sync_path, sidecar.to_string()).unwrap();
        let loaded = read_sync_state(&csv_path);
        assert_eq!(loaded.rows.len(), 5);
        assert!(loaded
            .rows
            .values()
            .all(|row| row.fields.company_stability.is_none()));

        let mut legacy_csv = parse_csv(&fs::read_to_string(&csv_path).unwrap());
        let idx = header_idx(&legacy_csv, "company_stability");
        for row in &mut legacy_csv {
            row.remove(idx);
        }
        fs::write(&csv_path, serialize_csv(&legacy_csv)).unwrap();
        let legacy_import =
            import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
        assert!(legacy_import.errors.is_empty());
        assert_eq!(legacy_import.summary.conflicts, 0);
        assert_eq!(stability(&other_id).company_stability, Some(2));
        assert_eq!(stability(&scored_id).company_stability, None);
    }

    mod round_trip_props {
        use super::*;
        use proptest::prelude::*;

        fn score() -> impl Strategy<Value = Option<i64>> {
            prop::option::of(1i64..=5)
        }

        // Feature: company-employment-stability, Property 10: CSV score cells and unedited files round-trip
        // **Validates: Requirements 9.1, 9.2, 9.3, 9.4, 9.13, 9.15, 9.16, 9.17**
        proptest! {
            #![proptest_config(ProptestConfig::with_cases(100))]
            #[test]
            fn csv_score_cells_and_unedited_files_round_trip(
                company_stability in score(),
                employment_stability in score(),
                jobs in prop::collection::vec((score(), score(), any::<bool>()), 1..=3)
            ) {
                prop_assert_eq!(
                    parse_company_stability_cell(&cell(company_stability)).unwrap(),
                    company_stability
                );
                prop_assert_eq!(
                    parse_employment_stability_cell(&cell(employment_stability)).unwrap(),
                    employment_stability
                );

                let dir = tempdir().unwrap();
                let db_path = dir.path().join("job-tracker.db");
                let csv_path = dir.path().join("jobs.csv");
                let mut expected = Vec::new();
                let mut first_id = None;
                {
                    let conn = Connection::open(&db_path).unwrap();
                    migrate(&conn).unwrap();
                    for (i, (appeal, ease, favorite)) in jobs.iter().enumerate() {
                        let (job, _) = create_job_from_url(
                            &conn,
                            &format!("https://example.com/job/rt-{i}"),
                            &format!("Role {i}"),
                            Some("Acme"),
                            Some("wishlist"),
                            None,
                            None,
                            None,
                        )
                        .unwrap();
                        update_job(
                            &conn,
                            &job.id,
                            UpdateJobInput {
                                appeal: Some(*appeal),
                                application_ease: Some(*ease),
                                ..UpdateJobInput::default()
                            },
                        )
                        .unwrap();
                        crate::jobs::service::set_job_favorite(&conn, &job.id, *favorite).unwrap();
                        if first_id.is_none() {
                            first_id = Some(job.id.clone());
                        }
                        expected.push((job.id, *appeal, *ease, *favorite));
                    }
                    update_job(
                        &conn,
                        first_id.as_ref().unwrap(),
                        UpdateJobInput {
                            company_stability: Some(company_stability),
                            employment_stability: Some(employment_stability),
                            ..UpdateJobInput::default()
                        },
                    )
                    .unwrap();
                    export_jobs_csv(&conn, &csv_path, None).unwrap();
                }

                let result =
                    import_jobs_csv(&db_path, &csv_path, None, false, ImportMode::Merge).unwrap();
                prop_assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
                prop_assert!(result.conflicts.is_empty());
                prop_assert_eq!(result.summary.conflicts, 0);

                let parsed = parse_csv(&fs::read_to_string(&csv_path).unwrap());
                prop_assert_eq!(&parsed[0], CSV_HEADERS);
                prop_assert!(parsed.iter().all(|row| row.len() == CSV_HEADERS.len()));
                let company_idx = header_idx(&parsed, "company_stability");
                let employment_idx = header_idx(&parsed, "employment_stability");
                for row in parsed.iter().skip(1) {
                    prop_assert_eq!(&row[company_idx], &cell(company_stability));
                    prop_assert_eq!(&row[employment_idx], &cell(employment_stability));
                }

                let conn = Connection::open(&db_path).unwrap();
                for (id, appeal, ease, favorite) in expected {
                    let job = get_job_by_id(&conn, &id).unwrap().unwrap();
                    prop_assert_eq!(job.company_stability, company_stability);
                    prop_assert_eq!(job.employment_stability, employment_stability);
                    prop_assert_eq!(job.appeal, appeal);
                    prop_assert_eq!(job.application_ease, ease);
                    prop_assert_eq!(job.is_favorite, favorite);
                }
            }
        }
    }
}
