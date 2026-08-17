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
        watch_disposition: row.get(15)?,
        missing_from_sync_count: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
    })
}

const JOB_COLS: &str = "id, company_id, title, url, canonical_url, source_external_id, status, applied_at, posting_state, last_checked_at, last_check_result, source, notes, location, is_new_from_watch, watch_disposition, missing_from_sync_count, created_at, updated_at";

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
    location: Option<&str>,
    careers_url: Option<&str>,
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
            posting_state, last_checked_at, last_check_result, source, notes, location,
            is_new_from_watch, watch_disposition, missing_from_sync_count, created_at, updated_at
        ) VALUES (?1,?2,?3,?4,?5,NULL,?6,?7,'unknown',NULL,NULL,'manual',?8,?9,0,NULL,0,?10,?10)"#,
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
    pub location: Option<String>,
    pub new_from_watch: Option<bool>,
}

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct LocationSettings {
    pub country: String,
    pub cities: String,
}

fn expand_country_keywords(country: &str) -> Vec<String> {
    let mut expanded = Vec::new();
    let lower_country = country.trim().to_lowercase();
    
    if lower_country.is_empty() {
        return expanded;
    }

    expanded.push(lower_country.clone());

    if lower_country == "united states" || lower_country == "usa" || lower_country == "us" {
        expanded.push("united states".to_string());
        expanded.push("usa".to_string());
        expanded.push(", us".to_string());
        expanded.push("remote - us".to_string());
        
        let states = vec![
            "al", "ak", "az", "ar", "ca", "co", "ct", "de", "fl", "ga", 
            "hi", "id", "il", "in", "ia", "ks", "ky", "la", "me", "md", 
            "ma", "mi", "mn", "ms", "mo", "mt", "ne", "nv", "nh", "nj", 
            "nm", "ny", "nc", "nd", "oh", "ok", "or", "pa", "ri", "sc", 
            "sd", "tn", "tx", "ut", "vt", "va", "wa", "wv", "wi", "wy"
        ];
        for state in states {
            expanded.push(format!(", {}", state));
        }

        let hubs = vec![
            "san francisco", "san jose", "new york", "nyc", "seattle", "austin", 
            "boston", "chicago", "los angeles", "palo alto", "mountain view", 
            "sunnyvale", "santa clara", "menlo park", "redwood city", "san mateo", 
            "oakland", "berkeley", "santa monica", "venice", "culver city", "irvine", 
            "brooklyn", "manhattan", "queens", "jersey city", "bellevue", "redmond", 
            "kirkland", "cambridge", "atlanta", "denver", "boulder", "salt lake city", 
            "washington dc", "miami", "dallas", "houston", "raleigh", "bay area"
        ];
        for hub in hubs {
            expanded.push(hub.to_string());
        }
    } else if lower_country == "united kingdom" || lower_country == "uk" {
        expanded.push("united kingdom".to_string());
        expanded.push("uk".to_string());
        expanded.push(", uk".to_string());
        expanded.push("london".to_string());
    } else if lower_country == "canada" {
        expanded.push("canada".to_string());
        // Careful with "ca" which is California in the US context
        expanded.push(", on".to_string());
        expanded.push(", bc".to_string());
        expanded.push(", qc".to_string());
        expanded.push(", ab".to_string());
    }

    let mut unique = Vec::new();
    for e in expanded {
        if !unique.contains(&e) {
            unique.push(e);
        }
    }
    unique
}

fn expand_location_keywords(cities: &str) -> Vec<String> {
    let mut expanded = Vec::new();
    let lower_cities = cities.to_lowercase();
    
    // Simple predefined regions mapping for smarter "dynamic" filtering
    if lower_cities.contains("san jose") || lower_cities.contains("san francisco") || lower_cities.contains("oakland") || lower_cities.contains("bay area") {
        expanded.push("san jose".to_string());
        expanded.push("san francisco".to_string());
        expanded.push("oakland".to_string());
        expanded.push("santa clara".to_string());
        expanded.push("palo alto".to_string());
        expanded.push("mountain view".to_string());
        expanded.push("sunnyvale".to_string());
        expanded.push("cupertino".to_string());
        expanded.push("menlo park".to_string());
        expanded.push("san mateo".to_string());
        expanded.push("redwood city".to_string());
        expanded.push("bay area".to_string());
    }
    
    if lower_cities.contains("new york") || lower_cities.contains("nyc") || lower_cities.contains("brooklyn") {
        expanded.push("new york".to_string());
        expanded.push("nyc".to_string());
        expanded.push("brooklyn".to_string());
        expanded.push("manhattan".to_string());
        expanded.push("queens".to_string());
        expanded.push("jersey city".to_string());
    }

    if lower_cities.contains("seattle") || lower_cities.contains("bellevue") || lower_cities.contains("redmond") {
        expanded.push("seattle".to_string());
        expanded.push("bellevue".to_string());
        expanded.push("redmond".to_string());
        expanded.push("kirkland".to_string());
    }

    if lower_cities.contains("los angeles") || lower_cities.contains("santa monica") || lower_cities.contains("la") || lower_cities.contains("socal") {
        expanded.push("los angeles".to_string());
        expanded.push("santa monica".to_string());
        expanded.push("venice".to_string());
        expanded.push("culver city".to_string());
        expanded.push("irvine".to_string());
    }

    // Include user typed cities
    for city in cities.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        expanded.push(city.to_lowercase());
    }

    let mut unique = Vec::new();
    for e in expanded {
        if !unique.contains(&e) {
            unique.push(e);
        }
    }
    unique
}

pub fn list_jobs(conn: &Connection, filters: JobFilters) -> AppResult<Vec<JobListItem>> {
    let mut sql = String::from(
        "SELECT j.id, j.company_id, j.title, j.url, j.canonical_url, j.source_external_id, j.status, j.applied_at, j.posting_state, j.last_checked_at, j.last_check_result, j.source, j.notes, j.location, j.is_new_from_watch, j.watch_disposition, j.missing_from_sync_count, j.created_at, j.updated_at, c.name
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
    } else {
        // Pending and dismissed watch discoveries stay out of the pipeline.
        sql.push_str(
            " AND j.is_new_from_watch = 0 \
             AND (j.watch_disposition IS NULL OR j.watch_disposition != 'dismissed')",
        );
    }
    if let Some(search) = &filters.search {
        sql.push_str(" AND j.title LIKE ?");
        values.push(Box::new(format!("%{search}%")));
    }

    // Location filtering based on global settings
    let loc_settings = get_location_settings(conn).unwrap_or_default();
    let mut location_clauses = Vec::new();
    
    if !loc_settings.country.trim().is_empty() {
        let expanded_country = expand_country_keywords(&loc_settings.country);
        for c in expanded_country {
            location_clauses.push("j.location LIKE ?");
            values.push(Box::new(format!("%{}%", c)));
        }
    }
    if !loc_settings.cities.trim().is_empty() {
        let expanded_cities = expand_location_keywords(&loc_settings.cities);
        for city in expanded_cities {
            location_clauses.push("j.location LIKE ?");
            values.push(Box::new(format!("%{}%", city)));
        }
    }
    
    if !location_clauses.is_empty() {
        sql.push_str(" AND (");
        sql.push_str(&location_clauses.join(" OR "));
        sql.push_str(")");
    }

    // Optionally still support the UI location filter if passed
    if let Some(location) = &filters.location {
        sql.push_str(" AND j.location LIKE ?");
        values.push(Box::new(format!("%{location}%")));
    }

    if filters.new_from_watch == Some(true) {
        let keywords = get_watch_role_keywords(conn).unwrap_or_default();
        if !keywords.trim().is_empty() {
            let terms: Vec<&str> = keywords.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            if !terms.is_empty() {
                let mut keyword_clauses = Vec::new();
                for term in terms {
                    keyword_clauses.push("j.title LIKE ?");
                    values.push(Box::new(format!("%{term}%")));
                }
                sql.push_str(" AND (");
                sql.push_str(&keyword_clauses.join(" OR "));
                sql.push_str(")");
            }
        }
    }

    sql.push_str(" ORDER BY j.updated_at DESC");

    let mut stmt = conn.prepare(&sql).map_err(map_sqlite)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
    let rows = stmt
        .query_map(params_ref.as_slice(), |row| {
            Ok(JobListItem {
                job: map_job(row)?,
                company_name: row.get(19)?,
            })
        })
        .map_err(map_sqlite)?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(map_sqlite)?);
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
    let loc_settings = get_location_settings(conn).unwrap_or_default();
    let mut sql = String::from(
        "SELECT j.id, j.company_id, j.title, j.url, j.canonical_url, j.source_external_id, j.status, j.applied_at, j.posting_state, j.last_checked_at, j.last_check_result, j.source, j.notes, j.location, j.is_new_from_watch, j.watch_disposition, j.missing_from_sync_count, j.created_at, j.updated_at, c.name
         FROM jobs j INNER JOIN companies c ON j.company_id = c.id
         WHERE j.company_id = ?1
           AND j.posting_state = 'active'
           AND j.source IN ('greenhouse', 'lever', 'ashby')"
    );
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(company_id.to_string())];

    let mut location_clauses = Vec::new();
    
    if !loc_settings.country.trim().is_empty() {
        let expanded_country = expand_country_keywords(&loc_settings.country);
        for c in expanded_country {
            location_clauses.push("j.location LIKE ?");
            values.push(Box::new(format!("%{}%", c)));
        }
    }
    if !loc_settings.cities.trim().is_empty() {
        let expanded_cities = expand_location_keywords(&loc_settings.cities);
        for city in expanded_cities {
            location_clauses.push("j.location LIKE ?");
            values.push(Box::new(format!("%{}%", city)));
        }
    }
    
    if !location_clauses.is_empty() {
        sql.push_str(" AND (");
        sql.push_str(&location_clauses.join(" OR "));
        sql.push_str(")");
    }

    sql.push_str(" ORDER BY CASE j.watch_disposition
       WHEN 'new' THEN 0
       WHEN 'saved' THEN 1
       WHEN 'dismissed' THEN 2
       ELSE 1
     END, j.title COLLATE NOCASE");

    let mut stmt = conn.prepare(&sql).map_err(map_sqlite)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
    let rows = stmt
        .query_map(params_ref.as_slice(), |row| {
            Ok(JobListItem {
                job: map_job(row)?,
                company_name: row.get(19)?,
            })
        })
        .map_err(map_sqlite)?;

    rows.collect::<Result<Vec<_>, _>>().map_err(map_sqlite)
}

pub fn get_job_detail(conn: &Connection, job_id: &str) -> AppResult<Option<JobDetail>> {
    let mut stmt = conn
        .prepare(
            "SELECT j.id, j.company_id, j.title, j.url, j.canonical_url, j.source_external_id, j.status, j.applied_at, j.posting_state, j.last_checked_at, j.last_check_result, j.source, j.notes, j.location, j.is_new_from_watch, j.watch_disposition, j.missing_from_sync_count, j.created_at, j.updated_at,
                    c.id, c.name, c.careers_url, c.created_at, c.updated_at
             FROM jobs j INNER JOIN companies c ON j.company_id = c.id WHERE j.id = ?1",
        )
        .map_err(map_sqlite)?;

    let detail = stmt
        .query_row(params![job_id], |row| {
            Ok((
                map_job(row)?,
                Company {
                    id: row.get(19)?,
                    name: row.get(20)?,
                    careers_url: row.get(21)?,
                    created_at: row.get(22)?,
                    updated_at: row.get(23)?,
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

pub fn update_job(
    conn: &Connection,
    job_id: &str,
    updates: UpdateJobInput,
) -> AppResult<JobDetail> {
    let existing = get_job_by_id(conn, job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
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
    let value: Option<String> = conn.query_row(
        "SELECT value FROM app_settings WHERE key = 'watch_role_keywords'",
        [],
        |row| row.get(0),
    ).optional().map_err(map_sqlite)?;
    Ok(value.unwrap_or_default())
}

pub fn set_watch_role_keywords(conn: &Connection, keywords: &str) -> AppResult<()> {
    let timestamp = now_iso();
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES ('watch_role_keywords', ?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![keywords, timestamp],
    ).map_err(map_sqlite)?;
    Ok(())
}

pub fn get_location_settings(conn: &Connection) -> AppResult<LocationSettings> {
    let country: Option<String> = conn.query_row(
        "SELECT value FROM app_settings WHERE key = 'location_country'",
        [],
        |row| row.get(0),
    ).optional().map_err(map_sqlite)?;
    let cities: Option<String> = conn.query_row(
        "SELECT value FROM app_settings WHERE key = 'location_cities'",
        [],
        |row| row.get(0),
    ).optional().map_err(map_sqlite)?;
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
    ).map_err(map_sqlite)?;
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES ('location_cities', ?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![settings.cities, timestamp],
    ).map_err(map_sqlite)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;
    use crate::jobs::metadata::extract_job_metadata;

    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        connection
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

        // Set keyword to "Software Engineer"
        set_watch_role_keywords(&conn, "Software Engineer").unwrap();

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
        let ids: Vec<_> = filtered_inbox.iter().map(|item| item.job.id.as_str()).collect();
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
}
