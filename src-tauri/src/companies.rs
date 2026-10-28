use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{map_sqlite, AppError, AppResult};
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

pub fn list_companies_with_watches(conn: &Connection) -> AppResult<Vec<serde_json::Value>> {
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
            serde_json::json!({
                "company": company,
                "watches": company_watches,
                "reviews": company_reviews,
                "openPositionCount": open_position_count,
                "trackedJobCount": tracked_job_count
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
        )
            .prop_map(
                |(company_index, status, source, posting_state, is_favorite)| JobSpec {
                    company_index,
                    status,
                    source,
                    posting_state,
                    is_favorite,
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
                    "INSERT INTO jobs (id, company_id, title, url, canonical_url, status, posting_state, source, is_favorite, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                    params![
                        format!("job-{index}"),
                        company_id,
                        format!("Job {index}"),
                        url,
                        spec.status,
                        spec.posting_state,
                        spec.source,
                        spec.is_favorite,
                        timestamp
                    ],
                )
                .unwrap();
                jobs.push((company_id, spec.clone()));
            }
        }
        SeededFixture { company_ids, jobs }
    }

    fn row_company_id(row: &serde_json::Value) -> String {
        row["company"]["id"].as_str().unwrap().to_string()
    }

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
    }
}
