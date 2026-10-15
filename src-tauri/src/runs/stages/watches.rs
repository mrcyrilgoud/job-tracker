//! ATS watch synchronization stage (moved from `runner.rs`, unchanged semantics).
//!
//! This is stage 2 of a Jobs_Cycle (design.md, Component 6). Unlike the
//! postings stage, the watches/careers/csv stages run against the connection
//! the coordinator owns: a sliding `JoinSet` window fetches at most two unique
//! listing URLs at once (30 s per fetch), and each completion is applied
//! sequentially through the existing sync service, so the `!Send` `Connection`
//! is only touched between awaits.
//!
//! ## Stage-level result vs item-level result
//! A per-item *fetch* failure (network/timeout) is not a stage failure: it is
//! passed to `apply_watch_sync` as `Err(message)`, which records the failure
//! bookkeeping on the watch and returns an item result with `ok: false`. This
//! matches the pre-extraction runner behavior. A stage-level `Err` is returned
//! **only** when `apply_watch_sync` itself returns `Err` (an apply/DB failure).
//!
//! ## Scope boundary (task 8.1 vs 8.2)
//! This module exposes [`run_watches_stage`]. It calls a supplied progress
//! callback with a [`StageProgress`] snapshot at start, after each applied
//! item, and at completion, so the coordinator (task 8.2) can publish events.
//! The connection is owned by the caller; sequencing across stages, event
//! publishing, and CSV mark-dirty rules all belong to the coordinator.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::Connection;
use tokio::task::JoinSet;

use crate::ats::sync::{apply_watch_sync, apply_watch_sync_ref, fetch_remote_jobs};
use crate::ats::{listing_url, AtsJob};
use crate::error::{AppError, AppResult};
use crate::runs::model::{StageName, StageOutcome};
use crate::runs::progress::{BoundedCount, StageProgress};

/// Bounded concurrency for watch fetches (unchanged from `runner.rs`).
pub const WATCHES_CONCURRENCY: usize = 2;
/// Per-fetch timeout (unchanged from `runner.rs`).
pub const WATCHES_TIMEOUT: Duration = Duration::from_secs(30);

/// One watch's identity as read from `company_watches`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchTarget {
    pub watch_id: String,
    pub provider: String,
    pub board_slug: String,
}

/// Outcome of the whole watches stage.
#[derive(Debug, Clone)]
pub struct WatchesStageResult {
    /// Item-level results in `apply_watch_sync` order, shaped for the legacy
    /// JSON projection (`{ "watchId": ..., "result": ... }`).
    pub items: Vec<serde_json::Value>,
    /// `Succeeded` here always: an apply/DB failure short-circuits with `Err`
    /// before this is built, so a returned `Ok` is always a success.
    /// Final `StageProgress` (also delivered through the progress callback).
    pub progress: StageProgress,
}

/// Read the watch targets from the connection, in insertion order.
pub fn load_watch_targets(conn: &Connection) -> AppResult<Vec<WatchTarget>> {
    let mut stmt = conn.prepare("SELECT id, provider, board_slug FROM company_watches")?;
    let rows = stmt.query_map([], |r| {
        Ok(WatchTarget {
            watch_id: r.get(0)?,
            provider: r.get(1)?,
            board_slug: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// The pre-fetched remote jobs for one watch, keyed by watch id. `Ok` carries
/// the remote listing; `Err` carries the failure message that `apply_watch_sync`
/// records as failure bookkeeping (an item-level, not stage-level, failure).
type FetchedWatch = (String, Result<Vec<crate::ats::AtsJob>, String>);

/// Run the watches stage against the caller's connection.
///
/// `progress` is called with a [`StageProgress`] at start (`InProgress`, 0/total),
/// after each applied item, and once at completion. `on_item` is called with the
/// applied watch id right before its progress snapshot. Neither blocks the stage.
///
/// Production callers pass the real network fetcher; use
/// [`run_watches_stage_with_fetch`] to inject pre-fetched data in tests.
pub async fn run_watches_stage(
    conn: &mut Connection,
    mut progress: impl FnMut(StageProgress),
    mut on_item: impl FnMut(&str),
) -> AppResult<WatchesStageResult> {
    let targets = load_watch_targets(conn)?;
    run_watches_stage_live(conn, targets, &mut progress, &mut on_item).await
}

#[derive(Debug)]
struct WatchFetchGroup {
    provider: String,
    board_slug: String,
    watch_ids: Vec<String>,
}

type SharedWatchFetch = (Vec<String>, Arc<Result<Vec<AtsJob>, String>>);

/// Group only watches that resolve to the same known public listing URL. An
/// unsupported provider stays isolated so a malformed target keeps its own
/// failure semantics.
fn group_watch_targets(targets: Vec<WatchTarget>) -> Vec<WatchFetchGroup> {
    let mut groups = Vec::new();
    let mut by_url = HashMap::<String, usize>::new();

    for target in targets {
        let Some(url) = listing_url(&target.provider, &target.board_slug) else {
            groups.push(WatchFetchGroup {
                provider: target.provider,
                board_slug: target.board_slug,
                watch_ids: vec![target.watch_id],
            });
            continue;
        };

        if let Some(&index) = by_url.get(&url) {
            groups[index].watch_ids.push(target.watch_id);
        } else {
            by_url.insert(url, groups.len());
            groups.push(WatchFetchGroup {
                provider: target.provider,
                board_slug: target.board_slug,
                watch_ids: vec![target.watch_id],
            });
        }
    }
    groups
}

fn spawn_watch_fetch(fetches: &mut JoinSet<SharedWatchFetch>, group: WatchFetchGroup) {
    fetches.spawn(async move {
        let remote = tokio::time::timeout(
            WATCHES_TIMEOUT,
            fetch_remote_jobs(&group.provider, &group.board_slug),
        )
        .await
        .map_err(|_| "request timed out after 30s".to_string())
        .and_then(|result| result.map_err(|e| e.to_string()));
        (group.watch_ids, Arc::new(remote))
    });
}

/// Fetches at most [`WATCHES_CONCURRENCY`] groups at once and applies each
/// completion immediately. This keeps both task count and retained remote
/// listing memory bounded by the active window instead of by all watches.
async fn run_watches_stage_live(
    conn: &mut Connection,
    targets: Vec<WatchTarget>,
    progress: &mut impl FnMut(StageProgress),
    on_item: &mut impl FnMut(&str),
) -> AppResult<WatchesStageResult> {
    let total = targets.len();
    progress(stage_progress(StageOutcome::InProgress, 0, total, None));

    let mut groups = group_watch_targets(targets).into_iter();
    let mut fetches = JoinSet::new();
    for _ in 0..WATCHES_CONCURRENCY {
        if let Some(group) = groups.next() {
            spawn_watch_fetch(&mut fetches, group);
        }
    }

    let mut items = Vec::with_capacity(total);
    let mut completed = 0usize;
    while let Some(result) = fetches.join_next().await {
        let (watch_ids, remote) = result.map_err(|e| AppError::from(e.to_string()))?;
        for watch_id in watch_ids {
            let result = apply_watch_sync_ref(conn, &watch_id, remote.as_ref())?;
            on_item(&watch_id);
            items.push(serde_json::json!({ "watchId": watch_id, "result": result }));
            completed += 1;
            progress(stage_progress(
                StageOutcome::InProgress,
                completed,
                total,
                None,
            ));
        }
        if let Some(group) = groups.next() {
            spawn_watch_fetch(&mut fetches, group);
        }
    }

    let progress_final = stage_progress(StageOutcome::Succeeded, total, total, None);
    progress(progress_final.clone());
    Ok(WatchesStageResult {
        items,
        progress: progress_final,
    })
}

/// The stage body with an injectable fetch step, so tests can supply
/// pre-fetched data and exercise the apply path offline.
///
/// `fetch` receives the watch targets and returns the pre-fetched remote jobs
/// keyed by watch id. Every fetched entry is then applied sequentially through
/// `apply_watch_sync`; an apply/DB failure returns a stage-level `Err`.
pub async fn run_watches_stage_with_fetch<Fut>(
    conn: &mut Connection,
    targets: Vec<WatchTarget>,
    progress: &mut impl FnMut(StageProgress),
    on_item: &mut impl FnMut(&str),
    fetch: impl FnOnce(Vec<WatchTarget>) -> Fut,
) -> AppResult<WatchesStageResult>
where
    Fut: std::future::Future<Output = AppResult<Vec<FetchedWatch>>>,
{
    let total = targets.len();
    progress(stage_progress(StageOutcome::InProgress, 0, total, None));

    let fetched = fetch(targets).await?;

    let mut items = Vec::with_capacity(fetched.len());
    let mut completed = 0usize;
    for (watch_id, remote) in fetched {
        // A stage-level Err is returned only when the apply step fails (Req 2.2).
        let result = apply_watch_sync(conn, &watch_id, remote)?;
        on_item(&watch_id);
        items.push(serde_json::json!({ "watchId": watch_id, "result": result }));
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
    Ok(WatchesStageResult {
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
        name: StageName::Watches,
        outcome,
        current: BoundedCount::from_usize(current),
        total: BoundedCount::from_usize(total),
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ats::AtsJob;
    use crate::companies::{create_company, insert_watch};
    use crate::db::migrate::migrate;

    fn test_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn ats_job(external_id: &str, slug: &str) -> AtsJob {
        AtsJob {
            external_id: external_id.into(),
            title: "Platform Engineer".into(),
            url: format!("https://boards.greenhouse.io/{slug}/jobs/{external_id}"),
            location: Some("Remote".into()),
        }
    }

    #[test]
    fn groups_only_watches_with_the_same_known_listing_url() {
        let groups = group_watch_targets(vec![
            WatchTarget {
                watch_id: "watch-a".into(),
                provider: "greenhouse".into(),
                board_slug: "acme".into(),
            },
            WatchTarget {
                watch_id: "watch-b".into(),
                provider: "greenhouse".into(),
                board_slug: "acme".into(),
            },
            WatchTarget {
                watch_id: "watch-c".into(),
                provider: "lever".into(),
                board_slug: "acme".into(),
            },
            WatchTarget {
                watch_id: "watch-d".into(),
                provider: "future-ats".into(),
                board_slug: "acme".into(),
            },
            WatchTarget {
                watch_id: "watch-e".into(),
                provider: "future-ats".into(),
                board_slug: "acme".into(),
            },
        ]);

        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0].watch_ids, ["watch-a", "watch-b"]);
        assert_eq!(groups[1].watch_ids, ["watch-c"]);
        assert_eq!(groups[2].watch_ids, ["watch-d"]);
        assert_eq!(groups[3].watch_ids, ["watch-e"]);
    }

    #[tokio::test]
    async fn applies_fetched_jobs_and_reports_succeeded() {
        let mut conn = test_connection();
        let company = create_company(&conn, "Acme", None).unwrap();
        let watch = insert_watch(&conn, &company.id, "greenhouse", "acme").unwrap();
        let targets = load_watch_targets(&conn).unwrap();
        assert_eq!(targets.len(), 1);

        let watch_id = watch.id.clone();
        let mut snapshots = Vec::new();
        let mut applied = Vec::new();
        let result = run_watches_stage_with_fetch(
            &mut conn,
            targets,
            &mut |p| snapshots.push(p),
            &mut |id: &str| applied.push(id.to_string()),
            |_targets| async move { Ok(vec![(watch_id, Ok(vec![ats_job("role-1", "acme")]))]) },
        )
        .await
        .unwrap();
        assert_eq!(applied, vec![watch.id.clone()]);

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0]["watchId"], serde_json::json!(watch.id));
        assert_eq!(result.items[0]["result"]["ok"], serde_json::json!(true));
        assert_eq!(result.items[0]["result"]["created"], serde_json::json!(1));
        // Progress: start (0/1) + one applied (1/1) + final succeeded (1/1).
        assert_eq!(snapshots.len(), 3);
        assert_eq!(snapshots[0].outcome, StageOutcome::InProgress);
        assert_eq!(snapshots.last().unwrap().outcome, StageOutcome::Succeeded);
        assert_eq!(result.progress.current, BoundedCount::from_usize(1));
        assert_eq!(result.progress.total, BoundedCount::from_usize(1));
    }

    #[tokio::test]
    async fn item_fetch_error_stays_item_level_not_stage_level() {
        let mut conn = test_connection();
        let company = create_company(&conn, "Acme", None).unwrap();
        let watch = insert_watch(&conn, &company.id, "greenhouse", "acme").unwrap();
        let targets = load_watch_targets(&conn).unwrap();

        let watch_id = watch.id.clone();
        let result = run_watches_stage_with_fetch(
            &mut conn,
            targets,
            &mut |_p| {},
            &mut |_id: &str| {},
            |_targets| async move { Ok(vec![(watch_id, Err("request timed out".into()))]) },
        )
        .await
        .unwrap();

        // The stage still succeeds; the failure is recorded as an item result.
        assert_eq!(result.items[0]["result"]["ok"], serde_json::json!(false));
        let failures: i64 = conn
            .query_row(
                "SELECT consecutive_sync_failures FROM company_watches WHERE id = ?1",
                rusqlite::params![watch.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(failures, 1);
    }

    #[tokio::test]
    async fn apply_failure_surfaces_as_stage_err() {
        let mut conn = test_connection();
        let company = create_company(&conn, "Acme", None).unwrap();
        let _watch = insert_watch(&conn, &company.id, "greenhouse", "acme").unwrap();
        let targets = load_watch_targets(&conn).unwrap();

        // An invalid remote URL makes `apply_watch_sync` return Err (DB/apply
        // failure), which must surface as a stage-level Err.
        let bad = AtsJob {
            external_id: "role-1".into(),
            title: "Backend".into(),
            url: "not a valid url".into(),
            location: None,
        };
        let watch_id = targets[0].watch_id.clone();
        let result = run_watches_stage_with_fetch(
            &mut conn,
            targets,
            &mut |_p| {},
            &mut |_id: &str| {},
            |_targets| async move { Ok(vec![(watch_id, Ok(vec![bad]))]) },
        )
        .await;
        assert!(result.is_err(), "apply failure must be a stage-level Err");
    }

    #[tokio::test]
    async fn empty_stage_reports_zero_total_succeeded() {
        let mut conn = test_connection();
        let mut snapshots = Vec::new();
        let result = run_watches_stage(&mut conn, |p| snapshots.push(p), |_id| {})
            .await
            .unwrap();
        assert!(result.items.is_empty());
        assert_eq!(result.progress.total, BoundedCount::from_usize(0));
        assert_eq!(snapshots.first().unwrap().outcome, StageOutcome::InProgress);
        assert_eq!(snapshots.last().unwrap().outcome, StageOutcome::Succeeded);
    }
}
