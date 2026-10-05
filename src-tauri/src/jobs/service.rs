use std::collections::HashMap;

use chrono::{Datelike, Local, Timelike};
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::metadata::{resolve_job_metadata, JobMetadata};
use crate::models::{
    checked_appeal, is_job_status, AttachedDocument, Company, Document, Job, JobDetail,
    JobDocument, JobEvent, JobListItem, WeeklyActivity, WeeklyDay,
};
use crate::util::{create_id, guess_title_from_url, normalize_canonical_url, now_iso};

fn map_company(row: &rusqlite::Row<'_>) -> rusqlite::Result<Company> {
    Ok(Company {
        id: row.get(0)?,
        name: row.get(1)?,
        careers_url: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

/// Column order read by [`map_job`]. `appeal` is last so a company column
/// joined after this list starts at [`JOB_COL_COUNT`].
pub const JOB_COLUMNS: &[&str] = &[
    "id",
    "company_id",
    "title",
    "url",
    "canonical_url",
    "source_external_id",
    "status",
    "applied_at",
    "posting_state",
    "last_checked_at",
    "last_check_result",
    "source",
    "notes",
    "description",
    "location",
    "is_new_from_watch",
    "watch_disposition",
    "missing_from_sync_count",
    "is_favorite",
    "created_at",
    "updated_at",
    "appeal",
    "salary_min",
    "salary_max",
];

pub const JOB_COL_COUNT: usize = JOB_COLUMNS.len();

pub fn job_cols(alias: Option<&str>) -> String {
    JOB_COLUMNS
        .iter()
        .map(|column| match alias {
            Some(prefix) => format!("{prefix}.{column}"),
            None => (*column).to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: row.get(0)?,
        company_id: row.get(1)?,
        title: row.get(2)?,
        url: row.get(3)?,
        canonical_url: row.get(4)?,
        source_external_id: row.get(5)?,
        status: row.get(6)?,
        applied_at: row.get(7)?,
        posting_state: row.get(8)?,
        last_checked_at: row.get(9)?,
        last_check_result: row.get(10)?,
        source: row.get(11)?,
        notes: row.get(12)?,
        description: row.get(13)?,
        location: row.get(14)?,
        is_new_from_watch: row.get::<_, i64>(15)? != 0,
        watch_disposition: row.get(16)?,
        missing_from_sync_count: row.get(17)?,
        is_favorite: row.get::<_, i64>(18)? != 0,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
        appeal: row.get(21)?,
        salary_min: row.get(22)?,
        salary_max: row.get(23)?,
    })
}

/// Resolve a page title via network. Callers must not hold a DB mutex across this.
pub async fn resolve_title_from_url(url: &str, title: Option<&str>) -> String {
    if let Some(manual_title) = title.map(str::trim).filter(|t| !t.is_empty()) {
        return manual_title.to_string();
    }

    let metadata = resolve_job_metadata(url).await.ok();
    title_from_metadata(url, metadata.as_ref())
}

fn title_from_metadata(url: &str, metadata: Option<&JobMetadata>) -> String {
    metadata
        .and_then(|metadata| metadata.title.clone())
        .unwrap_or_else(|| guess_title_from_url(url))
}

pub fn create_job_from_url(
    conn: &Connection,
    url: &str,
    resolved_title: &str,
    company_name: Option<&str>,
    status: Option<&str>,
    applied_at: Option<&str>,
    notes: Option<&str>,
    location: Option<&str>,
) -> AppResult<(Job, Company)> {
    create_job_from_url_with_careers(
        conn,
        url,
        resolved_title,
        company_name,
        status,
        applied_at,
        notes,
        None,
        location,
        None,
    )
}

pub fn create_job_from_url_with_careers(
    conn: &Connection,
    url: &str,
    resolved_title: &str,
    company_name: Option<&str>,
    status: Option<&str>,
    applied_at: Option<&str>,
    notes: Option<&str>,
    description: Option<&str>,
    location: Option<&str>,
    careers_url: Option<&str>,
) -> AppResult<(Job, Company)> {
    if status == Some("closed") {
        return Err(AppError::from(
            "Closed jobs are deleted and cannot be added to the pipeline",
        ));
    }
    let canonical_url = normalize_canonical_url(url).map_err(AppError::from)?;
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM jobs WHERE canonical_url = ?1",
            params![canonical_url],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    if existing.is_some() {
        return Err(AppError::from("A job with this URL is already tracked"));
    }

    let company_name = company_name
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Unknown company");
    let timestamp = now_iso();

    let company = find_or_create_company(conn, company_name, careers_url)?;
    let status = status.unwrap_or("wishlist");
    let job_id = create_id();
    let applied = if let Some(a) = applied_at {
        Some(a.to_string())
    } else if status == "applied" {
        Some(timestamp.clone())
    } else {
        None
    };

    conn.execute(
        r#"INSERT INTO jobs (
            id, company_id, title, url, canonical_url, source_external_id, status, applied_at,
            posting_state, last_checked_at, last_check_result, source, notes, description, location,
            is_new_from_watch, watch_disposition, missing_from_sync_count, is_favorite, created_at, updated_at
        ) VALUES (?1,?2,?3,?4,?5,NULL,?6,?7,'unknown',NULL,NULL,'manual',?8,?9,?10,0,NULL,0,0,?11,?11)"#,
        params![
            job_id,
            company.id,
            resolved_title,
            url,
            canonical_url,
            status,
            applied,
            notes,
            description,
            location,
            timestamp
        ],
    )
    .map_err(map_sqlite)?;

    conn.execute(
        "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1,?2,'created',?3,?4)",
        params![
            create_id(),
            job_id,
            format!("Added from URL with status {status}"),
            timestamp
        ],
    )
    .map_err(map_sqlite)?;

    let job = get_job_by_id(conn, &job_id)?.ok_or_else(|| AppError::from("Job insert failed"))?;
    Ok((job, company))
}

pub fn find_or_create_company(
    conn: &Connection,
    name: &str,
    careers_url: Option<&str>,
) -> AppResult<Company> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::from("Company name cannot be empty"));
    }
    let timestamp = now_iso();
    if let Some(mut existing) = conn
        .query_row(
            "SELECT id, name, careers_url, created_at, updated_at FROM companies WHERE name = ?1 COLLATE NOCASE",
            params![name],
            map_company,
        )
        .optional()
        .map_err(map_sqlite)?
    {
        if let Some(url) = careers_url {
            conn.execute(
                "UPDATE companies SET careers_url = ?1, updated_at = ?2 WHERE id = ?3",
                params![url, timestamp, existing.id],
            )
            .map_err(map_sqlite)?;
            existing.careers_url = Some(url.to_string());
            existing.updated_at = timestamp;
        }
        return Ok(existing);
    }

    let company = Company {
        id: create_id(),
        name: name.to_string(),
        careers_url: careers_url.map(|s| s.to_string()),
        created_at: timestamp.clone(),
        updated_at: timestamp,
    };
    conn.execute(
        "INSERT INTO companies (id, name, careers_url, created_at, updated_at) VALUES (?1,?2,?3,?4,?5)",
        params![
            company.id,
            company.name,
            company.careers_url,
            company.created_at,
            company.updated_at
        ],
    )
    .map_err(map_sqlite)?;
    Ok(company)
}

/// Delete a company that became an untracked orphan after a posting was moved or deleted.
/// Companies with a board watch or careers-monitoring history are intentionally retained.
fn delete_untracked_empty_company(conn: &Connection, company_id: &str) -> AppResult<()> {
    conn.execute(
        r#"DELETE FROM companies
           WHERE id = ?1
             AND NOT EXISTS (SELECT 1 FROM jobs WHERE company_id = ?1)
             AND NOT EXISTS (SELECT 1 FROM company_watches WHERE company_id = ?1)
             AND NOT EXISTS (SELECT 1 FROM careers_page_snapshots WHERE company_id = ?1)
             AND NOT EXISTS (SELECT 1 FROM careers_page_reviews WHERE company_id = ?1)"#,
        params![company_id],
    )
    .map_err(map_sqlite)?;
    Ok(())
}

pub fn get_job_by_id(conn: &Connection, job_id: &str) -> AppResult<Option<Job>> {
    conn.query_row(
        &format!("SELECT {} FROM jobs WHERE id = ?1", job_cols(None)),
        params![job_id],
        map_job,
    )
    .optional()
    .map_err(map_sqlite)
}

#[derive(Default)]
pub struct JobFilters {
    pub status: Option<String>,
    pub company_id: Option<String>,
    pub posting_state: Option<String>,
    pub search: Option<String>,
    pub salary_min: Option<i64>,
    pub salary_max: Option<i64>,
    pub location: Option<String>,
    pub new_from_watch: Option<bool>,
    pub is_favorite: Option<bool>,
    pub is_archived: Option<bool>,
    /// Optional row cap applied as `LIMIT ?` — used to cap watch-preview fetches.
    pub limit: Option<usize>,
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct LocationSettings {
    pub country: String,
    pub cities: String,
}

pub fn list_jobs(conn: &Connection, filters: JobFilters) -> AppResult<Vec<JobListItem>> {
    let mut sql = format!(
        "SELECT {}, c.name
         FROM jobs j INNER JOIN companies c ON j.company_id = c.id WHERE 1=1",
        job_cols(Some("j"))
    );
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(status) = &filters.status {
        sql.push_str(" AND j.status = ?");
        values.push(Box::new(status.clone()));
    }
    if let Some(company_id) = &filters.company_id {
        sql.push_str(" AND j.company_id = ?");
        values.push(Box::new(company_id.clone()));
    }
    if let Some(posting_state) = &filters.posting_state {
        sql.push_str(" AND j.posting_state = ?");
        values.push(Box::new(posting_state.clone()));
    }
    if filters.is_favorite == Some(true) {
        sql.push_str(" AND j.is_favorite = 1");
    }
    if filters.is_archived == Some(true) {
        sql.push_str(" AND j.status IN ('archived', 'rejected', 'withdrawn', 'closed')");
    } else if filters.is_archived == Some(false) {
        sql.push_str(" AND j.status NOT IN ('archived', 'rejected', 'withdrawn', 'closed')");
    }
    if filters.new_from_watch == Some(true) {
        sql.push_str(" AND j.is_new_from_watch = 1");
    } else {
        // Pending and dismissed watch discoveries stay out of the pipeline.
        sql.push_str(
            " AND j.is_new_from_watch = 0 \
             AND (j.watch_disposition IS NULL OR j.watch_disposition != 'dismissed')",
        );
    }
    if let Some(search) = &filters.search {
        sql.push_str(
            " AND (j.title LIKE ? OR c.name LIKE ? OR j.url LIKE ? OR j.notes LIKE ? OR j.description LIKE ?)",
        );
        let pattern = format!("%{search}%");
        values.push(Box::new(pattern.clone()));
        values.push(Box::new(pattern.clone()));
        values.push(Box::new(pattern.clone()));
        values.push(Box::new(pattern.clone()));
        values.push(Box::new(pattern));
    }

    if filters.salary_min.is_some() || filters.salary_max.is_some() {
        // Treat a single known bound as an open-ended posting range; omit jobs
        // with no salary data entirely from salary-filtered results.
        sql.push_str(" AND (j.salary_min IS NOT NULL OR j.salary_max IS NOT NULL)");
        if let Some(minimum) = filters.salary_min {
            sql.push_str(" AND COALESCE(j.salary_max, 9223372036854775807) >= ?");
            values.push(Box::new(minimum));
        }
        if let Some(maximum) = filters.salary_max {
            sql.push_str(" AND COALESCE(j.salary_min, 0) <= ?");
            values.push(Box::new(maximum));
        }
    }

    // Location filtering: the watch path evaluates the structured filter engine
    // in memory (below) rather than building hardcoded location/keyword SQL, so
    // only the coarse `is_new_from_watch = 1` predicate applies here. The legacy
    // `filters.location` LIKE branch remains for the non-watch (pipeline) path.
    let watch_path = filters.new_from_watch == Some(true);
    if !watch_path {
        if let Some(location) = &filters.location {
            sql.push_str(" AND j.location LIKE ?");
            values.push(Box::new(format!("%{location}%")));
        }
    }

    sql.push_str(" ORDER BY j.updated_at DESC");

    // LIMIT ordering (Req 8.3): on the watch path the fine filter runs in Rust
    // *after* fetch, so the SQL LIMIT is applied post-filter (truncation below),
    // not pushed into the query. A generous fetch cap bounds the coarse load so
    // we never pull an unbounded number of candidate rows. On the non-watch path
    // the existing SQL LIMIT behavior is preserved.
    const WATCH_FETCH_CAP: i64 = 5000;
    if watch_path {
        sql.push_str(" LIMIT ?");
        values.push(Box::new(WATCH_FETCH_CAP));
    } else if let Some(lim) = filters.limit {
        sql.push_str(" LIMIT ?");
        values.push(Box::new(lim as i64));
    }

    let mut stmt = conn.prepare(&sql).map_err(map_sqlite)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
    let rows = stmt
        .query_map(params_ref.as_slice(), |row| {
            Ok(JobListItem {
                job: map_job(row)?,
                company_name: row.get(JOB_COL_COUNT)?,
            })
        })
        .map_err(map_sqlite)?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(map_sqlite)?);
    }

    // Query-time watch filtering (Req 8.1, 8.2, 8.3, 8.4): evaluate the resolved
    // structured criteria against each candidate in memory, keep only included
    // rows, then apply the caller's limit *after* filtering. This is purely a
    // read-only filter of the returned list — no job is deleted or mutated
    // (Req 18). Criteria are resolved once per (company_id, source) group via a
    // per-call cache and re-read on every call, so changing criteria re-filters
    // existing rows on the next listing without any re-sync (Req 8.4).
    if watch_path {
        use crate::filtering::engine::{matches, JobView};
        use crate::filtering::model::FilterCriteria;
        use crate::filtering::resolver::{load_alias_table, CriteriaCache};

        let aliases = load_alias_table(conn)?;
        let mut cache = CriteriaCache::new();

        out.retain(|item| {
            let criteria = cache
                .resolve(conn, &item.job.company_id, &item.job.source)
                .unwrap_or_else(|_| FilterCriteria::match_all());
            matches(
                &criteria,
                &aliases,
                JobView {
                    title: &item.job.title,
                    location: item.job.location.as_deref(),
                },
            )
            .included
        });

        if let Some(lim) = filters.limit {
            out.truncate(lim);
        }
    }

    Ok(out)
}

/// The latest known open snapshot from a company's connected ATS boards.
/// This intentionally includes roles the user previously dismissed: declining
/// a role is a personal triage choice, not evidence that the company closed it.
pub fn list_open_watch_positions(
    conn: &Connection,
    company_id: &str,
) -> AppResult<Vec<JobListItem>> {
    // Coarse SQL pre-filter only (company + active + supported providers). The
    // fine-grained location/keyword matching that used to be built here from
    // `expand_country_keywords`/`expand_location_keywords` is now evaluated in
    // memory by the single filter engine (Req 10.1, 10.2), so every watch
    // listing agrees on inclusion.
    let sql = format!(
        "SELECT {}, c.name
         FROM jobs j INNER JOIN companies c ON j.company_id = c.id
         WHERE j.company_id = ?1
           AND j.posting_state = 'active'
           AND j.source IN ('greenhouse', 'lever', 'ashby')
         ORDER BY j.updated_at DESC",
        job_cols(Some("j"))
    );

    let mut stmt = conn.prepare(&sql).map_err(map_sqlite)?;
    let rows = stmt
        .query_map(params![company_id], |row| {
            Ok(JobListItem {
                job: map_job(row)?,
                company_name: row.get(JOB_COL_COUNT)?,
            })
        })
        .map_err(map_sqlite)?;

    let mut out = rows.collect::<Result<Vec<_>, _>>().map_err(map_sqlite)?;

    // Query-time watch filtering through the single engine authority (Req 8.1,
    // 8.2, 10.1, 10.2): resolve the structured criteria per (company_id, source)
    // via the per-call cache and keep only rows the engine includes. This is a
    // read-only filter of the returned list — no job is mutated or deleted
    // (Req 18). There is no explicit row cap here, so no post-filter truncation
    // is required (had one existed, it would be applied after this retain per
    // Req 8.3).
    use crate::filtering::engine::{matches, JobView};
    use crate::filtering::model::FilterCriteria;
    use crate::filtering::resolver::{load_alias_table, CriteriaCache};

    let aliases = load_alias_table(conn)?;
    let mut cache = CriteriaCache::new();

    out.retain(|item| {
        let criteria = cache
            .resolve(conn, &item.job.company_id, &item.job.source)
            .unwrap_or_else(|_| FilterCriteria::match_all());
        matches(
            &criteria,
            &aliases,
            JobView {
                title: &item.job.title,
                location: item.job.location.as_deref(),
            },
        )
        .included
    });

    Ok(out)
}

pub fn get_job_detail(conn: &Connection, job_id: &str) -> AppResult<Option<JobDetail>> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {}, c.id, c.name, c.careers_url, c.created_at, c.updated_at
             FROM jobs j INNER JOIN companies c ON j.company_id = c.id WHERE j.id = ?1",
            job_cols(Some("j"))
        ))
        .map_err(map_sqlite)?;

    let detail = stmt
        .query_row(params![job_id], |row| {
            let company_at = JOB_COL_COUNT;
            Ok((
                map_job(row)?,
                Company {
                    id: row.get(company_at)?,
                    name: row.get(company_at + 1)?,
                    careers_url: row.get(company_at + 2)?,
                    created_at: row.get(company_at + 3)?,
                    updated_at: row.get(company_at + 4)?,
                },
            ))
        })
        .optional()
        .map_err(map_sqlite)?;

    let Some((job, company)) = detail else {
        return Ok(None);
    };

    let mut events_stmt = conn
        .prepare(
            "SELECT id, job_id, type, note, occurred_at FROM job_events WHERE job_id = ?1 ORDER BY occurred_at DESC",
        )
        .map_err(map_sqlite)?;
    let events = events_stmt
        .query_map(params![job_id], |row| {
            Ok(JobEvent {
                id: row.get(0)?,
                job_id: row.get(1)?,
                event_type: row.get(2)?,
                note: row.get(3)?,
                occurred_at: row.get(4)?,
            })
        })
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    let mut att_stmt = conn
        .prepare(
            "SELECT jd.id, jd.job_id, jd.document_id, jd.kind, jd.used_at,
                    d.id, d.original_filename, d.stored_filename, d.mime_type, d.checksum, d.size_bytes, d.imported_at
             FROM job_documents jd INNER JOIN documents d ON jd.document_id = d.id
             WHERE jd.job_id = ?1 ORDER BY jd.used_at DESC",
        )
        .map_err(map_sqlite)?;
    let attached = att_stmt
        .query_map(params![job_id], |row| {
            Ok(AttachedDocument {
                attachment: JobDocument {
                    id: row.get(0)?,
                    job_id: row.get(1)?,
                    document_id: row.get(2)?,
                    kind: row.get(3)?,
                    used_at: row.get(4)?,
                },
                document: Document {
                    id: row.get(5)?,
                    original_filename: row.get(6)?,
                    stored_filename: row.get(7)?,
                    mime_type: row.get(8)?,
                    checksum: row.get(9)?,
                    size_bytes: row.get(10)?,
                    imported_at: row.get(11)?,
                },
            })
        })
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    Ok(Some(JobDetail {
        job,
        company,
        events,
        attached,
    }))
}

/// `null` becomes `Some(None)` (an explicit clear). A missing field stays
/// `None` via `#[serde(default)]` and does not change the stored value.
fn deserialize_optional_update<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Some(serde::Deserialize::deserialize(deserializer)?))
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateJobInput {
    pub title: Option<String>,
    pub company_name: Option<String>,
    pub status: Option<String>,
    pub applied_at: Option<Option<String>>,
    pub notes: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub location: Option<Option<String>>,
    pub url: Option<String>,
    pub is_new_from_watch: Option<bool>,
    pub is_favorite: Option<bool>,
    /// `None` leaves salary unchanged; `Some(None)` clears a bound.
    #[serde(default, deserialize_with = "deserialize_optional_update")]
    pub salary_min: Option<Option<i64>>,
    #[serde(default, deserialize_with = "deserialize_optional_update")]
    pub salary_max: Option<Option<i64>>,
    /// `None` leaves the score unchanged. `Some(None)` clears it.
    /// `Some(Some(n))` sets it when `n` is 1–5.
    #[serde(default, deserialize_with = "deserialize_optional_update")]
    pub appeal: Option<Option<i64>>,
}

use serde::Deserialize;

pub fn update_job(
    conn: &Connection,
    job_id: &str,
    updates: UpdateJobInput,
) -> AppResult<JobDetail> {
    let existing = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
    let timestamp = now_iso();
    let mut company_id = existing.company_id.clone();
    let mut company_change: Option<(String, String)> = None;
    let mut next_url = existing.url.clone();
    let mut next_canonical = existing.canonical_url.clone();

    if let Some(url) = &updates.url {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err(AppError::from("URL cannot be empty"));
        }
        next_url = trimmed.to_string();
        next_canonical = normalize_canonical_url(trimmed).map_err(AppError::from)?;
        let dup: Option<String> = conn
            .query_row(
                "SELECT id FROM jobs WHERE canonical_url = ?1",
                params![next_canonical],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sqlite)?;
        if let Some(id) = dup {
            if id != job_id {
                return Err(AppError::from("A job with this URL is already tracked"));
            }
        }
    }

    if let Some(name) = &updates.company_name {
        let company = find_or_create_company(conn, name.trim(), None)?;
        if company.id != existing.company_id {
            let old_name: String = conn
                .query_row(
                    "SELECT name FROM companies WHERE id = ?1",
                    params![existing.company_id],
                    |row| row.get(0),
                )
                .map_err(map_sqlite)?;
            company_change = Some((old_name, company.name.clone()));
        }
        company_id = company.id;
    }

    let next_status = updates
        .status
        .clone()
        .unwrap_or_else(|| existing.status.clone());

    if next_status == "closed" {
        let mut detail =
            get_job_detail(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
        detail.job.status = "closed".to_string();
        delete_job(conn, job_id)?;
        return Ok(detail);
    }
    let next_applied = if let Some(applied) = &updates.applied_at {
        applied.clone()
    } else if next_status == "applied" && existing.applied_at.is_none() {
        Some(timestamp.clone())
    } else {
        existing.applied_at.clone()
    };

    let title = updates.title.as_deref().unwrap_or(&existing.title);
    let notes = updates
        .notes
        .clone()
        .unwrap_or_else(|| existing.notes.clone());
    let description = updates
        .description
        .clone()
        .unwrap_or_else(|| existing.description.clone());
    let location = updates
        .location
        .clone()
        .unwrap_or_else(|| existing.location.clone());
    let is_new = updates
        .is_new_from_watch
        .unwrap_or(existing.is_new_from_watch);
    let is_fav = updates.is_favorite.unwrap_or(existing.is_favorite);
    let salary_min = updates.salary_min.clone().unwrap_or(existing.salary_min);
    let salary_max = updates.salary_max.clone().unwrap_or(existing.salary_max);
    if salary_min.is_some_and(|value| value < 0)
        || salary_max.is_some_and(|value| value < 0)
        || matches!((salary_min, salary_max), (Some(minimum), Some(maximum)) if minimum > maximum)
    {
        return Err(AppError::from(
            "Salary must use non-negative USD amounts with minimum no greater than maximum",
        ));
    }
    let appeal = match updates.appeal {
        Some(next) => {
            if let Some(score) = next {
                checked_appeal(score).map_err(AppError::from)?;
            }
            next
        }
        None => existing.appeal,
    };

    conn.execute(
        r#"UPDATE jobs SET title=?1, company_id=?2, url=?3, canonical_url=?4, status=?5,
           applied_at=?6, notes=?7, description=?8, location=?9, is_new_from_watch=?10, is_favorite=?11, appeal=?12, salary_min=?13, salary_max=?14, updated_at=?15
           WHERE id=?16"#,
        params![
            title,
            company_id,
            next_url,
            next_canonical,
            next_status,
            next_applied,
            notes,
            description,
            location,
            if is_new { 1 } else { 0 },
            if is_fav { 1 } else { 0 },
            appeal,
            salary_min,
            salary_max,
            timestamp,
            job_id
        ],
    )
    .map_err(map_sqlite)?;

    if let Some(status) = &updates.status {
        if *status != existing.status {
            conn.execute(
                "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1,?2,'status_changed',?3,?4)",
                params![
                    create_id(),
                    job_id,
                    format!("Status changed from {} to {status}", existing.status),
                    timestamp
                ],
            )
            .map_err(map_sqlite)?;
        }
    }

    if let Some(fav) = updates.is_favorite {
        if fav != existing.is_favorite {
            let event_type = if fav { "favorited" } else { "unfavorited" };
            let note = if fav {
                "Marked as favorite"
            } else {
                "Removed from favorites"
            };
            conn.execute(
                "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1,?2,?3,?4,?5)",
                params![
                    create_id(),
                    job_id,
                    event_type,
                    note,
                    timestamp
                ],
            )
            .map_err(map_sqlite)?;
        }
    }

    if let Some((old_name, new_name)) = company_change {
        conn.execute(
            "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1,?2,'company_changed',?3,?4)",
            params![
                create_id(),
                job_id,
                format!("Company changed from {old_name} to {new_name}"),
                timestamp
            ],
        )
        .map_err(map_sqlite)?;
        delete_untracked_empty_company(conn, &existing.company_id)?;
    }

    get_job_detail(conn, job_id)?.ok_or_else(|| AppError::from("Job not found after update"))
}

/// Permanently delete a job and its associated events and attachments from SQLite.
pub fn delete_job(conn: &Connection, job_id: &str) -> AppResult<()> {
    let existing = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;

    conn.execute("DELETE FROM job_events WHERE job_id = ?1", params![job_id])
        .map_err(map_sqlite)?;

    conn.execute(
        "DELETE FROM job_documents WHERE job_id = ?1",
        params![job_id],
    )
    .map_err(map_sqlite)?;

    // Posting-check evidence belongs to the job and goes with it. `run_postings`
    // rows are intentionally kept as frozen run history (no FK to `jobs`).
    conn.execute(
        "DELETE FROM posting_check_evidence WHERE job_id = ?1",
        params![job_id],
    )
    .map_err(map_sqlite)?;

    conn.execute("DELETE FROM jobs WHERE id = ?1", params![job_id])
        .map_err(map_sqlite)?;

    delete_untracked_empty_company(conn, &existing.company_id)?;

    Ok(())
}

/// Permanently remove legacy closed jobs so they cannot reappear in the app or CSV.
pub fn delete_closed_jobs(conn: &Connection) -> AppResult<usize> {
    let ids = conn
        .prepare("SELECT id FROM jobs WHERE status = 'closed'")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    let count = ids.len();
    for id in ids {
        delete_job(conn, &id)?;
    }
    Ok(count)
}

/// Move a job out of the active pipeline into the archived status.
pub fn archive_job(conn: &Connection, job_id: &str) -> AppResult<JobDetail> {
    let existing = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
    if existing.status == "archived" {
        return get_job_detail(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"));
    }
    update_job(
        conn,
        job_id,
        UpdateJobInput {
            status: Some("archived".to_string()),
            ..Default::default()
        },
    )
}

/// Restore an archived job back into the active pipeline.
pub fn unarchive_job(
    conn: &Connection,
    job_id: &str,
    target_status: Option<&str>,
) -> AppResult<JobDetail> {
    let existing = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
    let next_status = target_status
        .filter(|s| is_job_status(s) && *s != "archived")
        .unwrap_or_else(|| {
            if existing.applied_at.is_some() {
                "applied"
            } else {
                "wishlist"
            }
        });
    update_job(
        conn,
        job_id,
        UpdateJobInput {
            status: Some(next_status.to_string()),
            ..Default::default()
        },
    )
}

/// Move a pending watch discovery onto the wishlist pipeline.
pub fn approve_watch_job(conn: &Connection, job_id: &str) -> AppResult<Job> {
    let existing = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
    if !existing.is_new_from_watch {
        return Ok(existing);
    }
    let timestamp = now_iso();
    conn.execute(
        "UPDATE jobs SET is_new_from_watch = 0, watch_disposition = 'saved', updated_at = ?1 WHERE id = ?2",
        params![timestamp, job_id],
    )
    .map_err(map_sqlite)?;
    add_job_event(conn, job_id, "approved_from_watch", None)?;
    get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found after approve"))
}

/// Dismiss a pending watch discovery so sync will not recreate it.
pub fn dismiss_watch_job(conn: &Connection, job_id: &str) -> AppResult<Job> {
    let existing = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
    if !existing.is_new_from_watch {
        return Ok(existing);
    }
    let timestamp = now_iso();
    conn.execute(
        "UPDATE jobs SET is_new_from_watch = 0, watch_disposition = 'dismissed', updated_at = ?1 WHERE id = ?2",
        params![timestamp, job_id],
    )
    .map_err(map_sqlite)?;
    add_job_event(conn, job_id, "dismissed_from_watch", None)?;
    get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found after dismiss"))
}

/// Save an open board role to the user's Jobs pipeline, including one that was
/// previously marked "Not for me".
pub fn save_open_watch_job(conn: &Connection, job_id: &str) -> AppResult<Job> {
    let existing = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
    if !matches!(existing.source.as_str(), "greenhouse" | "lever" | "ashby") {
        return Err(AppError::from("This job did not come from a watched board"));
    }
    if existing.posting_state != "active" {
        return Err(AppError::from("This position is no longer open"));
    }
    if existing.watch_disposition.as_deref() == Some("saved") && !existing.is_new_from_watch {
        return Ok(existing);
    }

    let timestamp = now_iso();
    conn.execute(
        "UPDATE jobs SET is_new_from_watch = 0, watch_disposition = 'saved', status = CASE WHEN status = 'closed' THEN 'wishlist' ELSE status END, updated_at = ?1 WHERE id = ?2",
        params![timestamp, job_id],
    )
    .map_err(map_sqlite)?;
    add_job_event(conn, job_id, "saved_from_open_board", None)?;
    get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found after save"))
}

/// Return a role closed by the legacy bulk-dismiss workflow to the New roles
/// inbox. Jobs already in the user's wishlist or application pipeline retain
/// their own state and are intentionally not resettable.
pub fn reset_dismissed_watch_job(conn: &Connection, job_id: &str) -> AppResult<Job> {
    let existing = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
    if existing.watch_disposition.as_deref() != Some("dismissed") {
        return Err(AppError::from("This role is not marked Not for me"));
    }
    if existing.status != "closed" {
        return Err(AppError::from(
            "Only closed watch roles can be reset; wishlist and applied jobs stay unchanged",
        ));
    }

    let timestamp = now_iso();
    conn.execute(
        "UPDATE jobs SET is_new_from_watch = 1, watch_disposition = 'new', status = 'wishlist', updated_at = ?1 WHERE id = ?2",
        params![timestamp, job_id],
    )
    .map_err(map_sqlite)?;
    add_job_event(conn, job_id, "reset_watch_dismissal", None)?;
    get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found after reset"))
}

pub fn add_job_event(
    conn: &Connection,
    job_id: &str,
    event_type: &str,
    note: Option<&str>,
) -> AppResult<JobEvent> {
    if get_job_by_id(conn, job_id)?.is_none() {
        return Err(AppError::from("Job not found"));
    }
    let event = JobEvent {
        id: create_id(),
        job_id: job_id.to_string(),
        event_type: event_type.to_string(),
        note: note.map(|s| s.to_string()),
        occurred_at: now_iso(),
    };
    conn.execute(
        "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1,?2,?3,?4,?5)",
        params![
            event.id,
            event.job_id,
            event.event_type,
            event.note,
            event.occurred_at
        ],
    )
    .map_err(map_sqlite)?;
    Ok(event)
}

pub fn get_pipeline_counts(conn: &Connection) -> AppResult<HashMap<String, i64>> {
    let mut counts: HashMap<String, i64> = [
        ("all", 0),
        ("favorites", 0),
        ("wishlist", 0),
        ("applied", 0),
        ("interviewing", 0),
        ("offer", 0),
        ("rejected", 0),
        ("withdrawn", 0),
        ("closed", 0),
        ("archived", 0),
        ("archivedTotal", 0),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();

    let mut stmt = conn
        .prepare(
            "SELECT status, COUNT(*) FROM jobs j \
             WHERE j.is_new_from_watch = 0 \
               AND (j.watch_disposition IS NULL OR j.watch_disposition != 'dismissed') \
             GROUP BY status",
        )
        .map_err(map_sqlite)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(map_sqlite)?;

    let mut total = 0i64;
    for row in rows {
        let (status, count) = row.map_err(map_sqlite)?;
        total += count;
        counts.insert(status, count);
    }
    counts.insert("all".into(), total);

    let favorites_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM jobs j \
             WHERE j.is_favorite = 1 \
               AND j.is_new_from_watch = 0 \
               AND (j.watch_disposition IS NULL OR j.watch_disposition != 'dismissed')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    counts.insert("favorites".into(), favorites_count);

    let archived_total = counts.get("archived").copied().unwrap_or(0)
        + counts.get("rejected").copied().unwrap_or(0)
        + counts.get("withdrawn").copied().unwrap_or(0)
        + counts.get("closed").copied().unwrap_or(0);
    counts.insert("archivedTotal".into(), archived_total);

    Ok(counts)
}

pub fn toggle_job_favorite(conn: &Connection, job_id: &str) -> AppResult<JobListItem> {
    let existing = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
    let new_fav = !existing.is_favorite;
    set_job_favorite(conn, job_id, new_fav)
}

pub fn set_job_favorite(
    conn: &Connection,
    job_id: &str,
    is_favorite: bool,
) -> AppResult<JobListItem> {
    let existing = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
    if existing.is_favorite == is_favorite {
        let company_name: String = conn
            .query_row(
                "SELECT name FROM companies WHERE id = ?1",
                params![existing.company_id],
                |r| r.get(0),
            )
            .map_err(map_sqlite)?;
        return Ok(JobListItem {
            job: existing,
            company_name,
        });
    }

    let timestamp = now_iso();
    conn.execute(
        "UPDATE jobs SET is_favorite = ?1, updated_at = ?2 WHERE id = ?3",
        params![if is_favorite { 1 } else { 0 }, timestamp, job_id],
    )
    .map_err(map_sqlite)?;

    let event_type = if is_favorite {
        "favorited"
    } else {
        "unfavorited"
    };
    let note = if is_favorite {
        "Marked as favorite"
    } else {
        "Removed from favorites"
    };
    add_job_event(conn, job_id, event_type, Some(note))?;

    let company_name: String = conn
        .query_row(
            "SELECT name FROM companies WHERE id = ?1",
            params![existing.company_id],
            |r| r.get(0),
        )
        .map_err(map_sqlite)?;

    let updated_job = get_job_by_id(conn, job_id)?
        .ok_or_else(|| AppError::from("Job not found after favorite"))?;
    Ok(JobListItem {
        job: updated_job,
        company_name,
    })
}

pub fn get_weekly_activity(conn: &Connection) -> AppResult<WeeklyActivity> {
    let now = Local::now();
    let start = (now - chrono::Duration::days(6))
        .with_hour(0)
        .and_then(|d| d.with_minute(0))
        .and_then(|d| d.with_second(0))
        .and_then(|d| d.with_nanosecond(0))
        .unwrap_or(now);

    let weekday_initials = ["S", "M", "T", "W", "T", "F", "S"];
    let today_key = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());

    let mut days = Vec::new();
    let mut buckets: HashMap<String, i64> = HashMap::new();
    for i in 0..7 {
        let date = start + chrono::Duration::days(i);
        let key = format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day());
        buckets.insert(key.clone(), 0);
        days.push(WeeklyDay {
            key: key.clone(),
            label: weekday_initials[date.weekday().num_days_from_sunday() as usize].into(),
            count: 0,
            is_today: key == today_key,
        });
    }

    let start_iso = start.with_timezone(&chrono::Utc).to_rfc3339();
    let mut stmt = conn
        .prepare(
            "SELECT occurred_at FROM job_events \
             WHERE occurred_at >= ?1 \
               AND type NOT IN ('discovered_from_watch', 'dismissed_from_watch', 'approved_from_watch')",
        )
        .map_err(map_sqlite)?;
    let events = stmt
        .query_map(params![start_iso], |row| row.get::<_, String>(0))
        .map_err(map_sqlite)?;

    let mut total = 0i64;
    for event in events {
        let occurred = event.map_err(map_sqlite)?;
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&occurred) {
            let local = dt.with_timezone(&Local);
            let key = format!(
                "{:04}-{:02}-{:02}",
                local.year(),
                local.month(),
                local.day()
            );
            if let Some(count) = buckets.get_mut(&key) {
                *count += 1;
                total += 1;
            }
        }
    }

    for day in &mut days {
        day.count = *buckets.get(&day.key).unwrap_or(&0);
    }

    Ok(WeeklyActivity { total, days })
}

pub fn get_watch_role_keywords(conn: &Connection) -> AppResult<String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'watch_role_keywords'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    Ok(value.unwrap_or_default())
}

pub fn set_watch_role_keywords(conn: &Connection, keywords: &str) -> AppResult<()> {
    let timestamp = now_iso();
    // Retain the legacy key so legacy readers and rollback still work (Req 15.1).
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES ('watch_role_keywords', ?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![keywords, timestamp],
    )
    .map_err(map_sqlite)?;

    // Write through to the structured global criteria so both surfaces stay
    // consistent (Req 15.2). Mirrors the legacy->structured mapping used by the
    // migration (`build_legacy_criteria`): keywords split on comma and newline,
    // trimmed and de-blanked, become `title.include` with Word match mode,
    // preserving the criteria's existing location/remote fields. If this
    // write-through fails, propagate the error so the legacy setter fails rather
    // than leaving the two surfaces inconsistent (Req 15.3).
    let mut criteria = crate::filtering::resolver::get_global_criteria(conn)?;
    criteria.title.include =
        crate::filtering::model::normalize_tokens(keywords.split([',', '\n']).collect::<Vec<_>>());
    criteria.title.match_mode = crate::filtering::model::MatchMode::Word;
    crate::filtering::resolver::set_global_criteria(conn, &criteria)?;

    Ok(())
}

pub fn get_location_settings(conn: &Connection) -> AppResult<LocationSettings> {
    let country: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'location_country'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    let cities: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'location_cities'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    Ok(LocationSettings {
        country: country.unwrap_or_default(),
        cities: cities.unwrap_or_default(),
    })
}

pub fn set_location_settings(conn: &Connection, settings: &LocationSettings) -> AppResult<()> {
    let timestamp = now_iso();
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES ('location_country', ?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![settings.country, timestamp],
    )
    .map_err(map_sqlite)?;
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES ('location_cities', ?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![settings.cities, timestamp],
    )
    .map_err(map_sqlite)?;

    // Write through to the structured global criteria so both surfaces stay
    // consistent (Req 15.2). Mirrors the legacy->structured mapping used by the
    // migration (`build_legacy_criteria`): a non-empty country becomes
    // `location.country` (else None), and cities split on comma, trimmed and
    // de-blanked, become `location.include`; title/remote fields are preserved.
    // Propagate any write-through failure so the legacy setter fails rather than
    // leaving the two surfaces inconsistent (Req 15.3).
    let mut criteria = crate::filtering::resolver::get_global_criteria(conn)?;
    let country_trimmed = settings.country.trim();
    criteria.location.country = if country_trimmed.is_empty() {
        None
    } else {
        Some(country_trimmed.to_string())
    };
    criteria.location.include =
        crate::filtering::model::normalize_tokens(settings.cities.split(',').collect::<Vec<_>>());
    crate::filtering::resolver::set_global_criteria(conn, &criteria)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::db::migrate::migrate;
    use crate::jobs::metadata::extract_job_metadata;

    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        connection
    }

    /// Manual performance gate for the live-search query at the planned 5,000-job scale.
    /// Run with `cargo test list_jobs_search_performance_5000_long_descriptions -- --ignored --nocapture`.
    #[test]
    #[ignore = "manual search latency benchmark"]
    fn list_jobs_search_performance_5000_long_descriptions() {
        let conn = test_connection();
        let timestamp = "2026-01-01T00:00:00Z";
        conn.execute(
            "INSERT INTO companies (id, name, created_at, updated_at) VALUES ('bench-company', 'Benchmark Co', ?1, ?1)",
            [timestamp],
        )
        .unwrap();
        let description = format!("{}", "Synthetic engineering role with detailed responsibilities and qualifications. ".repeat(110));
        let transaction = conn.unchecked_transaction().unwrap();
        for index in 0..5_000 {
            let title = if index == 4_999 {
                "Software Engineer, Sandboxing".to_string()
            } else {
                format!("Software Engineer {index}")
            };
            let id = format!("bench-job-{index}");
            let url = format!("https://example.test/jobs/{index}");
            transaction.execute(
                "INSERT INTO jobs (id, company_id, title, url, canonical_url, status, posting_state, source, description, is_new_from_watch, missing_from_sync_count, created_at, updated_at) VALUES (?1, 'bench-company', ?2, ?3, ?3, 'wishlist', 'active', 'manual', ?4, 0, 0, ?5, ?5)",
                params![id, title, url, description, timestamp],
            ).unwrap();
        }
        transaction.commit().unwrap();

        // Warm SQLite's page cache, then sample enough repeated searches to
        // provide a stable p95 without making ordinary unit tests timing-sensitive.
        for _ in 0..3 {
            list_jobs(&conn, JobFilters { search: Some("sandbo".into()), ..JobFilters::default() }).unwrap();
        }
        let mut samples = Vec::with_capacity(20);
        for _ in 0..20 {
            let started = Instant::now();
            let results = list_jobs(
                &conn,
                JobFilters { search: Some("sandbo".into()), ..JobFilters::default() },
            ).unwrap();
            samples.push(started.elapsed());
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].job.title, "Software Engineer, Sandboxing");
        }
        samples.sort_unstable();
        let p95 = samples[18];
        println!("5,000-job long-description search p95: {} ms", p95.as_millis());
        assert!(p95 <= Duration::from_millis(100), "search p95 exceeded 100 ms: {p95:?}");
    }

    fn insert_watch_pending_job(
        conn: &Connection,
        company_id: &str,
        title: &str,
        url: &str,
    ) -> String {
        let id = create_id();
        let timestamp = now_iso();
        let canonical = normalize_canonical_url(url).unwrap();
        conn.execute(
            r#"INSERT INTO jobs (
                id, company_id, title, url, canonical_url, source_external_id, status, applied_at,
                posting_state, last_checked_at, last_check_result, source, notes, location,
                is_new_from_watch, missing_from_sync_count, created_at, updated_at
            ) VALUES (?1,?2,?3,?4,?5,?6,'wishlist',NULL,'unknown',NULL,NULL,'ats',NULL,NULL,1,0,?7,?7)"#,
            params![id, company_id, title, url, canonical, format!("ext-{id}"), timestamp],
        )
        .unwrap();
        id
    }

    #[test]
    fn title_fallback_consumes_shared_metadata() {
        let metadata =
            extract_job_metadata(r#"<meta property="og:title" content="Shared Resolver Title">"#);

        assert_eq!(
            title_from_metadata("https://example.com/jobs/123", Some(&metadata)),
            "Shared Resolver Title"
        );
    }

    #[tokio::test]
    async fn manual_title_is_preserved() {
        let title = resolve_title_from_url(
            "file:///this-would-fail-if-fetched",
            Some("  Manually Entered Title  "),
        )
        .await;

        assert_eq!(title, "Manually Entered Title");
    }

    #[test]
    fn list_jobs_excludes_pending_watch_discoveries_by_default() {
        let conn = test_connection();
        let company = find_or_create_company(&conn, "Acme", None).unwrap();
        let (tracked, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/tracked",
            "Tracked Role",
            Some("Acme"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let pending_id = insert_watch_pending_job(
            &conn,
            &company.id,
            "Pending Role",
            "https://example.com/jobs/pending",
        );

        let pipeline = list_jobs(&conn, JobFilters::default()).unwrap();
        let ids: Vec<_> = pipeline.iter().map(|item| item.job.id.as_str()).collect();
        assert!(ids.contains(&tracked.id.as_str()));
        assert!(!ids.contains(&pending_id.as_str()));

        let inbox = list_jobs(
            &conn,
            JobFilters {
                new_from_watch: Some(true),
                ..JobFilters::default()
            },
        )
        .unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].job.id, pending_id);
    }

    #[test]
    fn list_jobs_searches_posting_url_and_filters_overlapping_salary_ranges() {
        let conn = test_connection();
        let (url_match, _) = create_job_from_url(
            &conn,
            "https://example.com/openings/unique-posting-code",
            "Senior Engineer",
            Some("Acme Research"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (salary_job, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/salary-role",
            "Salary Role",
            Some("Acme Research"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (open_ended, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/open-ended",
            "Open Ended",
            Some("Acme Research"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        conn.execute(
            "UPDATE jobs SET salary_min=120000, salary_max=180000 WHERE id=?1",
            [&salary_job.id],
        )
        .unwrap();
        conn.execute(
            "UPDATE jobs SET salary_min=200000, salary_max=NULL WHERE id=?1",
            [&open_ended.id],
        )
        .unwrap();

        let by_url = list_jobs(
            &conn,
            JobFilters {
                search: Some("UNIQUE-POSTING-CODE".into()),
                ..JobFilters::default()
            },
        )
        .unwrap();
        assert!(by_url.iter().any(|item| item.job.id == url_match.id));
        let overlap = list_jobs(
            &conn,
            JobFilters {
                salary_min: Some(175000),
                salary_max: Some(210000),
                ..JobFilters::default()
            },
        )
        .unwrap();
        let overlap_ids: Vec<_> = overlap.iter().map(|item| item.job.id.as_str()).collect();
        assert!(overlap_ids.contains(&salary_job.id.as_str()));
        assert!(overlap_ids.contains(&open_ended.id.as_str()));
        assert_eq!(overlap.len(), 2);
        let upper_bound = list_jobs(
            &conn,
            JobFilters {
                salary_max: Some(119999),
                ..JobFilters::default()
            },
        )
        .unwrap();
        assert!(upper_bound.is_empty());
        let lower_open = list_jobs(
            &conn,
            JobFilters {
                salary_max: Some(120000),
                ..JobFilters::default()
            },
        )
        .unwrap();
        assert_eq!(lower_open.len(), 1);
        assert_eq!(lower_open[0].job.id, salary_job.id);
    }

    #[test]
    fn update_job_persists_and_validates_salary_bounds() {
        let conn = test_connection();
        let (job, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/salary-update",
            "Salary Update",
            Some("Acme"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let saved = update_job(
            &conn,
            &job.id,
            UpdateJobInput {
                salary_min: Some(Some(100000)),
                salary_max: Some(Some(150000)),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();
        assert_eq!(saved.job.salary_min, Some(100000));
        assert_eq!(saved.job.salary_max, Some(150000));
        let error = update_job(
            &conn,
            &job.id,
            UpdateJobInput {
                salary_min: Some(Some(160000)),
                ..UpdateJobInput::default()
            },
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("minimum no greater than maximum"));
    }

    #[test]
    fn pipeline_counts_exclude_pending_watch_discoveries() {
        let conn = test_connection();
        let company = find_or_create_company(&conn, "Acme", None).unwrap();
        create_job_from_url(
            &conn,
            "https://example.com/jobs/tracked",
            "Tracked Role",
            Some("Acme"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap();
        insert_watch_pending_job(
            &conn,
            &company.id,
            "Pending Role",
            "https://example.com/jobs/pending",
        );

        let counts = get_pipeline_counts(&conn).unwrap();
        assert_eq!(counts.get("all"), Some(&1));
        assert_eq!(counts.get("wishlist"), Some(&1));
    }

    #[test]
    fn approve_watch_job_clears_flag_and_keeps_wishlist() {
        let conn = test_connection();
        let company = find_or_create_company(&conn, "Acme", None).unwrap();
        let job_id = insert_watch_pending_job(
            &conn,
            &company.id,
            "Pending Role",
            "https://example.com/jobs/pending",
        );

        let approved = approve_watch_job(&conn, &job_id).unwrap();
        assert!(!approved.is_new_from_watch);
        assert_eq!(approved.status, "wishlist");

        let events: Vec<String> = conn
            .prepare("SELECT type FROM job_events WHERE job_id = ?1 ORDER BY occurred_at")
            .unwrap()
            .query_map(params![job_id], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert!(events.iter().any(|t| t == "approved_from_watch"));

        let counts = get_pipeline_counts(&conn).unwrap();
        assert_eq!(counts.get("wishlist"), Some(&1));
        let inbox = list_jobs(
            &conn,
            JobFilters {
                new_from_watch: Some(true),
                ..JobFilters::default()
            },
        )
        .unwrap();
        assert!(inbox.is_empty());
    }

    #[test]
    fn dismiss_watch_job_keeps_posting_open_but_hides_it_from_pipeline() {
        let conn = test_connection();
        let company = find_or_create_company(&conn, "Acme", None).unwrap();
        let job_id = insert_watch_pending_job(
            &conn,
            &company.id,
            "Pending Role",
            "https://example.com/jobs/pending",
        );

        let dismissed = dismiss_watch_job(&conn, &job_id).unwrap();
        assert!(!dismissed.is_new_from_watch);
        assert_eq!(dismissed.status, "wishlist");
        assert_eq!(dismissed.watch_disposition.as_deref(), Some("dismissed"));

        let events: Vec<String> = conn
            .prepare("SELECT type FROM job_events WHERE job_id = ?1")
            .unwrap()
            .query_map(params![job_id], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert!(events.iter().any(|t| t == "dismissed_from_watch"));

        let counts = get_pipeline_counts(&conn).unwrap();
        assert_eq!(counts.get("all"), Some(&0));
        assert_eq!(counts.get("closed"), Some(&0));
        assert_eq!(counts.get("wishlist"), Some(&0));
        let pipeline = list_jobs(&conn, JobFilters::default()).unwrap();
        assert!(pipeline.is_empty());
        let inbox = list_jobs(
            &conn,
            JobFilters {
                new_from_watch: Some(true),
                ..JobFilters::default()
            },
        )
        .unwrap();
        assert!(inbox.is_empty());
    }

    #[test]
    fn open_watch_positions_include_dismissed_roles_and_can_save_them_again() {
        let conn = test_connection();
        let company = find_or_create_company(&conn, "Acme", None).unwrap();
        let job_id = insert_watch_pending_job(
            &conn,
            &company.id,
            "Pending Role",
            "https://example.com/jobs/pending",
        );
        conn.execute(
            "UPDATE jobs SET source = 'greenhouse', posting_state = 'active' WHERE id = ?1",
            params![job_id],
        )
        .unwrap();

        dismiss_watch_job(&conn, &job_id).unwrap();
        let positions = list_open_watch_positions(&conn, &company.id).unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(
            positions[0].job.watch_disposition.as_deref(),
            Some("dismissed")
        );

        let saved = save_open_watch_job(&conn, &job_id).unwrap();
        assert_eq!(saved.watch_disposition.as_deref(), Some("saved"));
        assert_eq!(saved.status, "wishlist");
        assert_eq!(list_jobs(&conn, JobFilters::default()).unwrap().len(), 1);
        assert_eq!(
            get_pipeline_counts(&conn).unwrap().get("wishlist"),
            Some(&1)
        );
    }

    #[test]
    fn reset_dismissed_watch_job_only_allows_closed_legacy_roles() {
        let conn = test_connection();
        let company = find_or_create_company(&conn, "Acme", None).unwrap();
        let job_id = insert_watch_pending_job(
            &conn,
            &company.id,
            "Pending Role",
            "https://example.com/jobs/pending",
        );
        conn.execute(
            "UPDATE jobs SET source = 'greenhouse', posting_state = 'active', status = 'closed' WHERE id = ?1",
            params![job_id],
        )
        .unwrap();
        dismiss_watch_job(&conn, &job_id).unwrap();

        let reset = reset_dismissed_watch_job(&conn, &job_id).unwrap();
        assert!(reset.is_new_from_watch);
        assert_eq!(reset.watch_disposition.as_deref(), Some("new"));
        assert_eq!(reset.status, "wishlist");

        conn.execute(
            "UPDATE jobs SET is_new_from_watch = 0, watch_disposition = 'dismissed', status = 'applied' WHERE id = ?1",
            params![job_id],
        )
        .unwrap();
        assert!(reset_dismissed_watch_job(&conn, &job_id).is_err());
    }

    #[test]
    fn watch_role_keywords_filter_new_roles_only() {
        let conn = test_connection();
        let company = find_or_create_company(&conn, "Thinking Machine Labs", None).unwrap();

        let id1 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Software Engineer, Developer Productivity, AI Tools",
            "https://example.com/jobs/1",
        );
        let id2 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Research Engineer, Developer Experience, Tinker",
            "https://example.com/jobs/2",
        );
        let id3 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Software Engineer, Full stack",
            "https://example.com/jobs/3",
        );

        // A job that's manually added (is_new_from_watch = 0)
        let (tracked, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/tracked",
            "Product Designer",
            Some("Thinking Machine Labs"),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // Without keywords, all new_from_watch jobs are returned
        let inbox = list_jobs(
            &conn,
            JobFilters {
                new_from_watch: Some(true),
                ..JobFilters::default()
            },
        )
        .unwrap();
        assert_eq!(inbox.len(), 3);

        // Configure the structured global criteria so the query-time filter
        // engine keeps only "Software Engineer" titles. Since task 8.1, the
        // watch listing evaluates the resolved FilterCriteria in memory rather
        // than reading the legacy `watch_role_keywords` SQL (the legacy
        // write-through to structured criteria is wired in task 12.2).
        let mut criteria = crate::filtering::model::FilterCriteria::match_all();
        criteria.title.include = vec!["Software Engineer".to_string()];
        crate::filtering::resolver::set_global_criteria(&conn, &criteria).unwrap();

        // Now inbox should only have the two Software Engineer jobs
        let filtered_inbox = list_jobs(
            &conn,
            JobFilters {
                new_from_watch: Some(true),
                ..JobFilters::default()
            },
        )
        .unwrap();
        assert_eq!(filtered_inbox.len(), 2);
        let ids: Vec<_> = filtered_inbox
            .iter()
            .map(|item| item.job.id.as_str())
            .collect();
        assert!(ids.contains(&id1.as_str()));
        assert!(ids.contains(&id3.as_str()));
        assert!(!ids.contains(&id2.as_str()));

        // The regular pipeline should still include the tracked job,
        // even though it doesn't match the "Software Engineer" keyword,
        // because keywords only apply to new_from_watch.
        let pipeline = list_jobs(&conn, JobFilters::default()).unwrap();
        let pipeline_ids: Vec<_> = pipeline.iter().map(|item| item.job.id.as_str()).collect();
        assert!(pipeline_ids.contains(&tracked.id.as_str()));
    }

    #[test]
    fn set_watch_role_keywords_write_through_filters_new_roles() {
        // Proves the legacy setter's write-through to structured criteria works
        // end to end (Req 15.2): calling `set_watch_role_keywords` makes the
        // watch listing filter, because the query-time engine reads the
        // structured global criteria the setter wrote through to.
        let conn = test_connection();
        let company = find_or_create_company(&conn, "Thinking Machine Labs", None).unwrap();

        let id1 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Software Engineer, Developer Productivity",
            "https://example.com/jobs/1",
        );
        let _id2 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Research Engineer, Developer Experience",
            "https://example.com/jobs/2",
        );
        let id3 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Software Engineer, Full stack",
            "https://example.com/jobs/3",
        );

        // Drive the legacy setter only — no direct structured write.
        set_watch_role_keywords(&conn, "Software Engineer").unwrap();

        // The legacy key is retained for rollback (Req 15.1).
        assert_eq!(
            get_watch_role_keywords(&conn).unwrap(),
            "Software Engineer".to_string()
        );

        // The structured global criteria were updated via write-through.
        let criteria = crate::filtering::resolver::get_global_criteria(&conn).unwrap();
        assert_eq!(
            criteria.title.include,
            vec!["Software Engineer".to_string()]
        );
        assert_eq!(
            criteria.title.match_mode,
            crate::filtering::model::MatchMode::Word
        );

        // The watch listing now filters to the two Software Engineer roles.
        let filtered = list_jobs(
            &conn,
            JobFilters {
                new_from_watch: Some(true),
                ..JobFilters::default()
            },
        )
        .unwrap();
        let ids: Vec<_> = filtered.iter().map(|item| item.job.id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1.as_str()));
        assert!(ids.contains(&id3.as_str()));
    }

    #[test]
    fn set_location_settings_writes_through_to_structured_criteria() {
        // Proves the location legacy setter writes through to the structured
        // global criteria's location fields (Req 15.2), while retaining the
        // legacy keys for rollback (Req 15.1).
        let conn = test_connection();

        set_location_settings(
            &conn,
            &LocationSettings {
                country: "United States".to_string(),
                cities: "San Francisco, New York".to_string(),
            },
        )
        .unwrap();

        // Legacy keys retained.
        let legacy = get_location_settings(&conn).unwrap();
        assert_eq!(legacy.country, "United States");
        assert_eq!(legacy.cities, "San Francisco, New York");

        // Structured global criteria updated via write-through.
        let criteria = crate::filtering::resolver::get_global_criteria(&conn).unwrap();
        assert_eq!(criteria.location.country, Some("United States".to_string()));
        assert_eq!(
            criteria.location.include,
            vec!["San Francisco".to_string(), "New York".to_string()]
        );

        // An empty country clears location.country on write-through.
        set_location_settings(
            &conn,
            &LocationSettings {
                country: "  ".to_string(),
                cities: "Remote".to_string(),
            },
        )
        .unwrap();
        let criteria = crate::filtering::resolver::get_global_criteria(&conn).unwrap();
        assert_eq!(criteria.location.country, None);
        assert_eq!(criteria.location.include, vec!["Remote".to_string()]);
    }

    #[test]
    fn favorite_toggle_and_pipeline_filtering() {
        let conn = test_connection();
        let (job1, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/role1",
            "Role 1",
            Some("Acme"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap();

        let (job2, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/role2",
            "Role 2",
            Some("Beta"),
            Some("applied"),
            None,
            None,
            None,
        )
        .unwrap();

        assert!(!job1.is_favorite);
        assert!(!job2.is_favorite);

        // Toggle job1 to favorite
        let toggled = toggle_job_favorite(&conn, &job1.id).unwrap();
        assert!(toggled.job.is_favorite);

        // Counts should show 1 favorite
        let counts = get_pipeline_counts(&conn).unwrap();
        assert_eq!(counts.get("favorites"), Some(&1));
        assert_eq!(counts.get("all"), Some(&2));

        // Filter list_jobs by favorites
        let fav_jobs = list_jobs(
            &conn,
            JobFilters {
                is_favorite: Some(true),
                ..JobFilters::default()
            },
        )
        .unwrap();
        assert_eq!(fav_jobs.len(), 1);
        assert_eq!(fav_jobs[0].job.id, job1.id);

        // Toggle job1 back to not favorite
        let toggled_back = toggle_job_favorite(&conn, &job1.id).unwrap();
        assert!(!toggled_back.job.is_favorite);
        let counts_after = get_pipeline_counts(&conn).unwrap();
        assert_eq!(counts_after.get("favorites"), Some(&0));

        // Update via update_job
        let updated = update_job(
            &conn,
            &job2.id,
            UpdateJobInput {
                is_favorite: Some(true),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();
        assert!(updated.job.is_favorite);
        assert!(updated.events.iter().any(|e| e.event_type == "favorited"));
    }

    #[test]
    fn delete_job_cascades_and_removes_records() {
        let conn = test_connection();
        let (job, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/to-delete",
            "Role To Delete",
            Some("Acme"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap();

        // Add an event
        add_job_event(&conn, &job.id, "test_event", Some("Sample note")).unwrap();

        // Check records exist (1 for 'created' + 1 for 'test_event')
        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM job_events WHERE job_id = ?1",
                params![job.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(event_count, 2);

        // Delete job
        delete_job(&conn, &job.id).unwrap();

        // Verify job and child records are gone
        assert!(get_job_by_id(&conn, &job.id).unwrap().is_none());
        let event_count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM job_events WHERE job_id = ?1",
                params![job.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(event_count_after, 0);
        let company_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM companies WHERE id = ?1",
                params![job.company_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(company_count, 0);

        // Attempting to delete non-existent job errors
        assert!(delete_job(&conn, &job.id).is_err());
    }

    #[test]
    fn delete_job_removes_evidence_but_keeps_run_postings_history() {
        let conn = test_connection();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        let (job, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/evidence",
            "Role With Evidence",
            Some("Acme"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap();

        conn.execute(
            "INSERT INTO runs (id, run_type, status, trigger, owner_pid, owns_runner_lock,
                               started_at, stages_json, updated_at)
             VALUES ('run-1', 'posting_check', 'completed', 'desktop', 1, 0,
                     '2026-01-01T00:00:00Z', '[]', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO run_postings (run_id, job_id, ordinal, job_title, company_name,
                                       posting_url, state_at_start, status, posting_state)
             VALUES ('run-1', ?1, 0, 'Role With Evidence', 'Acme',
                     'https://example.com/jobs/evidence', 'unknown', 'completed', 'active')",
            params![job.id],
        )
        .unwrap();
        for (id, run_id) in [("ev-1", Some("run-1")), ("ev-2", None)] {
            conn.execute(
                "INSERT INTO posting_check_evidence (id, run_id, job_id, kind, attempted_at,
                     posting_state, reason_code, reason, evidence_version, evidence_json,
                     created_at)
                 VALUES (?1, ?2, ?3, 'authoritative', '2026-01-01T00:00:00Z', 'active',
                         'provider_listed_open', 'Open', 1, '{}', '2026-01-01T00:00:00Z')",
                params![id, run_id, job.id],
            )
            .unwrap();
        }

        delete_job(&conn, &job.id).unwrap();

        let count =
            |sql: &str| -> i64 { conn.query_row(sql, params![job.id], |r| r.get(0)).unwrap() };
        assert_eq!(
            count("SELECT COUNT(*) FROM posting_check_evidence WHERE job_id = ?1"),
            0
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM run_postings WHERE job_id = ?1"),
            1
        );
        assert!(get_job_by_id(&conn, &job.id).unwrap().is_none());
    }

    #[test]
    fn reassigning_jobs_reuses_destination_and_cleans_untracked_former_company() {
        let conn = test_connection();
        let (first, source) = create_job_from_url(
            &conn,
            "https://example.com/jobs/first",
            "First role",
            Some("Acme"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap();
        let (second, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/second",
            "Second role",
            Some("Acme"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap();

        let moved_first = update_job(
            &conn,
            &first.id,
            UpdateJobInput {
                company_name: Some("Target Co".into()),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();
        assert_eq!(moved_first.company.name, "Target Co");
        assert!(moved_first
            .events
            .iter()
            .any(|event| event.event_type == "company_changed"));

        let moved_second = update_job(
            &conn,
            &second.id,
            UpdateJobInput {
                company_name: Some("  target co  ".into()),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();
        assert_eq!(moved_second.company.id, moved_first.company.id);

        let source_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM companies WHERE id = ?1",
                params![source.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source_exists, 0);
        let destination_jobs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE company_id = ?1",
                params![moved_first.company.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(destination_jobs, 2);
    }

    #[test]
    fn reassigning_last_job_keeps_former_company_with_watch() {
        let conn = test_connection();
        let (job, source) = create_job_from_url(
            &conn,
            "https://example.com/jobs/watched",
            "Watched role",
            Some("Watched Acme"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap();
        crate::companies::insert_watch(&conn, &source.id, "greenhouse", "watched-acme").unwrap();

        update_job(
            &conn,
            &job.id,
            UpdateJobInput {
                company_name: Some("Elsewhere".into()),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();

        let source_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM companies WHERE id = ?1",
                params![source.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source_exists, 1);
    }

    #[test]
    fn reassigning_last_job_keeps_former_company_with_careers_history() {
        let conn = test_connection();
        let (job, source) = create_job_from_url(
            &conn,
            "https://example.com/jobs/monitored",
            "Monitored role",
            Some("Monitored Co"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO careers_page_snapshots (id, company_id, content_hash, normalized_text, captured_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![create_id(), source.id, "hash", "careers copy", now_iso()],
        )
        .unwrap();

        update_job(
            &conn,
            &job.id,
            UpdateJobInput {
                company_name: Some("Elsewhere".into()),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();

        let source_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM companies WHERE id = ?1",
                params![source.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source_exists, 1);
    }

    #[test]
    fn archive_and_unarchive_job_workflow() {
        let conn = test_connection();
        let (job, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/to-archive",
            "Role To Archive",
            Some("Acme"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap();

        // Archive job
        let archived = archive_job(&conn, &job.id).unwrap();
        assert_eq!(archived.job.status, "archived");

        // Check counts
        let counts = get_pipeline_counts(&conn).unwrap();
        assert_eq!(counts.get("archived"), Some(&1));
        assert_eq!(counts.get("archivedTotal"), Some(&1));

        // Filter list_jobs by is_archived = true
        let archived_list = list_jobs(
            &conn,
            JobFilters {
                is_archived: Some(true),
                ..JobFilters::default()
            },
        )
        .unwrap();
        assert_eq!(archived_list.len(), 1);
        assert_eq!(archived_list[0].job.id, job.id);

        // Filter list_jobs by is_archived = false
        let active_list = list_jobs(
            &conn,
            JobFilters {
                is_archived: Some(false),
                ..JobFilters::default()
            },
        )
        .unwrap();
        assert_eq!(active_list.len(), 0);

        // Unarchive job
        let restored = unarchive_job(&conn, &job.id, None).unwrap();
        assert_eq!(restored.job.status, "wishlist");

        let counts_after = get_pipeline_counts(&conn).unwrap();
        assert_eq!(counts_after.get("archived"), Some(&0));
        assert_eq!(counts_after.get("wishlist"), Some(&1));
    }

    #[test]
    fn setting_status_to_closed_deletes_job() {
        let conn = test_connection();
        let (job, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/closed",
            "Closed role",
            Some("Acme"),
            Some("applied"),
            None,
            None,
            None,
        )
        .unwrap();

        let result = update_job(
            &conn,
            &job.id,
            UpdateJobInput {
                status: Some("closed".into()),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();

        assert_eq!(result.job.status, "closed");
        assert!(get_job_by_id(&conn, &job.id).unwrap().is_none());
    }

    #[test]
    fn appeal_is_optional_and_does_not_change_favorite_or_closed_delete() {
        let conn = test_connection();
        let (job, _) = create_job_from_url(
            &conn,
            "https://example.com/jobs/appeal",
            "Appealing role",
            Some("Acme"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(job.appeal, None);
        assert!(!job.is_favorite);

        let favorited = set_job_favorite(&conn, &job.id, true).unwrap();
        assert!(favorited.job.is_favorite);

        let scored = update_job(
            &conn,
            &job.id,
            UpdateJobInput {
                appeal: Some(Some(5)),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();
        assert_eq!(scored.job.appeal, Some(5));
        assert!(scored.job.is_favorite);
        assert_eq!(
            scored
                .events
                .iter()
                .filter(|event| event.event_type == "favorited")
                .count(),
            1
        );

        let missing: UpdateJobInput = serde_json::from_str("{}").unwrap();
        assert!(missing.appeal.is_none());
        let clear_input: UpdateJobInput = serde_json::from_str(r#"{"appeal":null}"#).unwrap();
        assert_eq!(clear_input.appeal, Some(None));
        let set_input: UpdateJobInput = serde_json::from_str(r#"{"appeal":4}"#).unwrap();
        assert_eq!(set_input.appeal, Some(Some(4)));

        let cleared = update_job(&conn, &job.id, clear_input).unwrap();
        assert_eq!(cleared.job.appeal, None);
        assert!(cleared.job.is_favorite);

        assert!(update_job(
            &conn,
            &job.id,
            UpdateJobInput {
                appeal: Some(Some(0)),
                ..UpdateJobInput::default()
            },
        )
        .is_err());
        assert!(update_job(
            &conn,
            &job.id,
            UpdateJobInput {
                appeal: Some(Some(6)),
                ..UpdateJobInput::default()
            },
        )
        .is_err());
        assert_eq!(get_job_by_id(&conn, &job.id).unwrap().unwrap().appeal, None);

        let archived = archive_job(&conn, &job.id).unwrap();
        assert_eq!(archived.job.appeal, None);
        let restored = update_job(
            &conn,
            &job.id,
            UpdateJobInput {
                status: Some("wishlist".into()),
                appeal: Some(Some(3)),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();
        assert_eq!(restored.job.status, "wishlist");
        assert_eq!(restored.job.appeal, Some(3));

        let closed = update_job(
            &conn,
            &job.id,
            UpdateJobInput {
                status: Some("closed".into()),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();
        assert_eq!(closed.job.status, "closed");
        assert!(get_job_by_id(&conn, &job.id).unwrap().is_none());
    }

    // ---- Query-time filtering integration tests (task 8.3) ----
    //
    // These cover the query-time behavior of `list_jobs` on the watch path:
    // criteria changes re-filter without a re-sync (Req 8.4), the caller's
    // limit is applied *after* filtering (Req 8.3), and listing is purely
    // read-only — it never deletes or mutates stored rows (Req 18.1, 18.5).

    /// Set `updated_at` explicitly so tests can control ORDER BY / pre-filter
    /// ordering independently of insertion time.
    fn set_updated_at(conn: &Connection, job_id: &str, updated_at: &str) {
        conn.execute(
            "UPDATE jobs SET updated_at = ?1 WHERE id = ?2",
            params![updated_at, job_id],
        )
        .unwrap();
    }

    /// Build a title-include criteria in Word match mode (the default).
    fn title_include(terms: &[&str]) -> crate::filtering::model::FilterCriteria {
        let mut criteria = crate::filtering::model::FilterCriteria::match_all();
        criteria.title.include = terms.iter().map(|t| t.to_string()).collect();
        criteria
    }

    fn watch_inbox(conn: &Connection) -> Vec<JobListItem> {
        list_jobs(
            conn,
            JobFilters {
                new_from_watch: Some(true),
                ..JobFilters::default()
            },
        )
        .unwrap()
    }

    /// Req 8.4: changing criteria re-filters the *already-stored* watch rows on
    /// the very next listing, with no re-sync and no re-ingest. The stored rows
    /// are untouched between calls; only the returned Vec changes.
    #[test]
    fn criteria_change_refilters_without_resync() {
        let conn = test_connection();
        let company = find_or_create_company(&conn, "Acme", None).unwrap();

        let se = insert_watch_pending_job(
            &conn,
            &company.id,
            "Software Engineer",
            "https://example.com/jobs/se",
        );
        let re = insert_watch_pending_job(
            &conn,
            &company.id,
            "Research Engineer",
            "https://example.com/jobs/re",
        );
        let pm = insert_watch_pending_job(
            &conn,
            &company.id,
            "Product Manager",
            "https://example.com/jobs/pm",
        );

        // No criteria (match-all) returns all three watch-pending rows.
        let all = watch_inbox(&conn);
        assert_eq!(all.len(), 3);

        // Tighten to "Software Engineer": only that subset survives on the next
        // listing — no re-sync was performed, the same stored rows are re-read.
        crate::filtering::resolver::set_global_criteria(
            &conn,
            &title_include(&["Software Engineer"]),
        )
        .unwrap();
        let filtered = watch_inbox(&conn);
        let ids: Vec<_> = filtered.iter().map(|i| i.job.id.as_str()).collect();
        assert_eq!(ids, vec![se.as_str()]);

        // Change criteria again to "Engineer" (Word match). The listing reflects
        // the new criteria on the next call — both engineer roles return, the PM
        // role does not — still without any re-sync.
        crate::filtering::resolver::set_global_criteria(&conn, &title_include(&["Engineer"]))
            .unwrap();
        let refiltered = watch_inbox(&conn);
        let mut ids: Vec<_> = refiltered.iter().map(|i| i.job.id.clone()).collect();
        ids.sort();
        let mut expected = vec![se.clone(), re.clone()];
        expected.sort();
        assert_eq!(ids, expected);
        assert!(!refiltered.iter().any(|i| i.job.id == pm));
    }

    /// Req 8.3: the caller's `limit` is applied *after* filtering. This is
    /// constructed adversarially: the non-matching jobs are made "newer"
    /// (later `updated_at`) so a naive pre-filter `LIMIT K` pushed into SQL
    /// would grab those newest rows first and then filter them all out,
    /// yielding fewer than K matches. Correct post-filter limiting still
    /// returns exactly K matching rows.
    #[test]
    fn limit_applies_after_filtering() {
        let conn = test_connection();
        let company = find_or_create_company(&conn, "Acme", None).unwrap();

        // Three matching rows (older) ...
        let m1 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Software Engineer One",
            "https://example.com/jobs/m1",
        );
        let m2 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Software Engineer Two",
            "https://example.com/jobs/m2",
        );
        let m3 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Software Engineer Three",
            "https://example.com/jobs/m3",
        );
        // ... and three non-matching rows made strictly newer so a pre-filter
        // LIMIT would grab these first.
        let n1 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Product Manager One",
            "https://example.com/jobs/n1",
        );
        let n2 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Product Manager Two",
            "https://example.com/jobs/n2",
        );
        let n3 = insert_watch_pending_job(
            &conn,
            &company.id,
            "Product Manager Three",
            "https://example.com/jobs/n3",
        );

        // Matching rows are older; non-matching rows are newer (listing is
        // ORDER BY updated_at DESC, so newest come first).
        set_updated_at(&conn, &m1, "2024-01-01T00:00:00Z");
        set_updated_at(&conn, &m2, "2024-01-02T00:00:00Z");
        set_updated_at(&conn, &m3, "2024-01-03T00:00:00Z");
        set_updated_at(&conn, &n1, "2024-06-01T00:00:00Z");
        set_updated_at(&conn, &n2, "2024-06-02T00:00:00Z");
        set_updated_at(&conn, &n3, "2024-06-03T00:00:00Z");

        crate::filtering::resolver::set_global_criteria(
            &conn,
            &title_include(&["Software Engineer"]),
        )
        .unwrap();

        const K: usize = 3;
        let out = list_jobs(
            &conn,
            JobFilters {
                new_from_watch: Some(true),
                limit: Some(K),
                ..JobFilters::default()
            },
        )
        .unwrap();

        // Exactly K matching rows returned — not fewer. Every returned row is a
        // matching (Software Engineer) row, none of the newer PM rows leaked in.
        assert_eq!(out.len(), K);
        let matched: std::collections::HashSet<_> = out.iter().map(|i| i.job.id.clone()).collect();
        let expected: std::collections::HashSet<_> =
            [m1.clone(), m2.clone(), m3.clone()].into_iter().collect();
        assert_eq!(matched, expected);
    }

    /// Req 18.1, 18.5: listing with restrictive criteria that hides most watch
    /// rows is non-destructive. The stored row count is unchanged before and
    /// after listing — filtering only shaped the returned Vec, not the table.
    #[test]
    fn listing_with_restrictive_criteria_is_non_destructive() {
        let conn = test_connection();
        let company = find_or_create_company(&conn, "Acme", None).unwrap();

        insert_watch_pending_job(
            &conn,
            &company.id,
            "Software Engineer",
            "https://example.com/jobs/1",
        );
        insert_watch_pending_job(
            &conn,
            &company.id,
            "Research Engineer",
            "https://example.com/jobs/2",
        );
        insert_watch_pending_job(
            &conn,
            &company.id,
            "Product Manager",
            "https://example.com/jobs/3",
        );

        let count_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count_before, 3);

        // A restrictive title that matches nothing hides every watch row.
        crate::filtering::resolver::set_global_criteria(
            &conn,
            &title_include(&["Chief Executive Officer"]),
        )
        .unwrap();
        let out = watch_inbox(&conn);
        assert!(out.is_empty(), "restrictive criteria should hide all rows");

        // The stored jobs are untouched — nothing was deleted by listing.
        let count_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count_after, count_before);

        // Relaxing back to match-all shows all rows again, proving the data was
        // preserved rather than removed.
        crate::filtering::resolver::set_global_criteria(
            &conn,
            &crate::filtering::model::FilterCriteria::match_all(),
        )
        .unwrap();
        assert_eq!(watch_inbox(&conn).len(), 3);
    }
}
