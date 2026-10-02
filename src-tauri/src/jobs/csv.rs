use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::service::{
    add_job_event, create_job_from_url, delete_closed_jobs, delete_job, get_job_by_id, job_cols,
    map_job, update_job, UpdateJobInput, JOB_COL_COUNT,
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
    }
}

pub fn export_jobs_csv(
    conn: &Connection,
    csv_path: &Path,
    latest_note_overrides: Option<&HashMap<String, Option<String>>>,
) -> AppResult<ExportResult> {
    delete_closed_jobs(conn)?;
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
        last_import_at: sync.last_import_at,
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
                "SELECT {} FROM jobs WHERE canonical_url = ?1",
                job_cols(None)
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
];

fn parse_appeal_cell(raw: &str) -> Result<Option<i64>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed.parse::<i64>() {
        Ok(score) if (1..=5).contains(&score) => Ok(Some(score)),
        _ => Err(format!(
            "Invalid appeal '{trimmed}'. Use an integer from 1 to 5 (5 = most appealing), or leave blank"
        )),
    }
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
        };
        let id = normalize_nullable(Some(&get("id")));
        let key = id.clone().unwrap_or_else(|| {
            if csv_fields.url.is_empty() {
                format!("row-{row_number}")
            } else {
                csv_fields.url.clone()
            }
        });

        match process_row(
            &conn,
            &sync,
            &mut result,
            &mut seen_job_ids,
            &mut imported_notes,
            row_number,
            &key,
            id.as_deref(),
            &csv_fields,
            dry_run,
            &mode,
        ) {
            Ok(()) => {}
            Err(e) => {
                result.summary.skipped += 1;
                result.errors.push(serde_json::json!({
                    "row": row_number,
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
        let exported = export_jobs_csv(&conn, csv_path, Some(&imported_notes))?;
        let mut next_sync = read_sync_state(csv_path);
        next_sync.last_import_at = Some(now_iso());
        next_sync.last_export_at = Some(exported.exported_at);
        write_sync_state(&next_sync)?;
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
) -> AppResult<()> {
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
            if csv_fields.appeal.is_some() {
                update_job(
                    conn,
                    &created.0.id,
                    UpdateJobInput {
                        appeal: Some(csv_fields.appeal),
                        ..UpdateJobInput::default()
                    },
                )?;
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

    if csv_fields.status == "closed" {
        if !dry_run {
            delete_job(conn, &existing.id)?;
        }
        result.changes.push(serde_json::json!({
            "action": "delete",
            "jobId": existing.id,
            "reason": "closed"
        }));
        return Ok(());
    }
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
    let status = get_jobs_csv_status(&conn, csv_path)?;
    if status.exists
        && (status.last_export_at.is_none()
            || status
                .file_mtime
                .as_ref()
                .zip(status.last_export_at.as_ref())
                .map(|(m, e)| m > e)
                .unwrap_or(false))
    {
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
    fn export_removes_legacy_closed_jobs_from_database_and_csv() {
        let conn = test_connection();
        let dir = tempdir().unwrap();
        let csv_path = dir.path().join("jobs.csv");
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

        assert_eq!(exported.row_count, 0);
        assert!(get_job_by_id(&conn, &job.id).unwrap().is_none());
        assert_eq!(parse_csv(&fs::read_to_string(csv_path).unwrap()).len(), 1);
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
        assert_eq!(parsed[0].last().map(String::as_str), Some("appeal"));
        assert_eq!(parsed[0].len(), parsed[1].len());
        assert_eq!(parsed[1].last().map(String::as_str), Some(""));
        let appeal_idx = parsed[0].len() - 1;

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
        for row in &mut stripped {
            row.pop();
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
        assert!(!new_job.0.is_favorite);
    }
}
