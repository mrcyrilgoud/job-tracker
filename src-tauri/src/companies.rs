use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::service::find_or_create_company;
use crate::models::{CareersPageReview, Company, CompanyWatch};
use crate::util::{create_id, now_iso};

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
    let timestamp = now_iso();
    let watch = CompanyWatch {
        id: create_id(),
        company_id: company_id.to_string(),
        provider: provider.to_string(),
        board_slug: board_slug.trim().to_string(),
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
        .query_map([], |row| {
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
        })
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
            serde_json::json!({
                "company": company,
                "watches": company_watches,
                "reviews": company_reviews
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
