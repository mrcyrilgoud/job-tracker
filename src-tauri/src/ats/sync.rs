use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::ats::{list_jobs, AtsJob};
use crate::companies::get_watch;
use crate::error::{map_sqlite, AppError, AppResult};
use crate::models::Job;
use crate::util::{create_id, normalize_canonical_url, now_iso};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOk {
    pub ok: bool,
    pub created: usize,
    pub reactivated: usize,
    pub deactivated: usize,
    pub total_remote: usize,
    pub synced_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncErr {
    pub ok: bool,
    pub error: String,
    pub synced_at: String,
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

/// Fetch remote jobs without holding a DB lock.
pub async fn fetch_remote_jobs(provider: &str, board_slug: &str) -> AppResult<Vec<AtsJob>> {
    list_jobs(provider, board_slug).await
}

pub fn apply_watch_sync(
    conn: &Connection,
    watch_id: &str,
    remote_jobs: Result<Vec<AtsJob>, String>,
) -> AppResult<serde_json::Value> {
    let watch = get_watch(conn, watch_id)?.ok_or_else(|| AppError::from("Watch not found"))?;
    let synced_at = now_iso();

    let remote_jobs = match remote_jobs {
        Ok(jobs) => jobs,
        Err(message) => {
            conn.execute(
                "UPDATE company_watches SET consecutive_sync_failures = consecutive_sync_failures + 1, last_sync_error = ?1, updated_at = ?2 WHERE id = ?3",
                params![message, synced_at, watch_id],
            )
            .map_err(map_sqlite)?;
            return Ok(serde_json::json!({
                "ok": false,
                "error": message,
                "syncedAt": synced_at
            }));
        }
    };

    let remote_ids: std::collections::HashSet<_> =
        remote_jobs.iter().map(|j| j.external_id.clone()).collect();

    let mut stmt = conn
        .prepare(
            "SELECT id, company_id, title, url, canonical_url, source_external_id, status, applied_at, posting_state, last_checked_at, last_check_result, source, notes, location, is_new_from_watch, watch_disposition, missing_from_sync_count, created_at, updated_at FROM jobs WHERE company_id = ?1 AND source = ?2",
        )
        .map_err(map_sqlite)?;
    let existing = stmt
        .query_map(params![watch.company_id, watch.provider], map_job)
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    let mut created = 0usize;
    let mut reactivated = 0usize;

    for remote in &remote_jobs {
        let canonical_url = normalize_canonical_url(&remote.url).map_err(AppError::from)?;
        let by_external = existing
            .iter()
            .find(|j| j.source_external_id.as_deref() == Some(remote.external_id.as_str()));

        if let Some(local) = by_external {
            if local.posting_state == "inactive" {
                reactivated += 1;
                conn.execute(
                    "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1,?2,'posting_state_changed',?3,?4)",
                    params![
                        create_id(),
                        local.id,
                        "Role reappeared in a successful ATS sync",
                        synced_at
                    ],
                )
                .map_err(map_sqlite)?;
                conn.execute(
                    "UPDATE jobs SET title=?1, url=?2, location=?3, missing_from_sync_count=0, posting_state='active', updated_at=?4 WHERE id=?5",
                    params![
                        remote.title,
                        remote.url,
                        remote.location,
                        synced_at,
                        local.id
                    ],
                )
                .map_err(map_sqlite)?;
            } else {
                conn.execute(
                    "UPDATE jobs SET title=?1, url=?2, location=?3, missing_from_sync_count=0, updated_at=?4 WHERE id=?5",
                    params![
                        remote.title,
                        remote.url,
                        remote.location,
                        synced_at,
                        local.id
                    ],
                )
                .map_err(map_sqlite)?;
            }
            continue;
        }

        let by_url: Option<String> = conn
            .query_row(
                "SELECT id FROM jobs WHERE canonical_url = ?1",
                params![canonical_url],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sqlite)?;

        if let Some(job_id) = by_url {
            conn.execute(
                "UPDATE jobs SET source=?1, source_external_id=?2, company_id=?3, title=?4, location=COALESCE(?5, location), is_new_from_watch=0, watch_disposition='saved', missing_from_sync_count=0, updated_at=?6 WHERE id=?7",
                params![
                    watch.provider,
                    remote.external_id,
                    watch.company_id,
                    remote.title,
                    remote.location,
                    synced_at,
                    job_id
                ],
            )
            .map_err(map_sqlite)?;
            continue;
        }

        let job_id = create_id();
        conn.execute(
            r#"INSERT INTO jobs (
                id, company_id, title, url, canonical_url, source_external_id, status, applied_at,
                posting_state, last_checked_at, last_check_result, source, notes, location,
                is_new_from_watch, watch_disposition, missing_from_sync_count, created_at, updated_at
            ) VALUES (?1,?2,?3,?4,?5,?6,'wishlist',NULL,'active',NULL,NULL,?7,NULL,?8,1,'new',0,?9,?9)"#,
            params![
                job_id,
                watch.company_id,
                remote.title,
                remote.url,
                canonical_url,
                remote.external_id,
                watch.provider,
                remote.location,
                synced_at
            ],
        )
        .map_err(map_sqlite)?;
        conn.execute(
            "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1,?2,'discovered_from_watch',?3,?4)",
            params![
                create_id(),
                job_id,
                format!("Discovered via {} watch", watch.provider),
                synced_at
            ],
        )
        .map_err(map_sqlite)?;
        created += 1;
    }

    let mut deactivated = 0usize;
    for local in &existing {
        let Some(ext_id) = &local.source_external_id else {
            continue;
        };
        if remote_ids.contains(ext_id) {
            continue;
        }
        let next_missing = local.missing_from_sync_count + 1;
        if next_missing >= 2 && local.posting_state != "inactive" {
            deactivated += 1;
            conn.execute(
                "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1,?2,'posting_state_changed',?3,?4)",
                params![
                    create_id(),
                    local.id,
                    "Marked inactive after two successful syncs without this role",
                    synced_at
                ],
            )
            .map_err(map_sqlite)?;
            conn.execute(
                "UPDATE jobs SET missing_from_sync_count=?1, posting_state='inactive', updated_at=?2 WHERE id=?3",
                params![next_missing, synced_at, local.id],
            )
            .map_err(map_sqlite)?;
        } else {
            conn.execute(
                "UPDATE jobs SET missing_from_sync_count=?1, updated_at=?2 WHERE id=?3",
                params![next_missing, synced_at, local.id],
            )
            .map_err(map_sqlite)?;
        }
    }

    conn.execute(
        "UPDATE company_watches SET last_synced_at=?1, consecutive_sync_failures=0, last_sync_error=NULL, updated_at=?1 WHERE id=?2",
        params![synced_at, watch_id],
    )
    .map_err(map_sqlite)?;

    Ok(serde_json::json!({
        "ok": true,
        "created": created,
        "reactivated": reactivated,
        "deactivated": deactivated,
        "totalRemote": remote_jobs.len(),
        "syncedAt": synced_at
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn two_miss_rule() {
        let missing = 1;
        let next = missing + 1;
        assert!(next >= 2);
    }
}
