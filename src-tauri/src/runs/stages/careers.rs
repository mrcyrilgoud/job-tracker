//! Careers-page check stage (moved from `runner.rs`, unchanged semantics).
//!
//! Stage 3 of a Jobs_Cycle (design.md, Component 6). Like the watches stage it
//! runs against the coordinator-owned connection: a sliding `JoinSet` window
//! fetches at most four unique careers URLs at once (30 s per fetch), and each
//! completion is applied sequentially through `apply_careers_check`, so the
//! `!Send` `Connection` is only touched between awaits.
//!
//! ## Stage-level result vs item-level result
//! A per-item *fetch* failure (network/timeout/HTTP) is not a stage failure:
//! the original runner recorded it as an item result of the shape
//! `{ "changed": false, "reason": <error> }` and moved on. That behavior is
//! preserved here. A stage-level `Err` is returned **only** when
//! `apply_careers_check` returns `Err` (an apply/DB failure).
//!
//! ## Scope boundary (task 8.1 vs 8.2)
//! Exposes [`run_careers_stage`]. It calls a supplied progress callback with a
//! [`StageProgress`] at start, after each applied item, and at completion. The
//! coordinator (task 8.2) owns the connection, sequencing, and event publishing.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::Connection;
use tokio::task::JoinSet;

use crate::ats::careers::{apply_careers_check, fetch_careers_hash};
use crate::error::{AppError, AppResult};
use crate::runs::model::{StageName, StageOutcome};
use crate::runs::progress::{BoundedCount, StageProgress};

/// Bounded concurrency for careers fetches (unchanged from `runner.rs`).
pub const CAREERS_CONCURRENCY: usize = 4;
/// Per-fetch timeout (unchanged from `runner.rs`).
pub const CAREERS_TIMEOUT: Duration = Duration::from_secs(30);

/// One company with a careers URL, read from `companies`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CareersTarget {
    pub company_id: String,
    pub name: String,
    pub careers_url: String,
}

/// Outcome of the whole careers stage.
#[derive(Debug, Clone)]
pub struct CareersStageResult {
    /// Item-level results in apply order, shaped for the legacy JSON
    /// projection (`{ "companyId": ..., "result": ... }`).
    pub items: Vec<serde_json::Value>,
    /// Final `StageProgress` (also delivered through the progress callback).
    pub progress: StageProgress,
}

/// Read the companies that have a careers URL, in insertion order.
pub fn load_careers_targets(conn: &Connection) -> AppResult<Vec<CareersTarget>> {
    let mut stmt =
        conn.prepare("SELECT id, name, careers_url FROM companies WHERE careers_url IS NOT NULL")?;
    let rows = stmt.query_map([], |r| {
        Ok(CareersTarget {
            company_id: r.get(0)?,
            name: r.get(1)?,
            careers_url: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// A fetched careers page: `(company_id, name, Ok((hash, normalized_text)) | Err(message))`.
type FetchedCareers = (String, String, AppResult<(String, String)>);

/// Run the careers stage against the caller's connection.
///
/// `progress` is called with a [`StageProgress`] at start (`InProgress`, 0/total),
/// after each applied item, and once at completion. `on_item` is called with the
/// applied company name right before its progress snapshot, in apply order (the
/// original runner emitted `Checked {name}` in fetch-completion order).
///
/// Production callers use the real fetcher; [`run_careers_stage_with_fetch`]
/// injects pre-fetched data for offline tests.
pub async fn run_careers_stage(
    conn: &mut Connection,
    mut progress: impl FnMut(StageProgress),
    mut on_item: impl FnMut(&str),
) -> AppResult<CareersStageResult> {
    let targets = load_careers_targets(conn)?;
    run_careers_stage_live(conn, targets, &mut progress, &mut on_item).await
}

#[derive(Debug)]
struct CareersFetchGroup {
    careers_url: String,
    targets: Vec<(String, String)>,
}

type SharedCareersFetch = Arc<Result<(String, String), String>>;

/// Exact URL equality is the safe deduplication boundary: each company still
/// receives its own snapshot/apply operation, while the page and normalized
/// text are fetched and retained only once.
fn group_careers_targets(targets: Vec<CareersTarget>) -> Vec<CareersFetchGroup> {
    let mut groups: Vec<CareersFetchGroup> = Vec::new();
    let mut by_url = HashMap::<String, usize>::new();
    for target in targets {
        if let Some(&index) = by_url.get(&target.careers_url) {
            groups[index].targets.push((target.company_id, target.name));
        } else {
            by_url.insert(target.careers_url.clone(), groups.len());
            groups.push(CareersFetchGroup {
                careers_url: target.careers_url,
                targets: vec![(target.company_id, target.name)],
            });
        }
    }
    groups
}

fn spawn_careers_fetch(
    fetches: &mut JoinSet<(Vec<(String, String)>, SharedCareersFetch)>,
    group: CareersFetchGroup,
) {
    fetches.spawn(async move {
        let fetched = tokio::time::timeout(CAREERS_TIMEOUT, fetch_careers_hash(&group.careers_url))
            .await
            .map_err(|_| "request timed out after 30s".to_string())
            .and_then(|result| result.map_err(|e| e.to_string()));
        (group.targets, Arc::new(fetched))
    });
}

/// Keep only [`CAREERS_CONCURRENCY`] active fetch tasks and apply each result
/// as it completes. Duplicate URLs share the result without cloning the page.
async fn run_careers_stage_live(
    conn: &mut Connection,
    targets: Vec<CareersTarget>,
    progress: &mut impl FnMut(StageProgress),
    on_item: &mut impl FnMut(&str),
) -> AppResult<CareersStageResult> {
    let total = targets.len();
    progress(stage_progress(StageOutcome::InProgress, 0, total, None));

    let mut groups = group_careers_targets(targets).into_iter();
    let mut fetches = JoinSet::new();
    for _ in 0..CAREERS_CONCURRENCY {
        if let Some(group) = groups.next() {
            spawn_careers_fetch(&mut fetches, group);
        }
    }

    let mut items = Vec::with_capacity(total);
    let mut completed = 0usize;
    while let Some(result) = fetches.join_next().await {
        let (company_targets, fetched) = result.map_err(|e| AppError::from(e.to_string()))?;
        for (company_id, name) in company_targets {
            match fetched.as_ref() {
                Ok((hash, text)) => {
                    let result = apply_careers_check(conn, &company_id, &name, hash, text)?;
                    items.push(serde_json::json!({ "companyId": company_id, "result": result }));
                }
                Err(error) => items.push(serde_json::json!({
                    "companyId": company_id,
                    "result": { "changed": false, "reason": error }
                })),
            }
            on_item(&name);
            completed += 1;
            progress(stage_progress(
                StageOutcome::InProgress,
                completed,
                total,
                None,
            ));
        }
        if let Some(group) = groups.next() {
            spawn_careers_fetch(&mut fetches, group);
        }
    }

    let progress_final = stage_progress(StageOutcome::Succeeded, total, total, None);
    progress(progress_final.clone());
    Ok(CareersStageResult {
        items,
        progress: progress_final,
    })
}

/// The stage body with an injectable fetch step, so tests can supply
/// pre-fetched data and exercise the apply path offline.
pub async fn run_careers_stage_with_fetch<Fut>(
    conn: &mut Connection,
    targets: Vec<CareersTarget>,
    progress: &mut impl FnMut(StageProgress),
    on_item: &mut impl FnMut(&str),
    fetch: impl FnOnce(Vec<CareersTarget>) -> Fut,
) -> AppResult<CareersStageResult>
where
    Fut: std::future::Future<Output = AppResult<Vec<FetchedCareers>>>,
{
    let total = targets.len();
    progress(stage_progress(StageOutcome::InProgress, 0, total, None));

    let fetched = fetch(targets).await?;

    let mut items = Vec::with_capacity(fetched.len());
    let mut completed = 0usize;
    for (company_id, name, fetched) in fetched {
        match fetched {
            // A stage-level Err is returned only when the apply step fails.
            Ok((hash, text)) => {
                let result = apply_careers_check(conn, &company_id, &name, &hash, &text)?;
                items.push(serde_json::json!({ "companyId": company_id, "result": result }));
            }
            // A per-item fetch failure stays an item-level result, exactly as
            // the original runner loop did.
            Err(e) => {
                items.push(serde_json::json!({
                    "companyId": company_id,
                    "result": { "changed": false, "reason": e.to_string() }
                }));
            }
        }
        // Reported in apply order, matching the original `Checked {name}` emit.
        on_item(&name);
        completed += 1;
        progress(stage_progress(
            StageOutcome::InProgress,
            completed,
            total,
            None,
        ));
    }

    let progress_final = stage_progress(StageOutcome::Succeeded, total, total, None);
    progress(progress_final.clone());
    Ok(CareersStageResult {
        items,
        progress: progress_final,
    })
}

fn stage_progress(
    outcome: StageOutcome,
    current: usize,
    total: usize,
    error: Option<String>,
) -> StageProgress {
    StageProgress {
        name: StageName::Careers,
        outcome,
        current: BoundedCount::from_usize(current),
        total: BoundedCount::from_usize(total),
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::companies::create_company;
    use crate::db::migrate::migrate;
    use rusqlite::params;

    fn test_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn company_with_careers(conn: &Connection, name: &str, url: &str) -> String {
        let company = create_company(conn, name, Some(url)).unwrap();
        company.id
    }

    #[test]
    fn groups_exact_duplicate_careers_urls_without_merging_other_pages() {
        let groups = group_careers_targets(vec![
            CareersTarget {
                company_id: "company-a".into(),
                name: "Acme".into(),
                careers_url: "https://jobs.example/careers".into(),
            },
            CareersTarget {
                company_id: "company-b".into(),
                name: "Acme Holdings".into(),
                careers_url: "https://jobs.example/careers".into(),
            },
            CareersTarget {
                company_id: "company-c".into(),
                name: "Other".into(),
                careers_url: "https://other.example/careers".into(),
            },
        ]);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].targets.len(), 2);
        assert_eq!(groups[1].targets.len(), 1);
    }

    #[tokio::test]
    async fn applies_fetched_pages_and_reports_succeeded() {
        let mut conn = test_connection();
        let id = company_with_careers(&conn, "Acme", "https://acme.example/careers");
        let targets = load_careers_targets(&conn).unwrap();
        assert_eq!(targets.len(), 1);

        let id_for_fetch = id.clone();
        let mut snapshots = Vec::new();
        let mut applied = Vec::new();
        let result = run_careers_stage_with_fetch(
            &mut conn,
            targets,
            &mut |p| snapshots.push(p),
            &mut |name: &str| applied.push(name.to_string()),
            |_targets| async move {
                Ok(vec![(
                    id_for_fetch,
                    "Acme".to_string(),
                    Ok(("v2:hash-1".to_string(), "careers open roles".to_string())),
                )])
            },
        )
        .await
        .unwrap();
        assert_eq!(applied, vec!["Acme".to_string()]);

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0]["companyId"], serde_json::json!(id));
        // First snapshot captured => initial-snapshot item result.
        assert_eq!(
            result.items[0]["result"]["changed"],
            serde_json::json!(false)
        );
        let snapshots_saved: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM careers_page_snapshots WHERE company_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snapshots_saved, 1);
        assert_eq!(snapshots.len(), 3);
    }

    #[tokio::test]
    async fn item_fetch_error_stays_item_level_not_stage_level() {
        let mut conn = test_connection();
        let id = company_with_careers(&conn, "Acme", "https://acme.example/careers");
        let targets = load_careers_targets(&conn).unwrap();

        let id_for_fetch = id.clone();
        let result = run_careers_stage_with_fetch(
            &mut conn,
            targets,
            &mut |_p| {},
            &mut |_name: &str| {},
            |_targets| async move {
                Ok(vec![(
                    id_for_fetch,
                    "Acme".to_string(),
                    Err(AppError::from("HTTP 503")),
                )])
            },
        )
        .await
        .unwrap();

        assert_eq!(
            result.items[0]["result"]["changed"],
            serde_json::json!(false)
        );
        assert_eq!(
            result.items[0]["result"]["reason"],
            serde_json::json!("HTTP 503")
        );
        // No snapshot row is written for a failed fetch.
        let snapshots_saved: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM careers_page_snapshots WHERE company_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snapshots_saved, 0);
    }

    #[tokio::test]
    async fn empty_stage_reports_zero_total_succeeded() {
        let mut conn = test_connection();
        let mut snapshots = Vec::new();
        let result = run_careers_stage(&mut conn, |p| snapshots.push(p), |_name| {})
            .await
            .unwrap();
        assert!(result.items.is_empty());
        assert_eq!(result.progress.total, BoundedCount::from_usize(0));
    }
}
