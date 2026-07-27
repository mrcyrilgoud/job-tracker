use std::collections::HashMap;

use chrono::{Datelike, Local, Timelike};
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::metadata::{resolve_job_metadata, JobMetadata};
use crate::models::{
    AttachedDocument, Company, Document, Job, JobDetail, JobDocument, JobEvent, JobListItem,
    WeeklyActivity, WeeklyDay,
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

fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
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
        location: row.get(13)?,
        is_new_from_watch: row.get::<_, i64>(14)? != 0,
        missing_from_sync_count: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

const JOB_COLS: &str = "id, company_id, title, url, canonical_url, source_external_id, status, applied_at, posting_state, last_checked_at, last_check_result, source, notes, location, is_new_from_watch, missing_from_sync_count, created_at, updated_at";

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

    let company = find_or_create_company(conn, company_name, None)?;
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
            posting_state, last_checked_at, last_check_result, source, notes, location,
            is_new_from_watch, missing_from_sync_count, created_at, updated_at
        ) VALUES (?1,?2,?3,?4,?5,NULL,?6,?7,'unknown',NULL,NULL,'manual',?8,?9,0,0,?10,?10)"#,
        params![
            job_id,
            company.id,
            resolved_title,
            url,
            canonical_url,
            status,
            applied,
            notes,
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
    let timestamp = now_iso();
    if let Some(mut existing) = conn
        .query_row(
            "SELECT id, name, careers_url, created_at, updated_at FROM companies WHERE name = ?1",
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

pub fn get_job_by_id(conn: &Connection, job_id: &str) -> AppResult<Option<Job>> {
    conn.query_row(
        &format!("SELECT {JOB_COLS} FROM jobs WHERE id = ?1"),
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
    pub new_from_watch: Option<bool>,
}

pub fn list_jobs(conn: &Connection, filters: JobFilters) -> AppResult<Vec<JobListItem>> {
    let mut sql = String::from(
        "SELECT j.id, j.company_id, j.title, j.url, j.canonical_url, j.source_external_id, j.status, j.applied_at, j.posting_state, j.last_checked_at, j.last_check_result, j.source, j.notes, j.location, j.is_new_from_watch, j.missing_from_sync_count, j.created_at, j.updated_at, c.name
         FROM jobs j INNER JOIN companies c ON j.company_id = c.id WHERE 1=1",
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
    if filters.new_from_watch == Some(true) {
        sql.push_str(" AND j.is_new_from_watch = 1");
    }
    if let Some(search) = &filters.search {
        sql.push_str(" AND j.title LIKE ?");
        values.push(Box::new(format!("%{search}%")));
    }
    sql.push_str(" ORDER BY j.updated_at DESC");

    let mut stmt = conn.prepare(&sql).map_err(map_sqlite)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
    let rows = stmt
        .query_map(params_ref.as_slice(), |row| {
            Ok(JobListItem {
                job: map_job(row)?,
                company_name: row.get(18)?,
            })
        })
        .map_err(map_sqlite)?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(map_sqlite)?);
    }
    Ok(out)
}

pub fn get_job_detail(conn: &Connection, job_id: &str) -> AppResult<Option<JobDetail>> {
    let mut stmt = conn
        .prepare(
            "SELECT j.id, j.company_id, j.title, j.url, j.canonical_url, j.source_external_id, j.status, j.applied_at, j.posting_state, j.last_checked_at, j.last_check_result, j.source, j.notes, j.location, j.is_new_from_watch, j.missing_from_sync_count, j.created_at, j.updated_at,
                    c.id, c.name, c.careers_url, c.created_at, c.updated_at
             FROM jobs j INNER JOIN companies c ON j.company_id = c.id WHERE j.id = ?1",
        )
        .map_err(map_sqlite)?;

    let detail = stmt
        .query_row(params![job_id], |row| {
            Ok((
                map_job(row)?,
                Company {
                    id: row.get(18)?,
                    name: row.get(19)?,
                    careers_url: row.get(20)?,
                    created_at: row.get(21)?,
                    updated_at: row.get(22)?,
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

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateJobInput {
    pub title: Option<String>,
    pub company_name: Option<String>,
    pub status: Option<String>,
    pub applied_at: Option<Option<String>>,
    pub notes: Option<Option<String>>,
    pub location: Option<Option<String>>,
    pub url: Option<String>,
    pub is_new_from_watch: Option<bool>,
}

use serde::Deserialize;

pub fn update_job(conn: &Connection, job_id: &str, updates: UpdateJobInput) -> AppResult<JobDetail> {
    let existing = get_job_by_id(conn, job_id)?
        .ok_or_else(|| AppError::from("Job not found"))?;
    let timestamp = now_iso();
    let mut company_id = existing.company_id.clone();
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
        company_id = company.id;
    }

    let next_status = updates
        .status
        .clone()
        .unwrap_or_else(|| existing.status.clone());
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
    let location = updates
        .location
        .clone()
        .unwrap_or_else(|| existing.location.clone());
    let is_new = updates
        .is_new_from_watch
        .unwrap_or(existing.is_new_from_watch);

    conn.execute(
        r#"UPDATE jobs SET title=?1, company_id=?2, url=?3, canonical_url=?4, status=?5,
           applied_at=?6, notes=?7, location=?8, is_new_from_watch=?9, updated_at=?10
           WHERE id=?11"#,
        params![
            title,
            company_id,
            next_url,
            next_canonical,
            next_status,
            next_applied,
            notes,
            location,
            if is_new { 1 } else { 0 },
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

    get_job_detail(conn, job_id)?.ok_or_else(|| AppError::from("Job not found after update"))
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
        ("wishlist", 0),
        ("applied", 0),
        ("interviewing", 0),
        ("offer", 0),
        ("rejected", 0),
        ("withdrawn", 0),
        ("closed", 0),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();

    let mut stmt = conn
        .prepare("SELECT status, COUNT(*) FROM jobs GROUP BY status")
        .map_err(map_sqlite)?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
        .map_err(map_sqlite)?;

    let mut total = 0i64;
    for row in rows {
        let (status, count) = row.map_err(map_sqlite)?;
        total += count;
        counts.insert(status, count);
    }
    counts.insert("all".into(), total);
    Ok(counts)
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
        .prepare("SELECT occurred_at FROM job_events WHERE occurred_at >= ?1")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::metadata::extract_job_metadata;

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
}
