use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::instant::LatestTimestamp;
use crate::jobs::service::find_or_create_company;
use crate::models::{CareersPageReview, Company, CompanyWatch};
use crate::util::{create_id, now_iso};

fn map_watch(row: &rusqlite::Row<'_>) -> rusqlite::Result<CompanyWatch> {
    Ok(CompanyWatch {
        id: row.get(0)?,
        company_id: row.get(1)?,
        provider: row.get(2)?,
        board_slug: row.get(3)?,
        last_synced_at: row.get(4)?,
        consecutive_sync_failures: row.get(5)?,
        last_sync_error: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

pub fn list_companies(conn: &Connection) -> AppResult<Vec<Company>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, careers_url, created_at, updated_at FROM companies ORDER BY name",
        )
        .map_err(map_sqlite)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Company {
                id: row.get(0)?,
                name: row.get(1)?,
                careers_url: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;
    Ok(rows)
}

pub fn create_company(
    conn: &Connection,
    name: &str,
    careers_url: Option<&str>,
) -> AppResult<Company> {
    find_or_create_company(conn, name.trim(), careers_url)
}

pub fn insert_watch(
    conn: &Connection,
    company_id: &str,
    provider: &str,
    board_slug: &str,
) -> AppResult<CompanyWatch> {
    let company_exists: Option<String> = conn
        .query_row(
            "SELECT id FROM companies WHERE id = ?1",
            params![company_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    if company_exists.is_none() {
        return Err(AppError::from("Company not found"));
    }
    let provider = provider.trim().to_ascii_lowercase();
    let board_slug = board_slug.trim().to_ascii_lowercase();
    if let Some(existing) = conn
        .query_row(
            "SELECT id, company_id, provider, board_slug, last_synced_at, consecutive_sync_failures, last_sync_error, created_at, updated_at
             FROM company_watches WHERE company_id = ?1 AND provider = ?2 AND board_slug = ?3",
            params![company_id, provider, board_slug],
            map_watch,
        )
        .optional()
        .map_err(map_sqlite)?
    {
        return Ok(existing);
    }

    let timestamp = now_iso();
    let watch = CompanyWatch {
        id: create_id(),
        company_id: company_id.to_string(),
        provider,
        board_slug,
        last_synced_at: None,
        consecutive_sync_failures: 0,
        last_sync_error: None,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    };
    conn.execute(
        "INSERT INTO company_watches (id, company_id, provider, board_slug, last_synced_at, consecutive_sync_failures, last_sync_error, created_at, updated_at) VALUES (?1,?2,?3,?4,NULL,0,NULL,?5,?5)",
        params![
            watch.id,
            watch.company_id,
            watch.provider,
            watch.board_slug,
            watch.created_at
        ],
    )
    .map_err(map_sqlite)?;
    Ok(watch)
}

/// Pipeline_Job: non-archived, triaged (not new from watch), not dismissed.
/// Shared by all pipeline aggregate queries; callers alias `jobs` as `j`.
const PIPELINE_JOB_PREDICATE: &str = "j.status <> 'archived' \
    AND j.is_new_from_watch = 0 \
    AND (j.watch_disposition IS NULL OR j.watch_disposition <> 'dismissed')";

/// Query A output for one company: wishlist, applied, interviewing counts,
/// then the guarded application_ease sum and count over wishlist jobs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PipelineCountRow {
    wishlist: i64,
    applied: i64,
    interviewing: i64,
    ease_sum: i64,
    ease_count: i64,
}

/// Per-company pipeline aggregates attached to each `CompanyRow`.
#[derive(Debug, Default, Clone)]
struct CompanyPipelineAggregates {
    wishlist: i64,
    applied: i64,
    interviewing: i64,
    ease_sum: i64,
    ease_count: i64,
    last_activity: LatestTimestamp,
    last_applied: LatestTimestamp,
}

impl CompanyPipelineAggregates {
    /// Wishlist_Ease_Average, unrounded; None when no wishlist job has ease 1–5.
    fn wishlist_ease_average(&self) -> Option<f64> {
        (self.ease_count > 0).then(|| self.ease_sum as f64 / self.ease_count as f64)
    }
}

/// Pure fold of the three grouped query results into per-company aggregates.
/// Timestamp candidates go through `LatestTimestamp`, so invalid strings are
/// ignored and the result does not depend on row order.
fn fold_pipeline_aggregates(
    counts: impl IntoIterator<Item = (String, PipelineCountRow)>,
    applied_pairs: impl IntoIterator<Item = (String, String)>,
    activity_pairs: impl IntoIterator<Item = (String, String)>,
) -> HashMap<String, CompanyPipelineAggregates> {
    let mut aggregates: HashMap<String, CompanyPipelineAggregates> = HashMap::new();
    for (company_id, row) in counts {
        let entry = aggregates.entry(company_id).or_default();
        entry.wishlist += row.wishlist;
        entry.applied += row.applied;
        entry.interviewing += row.interviewing;
        entry.ease_sum += row.ease_sum;
        entry.ease_count += row.ease_count;
    }
    for (company_id, applied_at) in applied_pairs {
        aggregates
            .entry(company_id)
            .or_default()
            .last_applied
            .offer(&applied_at);
    }
    for (company_id, occurred_at) in activity_pairs {
        aggregates
            .entry(company_id)
            .or_default()
            .last_activity
            .offer(&occurred_at);
    }
    aggregates
}

/// Runs Query A (counts + ease), Query B (applied_at), and Query C (user
/// activity occurred_at) and folds them. Reads only.
fn load_pipeline_aggregates(
    conn: &Connection,
) -> AppResult<HashMap<String, CompanyPipelineAggregates>> {
    let counts_sql = format!(
        "SELECT j.company_id,
            SUM(j.status = 'wishlist'),
            SUM(j.status IN ('applied','interviewing','offer','rejected','withdrawn')
                OR (j.status = 'closed' AND j.applied_at IS NOT NULL)),
            SUM(j.status IN ('interviewing','offer')),
            SUM(CASE WHEN j.status = 'wishlist' AND typeof(j.application_ease) = 'integer'
                      AND j.application_ease BETWEEN 1 AND 5 THEN j.application_ease ELSE 0 END),
            SUM(CASE WHEN j.status = 'wishlist' AND typeof(j.application_ease) = 'integer'
                      AND j.application_ease BETWEEN 1 AND 5 THEN 1 ELSE 0 END)
         FROM jobs j
         WHERE {PIPELINE_JOB_PREDICATE}
         GROUP BY j.company_id"
    );
    let mut counts_stmt = conn.prepare(&counts_sql).map_err(map_sqlite)?;
    let counts = counts_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                PipelineCountRow {
                    wishlist: row.get(1)?,
                    applied: row.get(2)?,
                    interviewing: row.get(3)?,
                    ease_sum: row.get(4)?,
                    ease_count: row.get(5)?,
                },
            ))
        })
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    let applied_sql = format!(
        "SELECT j.company_id, j.applied_at FROM jobs j
         WHERE {PIPELINE_JOB_PREDICATE} AND j.applied_at IS NOT NULL"
    );
    let mut applied_stmt = conn.prepare(&applied_sql).map_err(map_sqlite)?;
    let applied_rows = applied_stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(map_sqlite)?;

    let activity_sql = format!(
        "SELECT DISTINCT j.company_id, e.occurred_at
         FROM job_events e JOIN jobs j ON j.id = e.job_id
         WHERE {PIPELINE_JOB_PREDICATE}
           AND e.type NOT IN ('posting_state_changed', 'discovered_from_watch')"
    );
    let mut activity_stmt = conn.prepare(&activity_sql).map_err(map_sqlite)?;
    let activity_rows = activity_stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(map_sqlite)?;

    // Stream B and C rows into the fold so memory is bounded by the number of
    // companies, not events. A row error stops its stream and is returned.
    let mut applied_error: Option<rusqlite::Error> = None;
    let mut activity_error: Option<rusqlite::Error> = None;
    let aggregates = fold_pipeline_aggregates(
        counts,
        applied_rows.map_while(|item| item.map_err(|e| applied_error = Some(e)).ok()),
        activity_rows.map_while(|item| item.map_err(|e| activity_error = Some(e)).ok()),
    );
    if let Some(err) = applied_error.or(activity_error) {
        return Err(map_sqlite(err));
    }
    Ok(aggregates)
}

pub fn list_companies_with_watches(conn: &Connection) -> AppResult<Vec<serde_json::Value>> {
    // Deferred read transaction: every query sees one snapshot even if the
    // hourly jobs runner (separate process) writes concurrently. Dropped
    // without commit; nothing here writes.
    let tx = conn.unchecked_transaction().map_err(map_sqlite)?;
    let conn: &Connection = &tx;
    let companies = list_companies(conn)?;
    let mut watches_stmt = conn
        .prepare(
            "SELECT id, company_id, provider, board_slug, last_synced_at, consecutive_sync_failures, last_sync_error, created_at, updated_at FROM company_watches",
        )
        .map_err(map_sqlite)?;
    let watches = watches_stmt
        .query_map([], map_watch)
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    let mut reviews_stmt = conn
        .prepare(
            "SELECT id, company_id, previous_hash, current_hash, summary, status, created_at FROM careers_page_reviews WHERE status = 'pending' ORDER BY created_at DESC",
        )
        .map_err(map_sqlite)?;
    let reviews = reviews_stmt
        .query_map([], |row| {
            Ok(CareersPageReview {
                id: row.get(0)?,
                company_id: row.get(1)?,
                previous_hash: row.get(2)?,
                current_hash: row.get(3)?,
                summary: row.get(4)?,
                status: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    let mut counts_stmt = conn
        .prepare(
            "SELECT company_id, COUNT(*) FROM jobs WHERE posting_state = 'active' AND source IN ('greenhouse', 'lever', 'ashby') GROUP BY company_id",
        )
        .map_err(map_sqlite)?;
    let open_position_counts: HashMap<String, i64> = counts_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(map_sqlite)?
        .collect::<Result<_, _>>()
        .map_err(map_sqlite)?;

    // Tracked_Job_Count: every non-archived job for the company, any source,
    // posting_state, or favorite flag (closed jobs count).
    let mut tracked_stmt = conn
        .prepare(
            "SELECT company_id, COUNT(*) FROM jobs WHERE status <> 'archived' GROUP BY company_id",
        )
        .map_err(map_sqlite)?;
    let tracked_job_counts: HashMap<String, i64> = tracked_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(map_sqlite)?
        .collect::<Result<_, _>>()
        .map_err(map_sqlite)?;

    let mut pipeline = load_pipeline_aggregates(conn)?;

    Ok(companies
        .into_iter()
        .map(|company| {
            let company_watches: Vec<_> = watches
                .iter()
                .filter(|w| w.company_id == company.id)
                .cloned()
                .collect();
            let company_reviews: Vec<_> = reviews
                .iter()
                .filter(|r| r.company_id == company.id)
                .cloned()
                .collect();
            let open_position_count = open_position_counts.get(&company.id).copied().unwrap_or(0);
            let tracked_job_count = tracked_job_counts.get(&company.id).copied().unwrap_or(0);
            let agg = pipeline.remove(&company.id).unwrap_or_default();
            let ease_average = agg.wishlist_ease_average();
            serde_json::json!({
                "company": company,
                "watches": company_watches,
                "reviews": company_reviews,
                "openPositionCount": open_position_count,
                "trackedJobCount": tracked_job_count,
                "wishlistJobCount": agg.wishlist,
                "appliedJobCount": agg.applied,
                "interviewingJobCount": agg.interviewing,
                "lastActivityAt": agg.last_activity.into_value(),
                "lastAppliedAt": agg.last_applied.into_value(),
                "wishlistEaseAverage": ease_average
            })
        })
        .collect())
}

pub fn dismiss_careers_review(conn: &Connection, review_id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE careers_page_reviews SET status = 'dismissed' WHERE id = ?1",
        params![review_id],
    )
    .map_err(map_sqlite)?;
    Ok(())
}

pub fn delete_watch(conn: &Connection, watch_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM company_watches WHERE id = ?1",
        params![watch_id],
    )
    .map_err(map_sqlite)?;
    Ok(())
}

pub fn get_watch(conn: &Connection, watch_id: &str) -> AppResult<Option<CompanyWatch>> {
    conn.query_row(
        "SELECT id, company_id, provider, board_slug, last_synced_at, consecutive_sync_failures, last_sync_error, created_at, updated_at FROM company_watches WHERE id = ?1",
        params![watch_id],
        |row| {
            Ok(CompanyWatch {
                id: row.get(0)?,
                company_id: row.get(1)?,
                provider: row.get(2)?,
                board_slug: row.get(3)?,
                last_synced_at: row.get(4)?,
                consecutive_sync_failures: row.get(5)?,
                last_sync_error: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        },
    )
    .optional()
    .map_err(map_sqlite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;

    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        connection
    }

    #[test]
    fn insert_watch_is_idempotent_for_company_provider_and_slug() {
        let connection = test_connection();
        let company = create_company(&connection, "Acme", None).unwrap();

        let first = insert_watch(&connection, &company.id, " Greenhouse ", " Acme ").unwrap();
        let second = insert_watch(&connection, &company.id, "greenhouse", "acme").unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(first.provider, "greenhouse");
        assert_eq!(first.board_slug, "acme");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM company_watches", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn insert_watch_rejects_unknown_company_without_writing() {
        let connection = test_connection();
        let error = insert_watch(&connection, "missing", "ashby", "acme").unwrap_err();

        assert_eq!(error.to_string(), "Company not found");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM company_watches", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn create_company_reuses_selected_company_when_confirming_careers_page() {
        let connection = test_connection();
        let first = create_company(&connection, "Acme", None).unwrap();
        let second =
            create_company(&connection, " Acme ", Some("https://acme.example/careers")).unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(
            second.careers_url.as_deref(),
            Some("https://acme.example/careers")
        );
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM companies", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    /// **Validates: Requirements 4.7**
    #[test]
    fn list_companies_with_watches_errors_when_jobs_table_is_missing() {
        let connection = test_connection();
        create_company(&connection, "Acme", None).unwrap();
        // Disable FK enforcement so dependent tables don't block the drop.
        connection
            .execute_batch("PRAGMA foreign_keys = OFF; DROP TABLE jobs;")
            .unwrap();

        assert!(list_companies_with_watches(&connection).is_err());
    }

    /// Sanity example for the six pipeline fields (companies-tab-sorting 2.1).
    #[test]
    fn list_companies_with_watches_reports_pipeline_aggregates() {
        let connection = test_connection();
        connection
            .execute_batch(
                "INSERT INTO companies (id, name, careers_url, created_at, updated_at) VALUES
                   ('c1', 'Acme', NULL, '2026-01-01', '2026-01-01'),
                   ('c2', 'Beta', NULL, '2026-01-01', '2026-01-01');
                 INSERT INTO jobs (id, company_id, title, url, canonical_url, status, applied_at,
                   application_ease, is_new_from_watch, watch_disposition, created_at, updated_at) VALUES
                   ('w1', 'c1', 'W1', 'u1', 'u1', 'wishlist', NULL, 5, 0, NULL, 't', 't'),
                   ('w2', 'c1', 'W2', 'u2', 'u2', 'wishlist', NULL, 4, 0, 'saved', 't', 't'),
                   ('w3', 'c1', 'W3', 'u3', 'u3', 'wishlist', NULL, NULL, 0, NULL, 't', 't'),
                   ('a1', 'c1', 'A1', 'u4', 'u4', 'interviewing', '2026-10-01T09:00:00+09:00', NULL, 0, NULL, 't', 't'),
                   ('a2', 'c1', 'A2', 'u5', 'u5', 'closed', '2026-10-01T01:00:00Z', NULL, 0, NULL, 't', 't'),
                   ('a3', 'c1', 'A3', 'u6', 'u6', 'rejected', 'pending', NULL, 0, NULL, 't', 't'),
                   ('x1', 'c1', 'X1', 'u7', 'u7', 'applied', '2030-01-01', NULL, 0, 'dismissed', 't', 't'),
                   ('x2', 'c1', 'X2', 'u8', 'u8', 'wishlist', NULL, 1, 1, 'new', 't', 't'),
                   ('x3', 'c1', 'X3', 'u9', 'u9', 'archived', '2030-01-01', NULL, 0, NULL, 't', 't'),
                   ('c0', 'c1', 'C0', 'u10', 'u10', 'closed', NULL, NULL, 0, NULL, 't', 't');
                 INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES
                   ('e1', 'w1', 'created', NULL, '2026-09-01T00:00:00.000Z'),
                   ('e2', 'a1', 'status_changed', NULL, '2026-09-15'),
                   ('e3', 'a1', 'posting_state_changed', NULL, '2030-01-01T00:00:00Z'),
                   ('e4', 'w2', 'discovered_from_watch', NULL, '2030-01-01T00:00:00Z'),
                   ('e5', 'x1', 'note_added', NULL, '2030-01-01T00:00:00Z');",
            )
            .unwrap();

        let rows = list_companies_with_watches(&connection).unwrap();
        assert_eq!(rows.len(), 2);
        let acme = &rows[0];
        assert_eq!(acme["company"]["id"], "c1");
        assert_eq!(acme["wishlistJobCount"], 3);
        assert_eq!(acme["appliedJobCount"], 3);
        assert_eq!(acme["interviewingJobCount"], 1);
        assert_eq!(acme["wishlistEaseAverage"], 4.5);
        // Equal-or-later instant: 01:00Z is later than 09:00+09:00 (= 00:00Z).
        assert_eq!(acme["lastAppliedAt"], "2026-10-01T01:00:00Z");
        assert_eq!(acme["lastActivityAt"], "2026-09-15");

        let beta = &rows[1];
        assert_eq!(beta["wishlistJobCount"], 0);
        assert_eq!(beta["appliedJobCount"], 0);
        assert_eq!(beta["interviewingJobCount"], 0);
        assert!(beta["lastActivityAt"].is_null());
        assert!(beta["lastAppliedAt"].is_null());
        assert!(beta["wishlistEaseAverage"].is_null());
    }

    fn table_count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    /// An aggregate query failure returns Err and writes nothing.
    /// **Validates: Requirements 1.10**
    #[test]
    fn list_companies_with_watches_errors_without_writes_when_job_events_missing() {
        let connection = test_connection();
        connection
            .execute_batch(
                "INSERT INTO companies (id, name, careers_url, created_at, updated_at) VALUES
                   ('c1', 'Acme', NULL, '2026-01-01', '2026-01-01');
                 INSERT INTO jobs (id, company_id, title, url, canonical_url, status, applied_at,
                   application_ease, is_new_from_watch, watch_disposition, created_at, updated_at) VALUES
                   ('j1', 'c1', 'J1', 'u1', 'u1', 'applied', '2026-10-01', NULL, 0, NULL, 't', 't');
                 PRAGMA foreign_keys = OFF;
                 DROP TABLE job_events;",
            )
            .unwrap();
        let jobs_before = table_count(&connection, "jobs");
        let companies_before = table_count(&connection, "companies");

        assert!(list_companies_with_watches(&connection).is_err());
        assert_eq!(table_count(&connection, "jobs"), jobs_before);
        assert_eq!(table_count(&connection, "companies"), companies_before);
    }

    /// Archived, dismissed, and unreviewed watch jobs are not Pipeline_Jobs.
    /// **Validates: Requirements 1.6**
    #[test]
    fn company_with_only_non_pipeline_jobs_reports_zeros_and_nulls() {
        let connection = test_connection();
        connection
            .execute_batch(
                "INSERT INTO companies (id, name, careers_url, created_at, updated_at) VALUES
                   ('c1', 'Acme', NULL, '2026-01-01', '2026-01-01');
                 INSERT INTO jobs (id, company_id, title, url, canonical_url, status, applied_at,
                   application_ease, is_new_from_watch, watch_disposition, created_at, updated_at) VALUES
                   ('x1', 'c1', 'X1', 'u1', 'u1', 'archived', '2026-10-01', 5, 0, NULL, 't', 't'),
                   ('x2', 'c1', 'X2', 'u2', 'u2', 'wishlist', NULL, 4, 0, 'dismissed', 't', 't'),
                   ('x3', 'c1', 'X3', 'u3', 'u3', 'interviewing', '2026-10-01', 3, 1, 'new', 't', 't');
                 INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES
                   ('e1', 'x1', 'status_changed', NULL, '2026-10-01T00:00:00Z'),
                   ('e2', 'x2', 'note_added', NULL, '2026-10-01T00:00:00Z'),
                   ('e3', 'x3', 'created', NULL, '2026-10-01T00:00:00Z');",
            )
            .unwrap();

        let rows = list_companies_with_watches(&connection).unwrap();
        assert_eq!(rows.len(), 1);
        let acme = &rows[0];
        assert_eq!(acme["wishlistJobCount"], 0);
        assert_eq!(acme["appliedJobCount"], 0);
        assert_eq!(acme["interviewingJobCount"], 0);
        assert!(acme["lastActivityAt"].is_null());
        assert!(acme["lastAppliedAt"].is_null());
        assert!(acme["wishlistEaseAverage"].is_null());
    }

    /// `closed` counts as applied only when `applied_at` is set.
    /// **Validates: Requirements 1.6**
    #[test]
    fn closed_job_counts_as_applied_only_with_applied_at() {
        let connection = test_connection();
        connection
            .execute_batch(
                "INSERT INTO companies (id, name, careers_url, created_at, updated_at) VALUES
                   ('c1', 'Acme', NULL, '2026-01-01', '2026-01-01'),
                   ('c2', 'Beta', NULL, '2026-01-01', '2026-01-01');
                 INSERT INTO jobs (id, company_id, title, url, canonical_url, status, applied_at,
                   application_ease, is_new_from_watch, watch_disposition, created_at, updated_at) VALUES
                   ('j1', 'c1', 'J1', 'u1', 'u1', 'closed', '2026-09-28', NULL, 0, NULL, 't', 't'),
                   ('j2', 'c2', 'J2', 'u2', 'u2', 'closed', NULL, 3, 0, NULL, 't', 't');",
            )
            .unwrap();

        let rows = list_companies_with_watches(&connection).unwrap();
        let acme = &rows[0];
        assert_eq!(acme["company"]["id"], "c1");
        assert_eq!(acme["appliedJobCount"], 1);
        assert_eq!(acme["wishlistJobCount"], 0);
        assert_eq!(acme["interviewingJobCount"], 0);
        assert_eq!(acme["lastAppliedAt"], "2026-09-28");

        let beta = &rows[1];
        assert_eq!(beta["company"]["id"], "c2");
        assert_eq!(beta["appliedJobCount"], 0);
        assert_eq!(beta["wishlistJobCount"], 0);
        assert_eq!(beta["interviewingJobCount"], 0);
        assert!(beta["lastAppliedAt"].is_null());
        assert!(beta["wishlistEaseAverage"].is_null());
    }

    /// Background `posting_state_changed` events are not user activity.
    /// **Validates: Requirements 1.12**
    #[test]
    fn later_posting_state_changed_event_leaves_last_activity_unchanged() {
        let connection = test_connection();
        connection
            .execute_batch(
                "INSERT INTO companies (id, name, careers_url, created_at, updated_at) VALUES
                   ('c1', 'Acme', NULL, '2026-01-01', '2026-01-01');
                 INSERT INTO jobs (id, company_id, title, url, canonical_url, status, applied_at,
                   application_ease, is_new_from_watch, watch_disposition, created_at, updated_at) VALUES
                   ('j1', 'c1', 'J1', 'u1', 'u1', 'applied', NULL, NULL, 0, NULL, 't', 't');
                 INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES
                   ('e1', 'j1', 'status_changed', NULL, '2026-09-15T12:00:00.000Z');",
            )
            .unwrap();
        let before = list_companies_with_watches(&connection).unwrap();
        assert_eq!(before[0]["lastActivityAt"], "2026-09-15T12:00:00.000Z");

        connection
            .execute(
                "INSERT INTO job_events (id, job_id, type, note, occurred_at)
                 VALUES ('e2', 'j1', 'posting_state_changed', NULL, '2026-10-20T00:00:00.000Z')",
                [],
            )
            .unwrap();
        let after = list_companies_with_watches(&connection).unwrap();
        assert_eq!(after[0]["lastActivityAt"], "2026-09-15T12:00:00.000Z");
    }

    // =====================================================================
    // Property tests: list_companies_with_watches aggregates
    // =====================================================================

    use crate::models::JOB_STATUSES;
    use proptest::prelude::*;

    /// Sources in the generation domain: the three ATS providers that feed
    /// Open_Position_Count plus non-ATS sources that must not.
    const JOB_SOURCES: &[&str] = &["greenhouse", "lever", "ashby", "manual", "careers_page"];
    const POSTING_STATES: &[&str] = &["active", "inactive", "unknown"];
    /// Small name pool with mixed case and duplicates so company ordering
    /// (`ORDER BY name`) sees ties and case differences.
    const COMPANY_NAMES: &[&str] = &["Acme", "acme", "Beta", "beta", "Zeta", "Ábaco"];

    #[derive(Debug, Clone)]
    struct JobSpec {
        /// Index into the generated companies (reduced modulo their count).
        company_index: usize,
        status: &'static str,
        source: &'static str,
        posting_state: &'static str,
        is_favorite: bool,
        applied_at: Option<&'static str>,
        application_ease: Option<i64>,
        is_new_from_watch: bool,
        watch_disposition: Option<&'static str>,
        /// `job_events` rows for this job as (type, occurred_at).
        events: Vec<(&'static str, &'static str)>,
    }

    /// Hand-written timestamp pool: (stored string, expected instant as
    /// (epoch seconds, sub-second nanos)) or None when not a Valid_Timestamp.
    /// The model uses these expected instants and never calls the parser.
    /// Includes equal instants written differently (tie rule, Req 1.4) and
    /// invalid strings that sort after every date by bytes (`MAX()` trap).
    /// 2026-10-01T00:00:00Z = 1_790_812_800.
    const TIMESTAMP_POOL: &[(&str, Option<(i64, u32)>)] = &[
        ("2026-10-01", Some((1_790_812_800, 0))),
        ("2026-10-01T00:00:00Z", Some((1_790_812_800, 0))),
        ("2026-10-01T00:00:00.000Z", Some((1_790_812_800, 0))),
        ("2026-10-01T09:00:00+09:00", Some((1_790_812_800, 0))),
        ("2026-10-01T00:00:00.5Z", Some((1_790_812_800, 500_000_000))),
        ("2026-10-01T00:00:01Z", Some((1_790_812_801, 0))),
        ("2026-10-01T01:00:00Z", Some((1_790_816_400, 0))),
        ("2026-10-01 01:00", Some((1_790_816_400, 0))),
        ("2026-09-30T20:00:00-05:00", Some((1_790_816_400, 0))),
        ("2026-09-15", Some((1_789_430_400, 0))),
        ("2025-12-31T23:59:59.999Z", Some((1_767_225_599, 999_000_000))),
        ("pending", None),
        ("now", None),
        ("2026-02-30", None),
        ("9999-13-01", None),
        (" 2026-10-01", None),
    ];

    fn expected_instant(value: &str) -> Option<(i64, u32)> {
        TIMESTAMP_POOL
            .iter()
            .find(|(raw, _)| *raw == value)
            .expect("timestamp outside the pool")
            .1
    }

    /// Model of Req 1.3/1.4: latest valid instant, ties by greatest string bytes.
    fn model_latest<'a>(candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
        candidates
            .filter_map(|raw| expected_instant(raw).map(|instant| (instant, raw)))
            .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.as_bytes().cmp(b.1.as_bytes())))
            .map(|(_, raw)| raw)
    }

    fn timestamp_strategy() -> impl Strategy<Value = &'static str> {
        prop::sample::select(TIMESTAMP_POOL).prop_map(|(raw, _)| raw)
    }
    /// User-activity types plus the two background types that must be ignored.
    const EVENT_TYPES: &[&str] = &[
        "created",
        "status_changed",
        "note_added",
        "posting_state_changed",
        "discovered_from_watch",
    ];
    const BACKGROUND_EVENT_TYPES: &[&str] = &["posting_state_changed", "discovered_from_watch"];
    const WATCH_DISPOSITIONS: &[&str] = &["new", "saved", "dismissed"];

    impl JobSpec {
        /// Pipeline_Job per the companies-tab-sorting glossary.
        fn is_pipeline(&self) -> bool {
            self.status != "archived"
                && !self.is_new_from_watch
                && self.watch_disposition != Some("dismissed")
        }
        fn is_wishlist(&self) -> bool {
            self.is_pipeline() && self.status == "wishlist"
        }
        fn is_applied(&self) -> bool {
            self.is_pipeline()
                && (matches!(
                    self.status,
                    "applied" | "interviewing" | "offer" | "rejected" | "withdrawn"
                ) || (self.status == "closed" && self.applied_at.is_some()))
        }
        fn is_interviewing(&self) -> bool {
            self.is_pipeline() && matches!(self.status, "interviewing" | "offer")
        }
    }

    #[derive(Debug, Clone)]
    struct CompaniesFixture {
        company_names: Vec<&'static str>,
        jobs: Vec<JobSpec>,
    }

    struct SeededFixture {
        /// Company ids, parallel to `CompaniesFixture::company_names`.
        company_ids: Vec<String>,
        /// Each inserted job as (company_id, spec). Jobs are dropped when
        /// there are no companies, since `jobs.company_id` is NOT NULL.
        jobs: Vec<(String, JobSpec)>,
    }

    fn job_spec_strategy() -> impl Strategy<Value = JobSpec> {
        (
            0usize..8,
            prop::sample::select(JOB_STATUSES),
            prop::sample::select(JOB_SOURCES),
            prop::sample::select(POSTING_STATES),
            any::<bool>(),
            prop::option::of(timestamp_strategy()),
            prop::option::of(1i64..=5),
            any::<bool>(),
            prop::option::of(prop::sample::select(WATCH_DISPOSITIONS)),
            prop::collection::vec(
                (
                    prop::sample::select(EVENT_TYPES),
                    timestamp_strategy(),
                ),
                0..=3,
            ),
        )
            .prop_map(
                |(
                    company_index,
                    status,
                    source,
                    posting_state,
                    is_favorite,
                    applied_at,
                    application_ease,
                    is_new_from_watch,
                    watch_disposition,
                    events,
                )| JobSpec {
                    company_index,
                    status,
                    source,
                    posting_state,
                    is_favorite,
                    applied_at,
                    application_ease,
                    is_new_from_watch,
                    watch_disposition,
                    events,
                },
            )
    }

    /// 0–8 companies and 0–40 jobs, each job with a random company, status
    /// (every Job_Status, including `archived` and `closed`), source,
    /// posting_state, and favorite flag.
    fn companies_fixture_strategy() -> impl Strategy<Value = CompaniesFixture> {
        (
            prop::collection::vec(prop::sample::select(COMPANY_NAMES), 0..=8),
            prop::collection::vec(job_spec_strategy(), 0..=40),
        )
            .prop_map(|(company_names, jobs)| CompaniesFixture {
                company_names,
                jobs,
            })
    }

    fn seed_fixture(conn: &Connection, fixture: &CompaniesFixture) -> SeededFixture {
        let timestamp = "2026-01-01T00:00:00.000Z";
        let company_ids: Vec<String> = fixture
            .company_names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let id = format!("company-{index}");
                conn.execute(
                    "INSERT INTO companies (id, name, careers_url, created_at, updated_at) VALUES (?1, ?2, NULL, ?3, ?3)",
                    params![id, name, timestamp],
                )
                .unwrap();
                id
            })
            .collect();

        let mut jobs = Vec::new();
        if !company_ids.is_empty() {
            for (index, spec) in fixture.jobs.iter().enumerate() {
                let company_id = company_ids[spec.company_index % company_ids.len()].clone();
                let url = format!("https://jobs.example/{index}");
                conn.execute(
                    "INSERT INTO jobs (id, company_id, title, url, canonical_url, status, posting_state, source, is_favorite, created_at, updated_at,
                       applied_at, application_ease, is_new_from_watch, watch_disposition)
                     VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        format!("job-{index}"),
                        company_id,
                        format!("Job {index}"),
                        url,
                        spec.status,
                        spec.posting_state,
                        spec.source,
                        spec.is_favorite,
                        timestamp,
                        spec.applied_at,
                        spec.application_ease,
                        spec.is_new_from_watch,
                        spec.watch_disposition
                    ],
                )
                .unwrap();
                for (event_index, (event_type, occurred_at)) in spec.events.iter().enumerate() {
                    conn.execute(
                        "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1, ?2, ?3, NULL, ?4)",
                        params![
                            format!("event-{index}-{event_index}"),
                            format!("job-{index}"),
                            event_type,
                            occurred_at
                        ],
                    )
                    .unwrap();
                }
                jobs.push((company_id, spec.clone()));
            }
        }
        SeededFixture { company_ids, jobs }
    }

    fn row_company_id(row: &serde_json::Value) -> String {
        row["company"]["id"].as_str().unwrap().to_string()
    }

    /// Generated watch: (company index, provider index). Slugs are unique per
    /// watch so the (company, provider, slug) unique index never collides.
    type WatchSpec = (usize, usize);
    /// Generated review: (company index, pending?).
    type ReviewSpec = (usize, bool);

    const WATCH_PROVIDERS: &[&str] = &["greenhouse", "lever", "ashby"];

    /// Seeds watches and careers-page reviews for the Property 4 test and
    /// returns them as (id, company_id) and (id, company_id, created_at, pending).
    fn seed_watches_and_reviews(
        conn: &Connection,
        company_ids: &[String],
        watches: &[WatchSpec],
        reviews: &[ReviewSpec],
    ) -> (Vec<(String, String)>, Vec<(String, String, String, bool)>) {
        if company_ids.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let watch_rows = watches
            .iter()
            .enumerate()
            .map(|(index, (company_index, provider_index))| {
                let id = format!("watch-{index}");
                let company_id = company_ids[company_index % company_ids.len()].clone();
                conn.execute(
                    "INSERT INTO company_watches (id, company_id, provider, board_slug, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    params![
                        id,
                        company_id,
                        WATCH_PROVIDERS[provider_index % WATCH_PROVIDERS.len()],
                        format!("slug-{index}"),
                        "2026-01-01T00:00:00.000Z"
                    ],
                )
                .unwrap();
                (id, company_id)
            })
            .collect();
        let review_rows = reviews
            .iter()
            .enumerate()
            .map(|(index, (company_index, pending))| {
                let id = format!("review-{index}");
                let company_id = company_ids[company_index % company_ids.len()].clone();
                // Distinct created_at values keep `ORDER BY created_at DESC` total.
                let created_at = format!("2026-01-01T00:00:{index:02}.000Z");
                conn.execute(
                    "INSERT INTO careers_page_reviews (id, company_id, previous_hash, current_hash, summary, status, created_at) VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6)",
                    params![
                        id,
                        company_id,
                        format!("hash-{index}"),
                        format!("Summary {index}"),
                        if *pending { "pending" } else { "dismissed" },
                        created_at
                    ],
                )
                .unwrap();
                (id, company_id, created_at, *pending)
            })
            .collect();
        (watch_rows, review_rows)
    }

    const ROW_KEYS: &[&str] = &[
        "company",
        "watches",
        "reviews",
        "openPositionCount",
        "trackedJobCount",
        "wishlistJobCount",
        "appliedJobCount",
        "interviewingJobCount",
        "lastActivityAt",
        "lastAppliedAt",
        "wishlistEaseAverage",
    ];

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: companies-sidebar-sort-filter, Property 12: Tracked job count matches the non-archived job model
        /// **Validates: Requirements 4.1, 4.2, 4.3, 4.4**
        #[test]
        fn prop_tracked_job_count_matches_non_archived_model(
            fixture in companies_fixture_strategy()
        ) {
            let conn = test_connection();
            let seeded = seed_fixture(&conn, &fixture);

            let rows = list_companies_with_watches(&conn).unwrap();
            prop_assert_eq!(rows.len(), seeded.company_ids.len());

            let mut sum = 0i64;
            for row in &rows {
                let company_id = row_company_id(row);
                let expected = seeded
                    .jobs
                    .iter()
                    .filter(|(id, spec)| *id == company_id && spec.status != "archived")
                    .count() as i64;
                let actual = row["trackedJobCount"].as_i64();
                prop_assert_eq!(actual, Some(expected), "company {}", company_id);
                sum += actual.unwrap_or(0);
            }

            let total_non_archived = seeded
                .jobs
                .iter()
                .filter(|(_, spec)| spec.status != "archived")
                .count() as i64;
            prop_assert_eq!(sum, total_non_archived);
        }

        // Feature: companies-sidebar-sort-filter, Property 13: Existing row fields and order are unchanged
        /// **Validates: Requirements 4.6**
        #[test]
        fn prop_existing_row_fields_and_order_unchanged(
            fixture in companies_fixture_strategy()
        ) {
            let conn = test_connection();
            let seeded = seed_fixture(&conn, &fixture);

            let rows = list_companies_with_watches(&conn).unwrap();
            let row_ids: Vec<String> = rows.iter().map(row_company_id).collect();
            let expected_ids: Vec<String> = list_companies(&conn)
                .unwrap()
                .into_iter()
                .map(|company| company.id)
                .collect();
            prop_assert_eq!(&row_ids, &expected_ids);

            for (row, company_id) in rows.iter().zip(&row_ids) {
                let expected = seeded
                    .jobs
                    .iter()
                    .filter(|(id, spec)| {
                        id == company_id
                            && spec.posting_state == "active"
                            && matches!(spec.source, "greenhouse" | "lever" | "ashby")
                    })
                    .count() as i64;
                prop_assert_eq!(
                    row["openPositionCount"].as_i64(),
                    Some(expected),
                    "company {}",
                    company_id
                );
            }
        }

        // Feature: companies-tab-sorting, Property 1: Pipeline counts and ease average match the job model
        /// **Validates: Requirements 1.1, 1.5, 1.6, 1.7, 1.8**
        #[test]
        fn prop_pipeline_counts_and_ease_average_match_job_model(
            fixture in companies_fixture_strategy()
        ) {
            let conn = test_connection();
            let seeded = seed_fixture(&conn, &fixture);

            let rows = list_companies_with_watches(&conn).unwrap();
            prop_assert_eq!(rows.len(), seeded.company_ids.len());

            for row in &rows {
                let company_id = row_company_id(row);
                let jobs: Vec<&JobSpec> = seeded
                    .jobs
                    .iter()
                    .filter(|(id, _)| *id == company_id)
                    .map(|(_, spec)| spec)
                    .collect();
                let pipeline = jobs.iter().filter(|j| j.is_pipeline()).count() as i64;
                let wishlist = jobs.iter().filter(|j| j.is_wishlist()).count() as i64;
                let applied = jobs.iter().filter(|j| j.is_applied()).count() as i64;
                let interviewing = jobs.iter().filter(|j| j.is_interviewing()).count() as i64;
                let eases: Vec<i64> = jobs
                    .iter()
                    .filter(|j| j.is_wishlist())
                    .filter_map(|j| j.application_ease)
                    .collect();

                let actual_wishlist = row["wishlistJobCount"].as_i64();
                let actual_applied = row["appliedJobCount"].as_i64();
                let actual_interviewing = row["interviewingJobCount"].as_i64();
                prop_assert_eq!(actual_wishlist, Some(wishlist), "wishlist {}", company_id);
                prop_assert_eq!(actual_applied, Some(applied), "applied {}", company_id);
                prop_assert_eq!(
                    actual_interviewing,
                    Some(interviewing),
                    "interviewing {}",
                    company_id
                );

                // Req 1.7, 1.8 invariants on the returned values.
                let (w, a, i) = (
                    actual_wishlist.unwrap(),
                    actual_applied.unwrap(),
                    actual_interviewing.unwrap(),
                );
                prop_assert!(w >= 0 && a >= 0 && i >= 0);
                prop_assert!(w + a <= pipeline, "{} + {} > {} for {}", w, a, pipeline, company_id);
                prop_assert!(i <= a, "{} > {} for {}", i, a, company_id);

                let average = &row["wishlistEaseAverage"];
                if eases.is_empty() {
                    prop_assert!(average.is_null(), "expected null average for {}", company_id);
                } else {
                    let exact = eases.iter().sum::<i64>() as f64 / eases.len() as f64;
                    let actual = average.as_f64();
                    prop_assert!(actual.is_some(), "missing average for {}", company_id);
                    let actual = actual.unwrap();
                    prop_assert!((actual - exact).abs() < 0.001, "{} vs {}", actual, exact);
                    prop_assert!((1.0..=5.0).contains(&actual));
                }

                // Req 1.6: no Pipeline_Jobs → zeros and nulls.
                if pipeline == 0 {
                    prop_assert_eq!((w, a, i), (0, 0, 0));
                    prop_assert!(average.is_null());
                    prop_assert!(row["lastActivityAt"].is_null());
                    prop_assert!(row["lastAppliedAt"].is_null());
                }
            }
        }

        // Feature: companies-tab-sorting, Property 2: Latest timestamps match the instant model and ignore background events
        /// **Validates: Requirements 1.2, 1.3, 1.4, 1.12**
        #[test]
        fn prop_latest_timestamps_match_instant_model(
            fixture in companies_fixture_strategy()
        ) {
            let conn = test_connection();
            let seeded = seed_fixture(&conn, &fixture);

            let rows = list_companies_with_watches(&conn).unwrap();
            prop_assert_eq!(rows.len(), seeded.company_ids.len());

            let stored_applied: Vec<String> = conn
                .prepare("SELECT applied_at FROM jobs WHERE applied_at IS NOT NULL")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let stored_occurred: Vec<String> = conn
                .prepare("SELECT occurred_at FROM job_events")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

            for row in &rows {
                let company_id = row_company_id(row);
                let pipeline_jobs: Vec<&JobSpec> = seeded
                    .jobs
                    .iter()
                    .filter(|(id, spec)| *id == company_id && spec.is_pipeline())
                    .map(|(_, spec)| spec)
                    .collect();

                let expected_applied =
                    model_latest(pipeline_jobs.iter().filter_map(|j| j.applied_at));
                let expected_activity = model_latest(pipeline_jobs.iter().flat_map(|j| {
                    j.events
                        .iter()
                        .filter(|(event_type, _)| !BACKGROUND_EVENT_TYPES.contains(event_type))
                        .map(|(_, occurred_at)| *occurred_at)
                }));

                let actual_applied = &row["lastAppliedAt"];
                let actual_activity = &row["lastActivityAt"];
                prop_assert_eq!(
                    actual_applied.as_str(),
                    expected_applied,
                    "lastAppliedAt for {}",
                    company_id
                );
                prop_assert_eq!(
                    actual_activity.as_str(),
                    expected_activity,
                    "lastActivityAt for {}",
                    company_id
                );
                if expected_applied.is_none() {
                    prop_assert!(actual_applied.is_null());
                }
                if expected_activity.is_none() {
                    prop_assert!(actual_activity.is_null());
                }

                // Req 1.2: returned strings are byte-identical to stored values.
                if let Some(value) = actual_applied.as_str() {
                    prop_assert!(stored_applied.iter().any(|s| s.as_bytes() == value.as_bytes()));
                }
                if let Some(value) = actual_activity.as_str() {
                    prop_assert!(stored_occurred.iter().any(|s| s.as_bytes() == value.as_bytes()));
                }
            }
        }

        // Feature: companies-tab-sorting, Property 4: Existing row fields and order are unchanged
        /// **Validates: Requirements 1.9**
        #[test]
        fn prop_pipeline_fields_leave_existing_row_fields_and_order_unchanged(
            fixture in companies_fixture_strategy(),
            watches in prop::collection::vec((0usize..8, 0usize..3), 0..=6),
            reviews in prop::collection::vec((0usize..8, any::<bool>()), 0..=6),
        ) {
            let conn = test_connection();
            let seeded = seed_fixture(&conn, &fixture);
            let (watch_rows, review_rows) =
                seed_watches_and_reviews(&conn, &seeded.company_ids, &watches, &reviews);

            let rows = list_companies_with_watches(&conn).unwrap();
            let companies = list_companies(&conn).unwrap();

            // Same company set and order as list_companies.
            let row_ids: Vec<String> = rows.iter().map(row_company_id).collect();
            let expected_ids: Vec<String> = companies.iter().map(|c| c.id.clone()).collect();
            prop_assert_eq!(&row_ids, &expected_ids);

            for (row, company) in rows.iter().zip(&companies) {
                let company_id = &company.id;

                // Exactly the five existing keys plus the six new ones.
                let object = row.as_object().unwrap();
                let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
                keys.sort_unstable();
                let mut expected_keys = ROW_KEYS.to_vec();
                expected_keys.sort_unstable();
                prop_assert_eq!(keys, expected_keys, "keys for {}", company_id);

                // company: identical to the list_companies value.
                prop_assert_eq!(&row["company"], &serde_json::to_value(company).unwrap());

                // watches: every watch of this company, each equal to get_watch.
                let watch_values = row["watches"].as_array().unwrap();
                let mut actual_watch_ids: Vec<String> = watch_values
                    .iter()
                    .map(|w| w["id"].as_str().unwrap().to_string())
                    .collect();
                actual_watch_ids.sort();
                let mut expected_watch_ids: Vec<String> = watch_rows
                    .iter()
                    .filter(|(_, cid)| cid == company_id)
                    .map(|(id, _)| id.clone())
                    .collect();
                expected_watch_ids.sort();
                prop_assert_eq!(actual_watch_ids, expected_watch_ids, "watches for {}", company_id);
                for value in watch_values {
                    let watch = get_watch(&conn, value["id"].as_str().unwrap()).unwrap().unwrap();
                    prop_assert_eq!(value, &serde_json::to_value(watch).unwrap());
                }

                // reviews: pending reviews of this company, newest created_at first.
                let actual_review_ids: Vec<String> = row["reviews"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["id"].as_str().unwrap().to_string())
                    .collect();
                let mut expected_reviews: Vec<&(String, String, String, bool)> = review_rows
                    .iter()
                    .filter(|(_, cid, _, pending)| cid == company_id && *pending)
                    .collect();
                expected_reviews.sort_by(|a, b| b.2.cmp(&a.2));
                let expected_review_ids: Vec<String> =
                    expected_reviews.iter().map(|r| r.0.clone()).collect();
                prop_assert_eq!(actual_review_ids, expected_review_ids, "reviews for {}", company_id);

                // openPositionCount and trackedJobCount: prior-spec models.
                let company_jobs = seeded.jobs.iter().filter(|(id, _)| id == company_id);
                let expected_open = company_jobs
                    .clone()
                    .filter(|(_, spec)| {
                        spec.posting_state == "active"
                            && matches!(spec.source, "greenhouse" | "lever" | "ashby")
                    })
                    .count() as i64;
                let expected_tracked = company_jobs
                    .filter(|(_, spec)| spec.status != "archived")
                    .count() as i64;
                prop_assert_eq!(row["openPositionCount"].as_i64(), Some(expected_open));
                prop_assert_eq!(row["trackedJobCount"].as_i64(), Some(expected_tracked));
            }
        }
    }
}
