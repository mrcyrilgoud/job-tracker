//! Single-posting availability check (`check_job_posting`) as a thin shim over
//! the evidence-based pipeline in [`crate::jobs::posting_check`].
//!
//! The Tauri `check_job_posting` command runs three steps so the DB mutex is
//! never held across network I/O:
//! 1. [`load_posting_check_input`] (DB read),
//! 2. [`evaluate_posting`] (network only, bounded by [`POSTING_EVAL_TIMEOUT`]),
//! 3. [`apply_posting_evidence`] (classify + `apply_classified_check` with
//!    `run_id = NULL`, inside the caller's transaction).
//!
//! It is not a Run and takes no runner lock, as before.
//!
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::posting_check::classify::classify;
use crate::jobs::posting_check::evidence::CheckEvidence;
use crate::jobs::posting_check::fetch::PostingFetcher;
use crate::jobs::posting_check::persist::apply_classified_check;
use crate::jobs::posting_check::provider::{ProviderListingCache, WatchBoard};
use crate::jobs::posting_check::{evaluate, PostingCheckInput};
use crate::runs::model::JobIdentity;
use crate::util::now_iso;

/// Upper bound for one posting evaluation (Req 8.1). On expiry the result is
/// Unknown with failure category `timeout` (Req 8.2).
pub const POSTING_EVAL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckPostingResult {
    pub posting_state: String,
    pub last_check_result: String,
    pub last_checked_at: String,
}

/// Load the job's identity, provider fields, and its company's watch rows.
/// Returns `Job not found` for an unknown id (unchanged error text).
pub fn load_posting_check_input(conn: &Connection, job_id: &str) -> AppResult<PostingCheckInput> {
    let row = conn
        .query_row(
            "SELECT j.id, j.title, c.name, j.url, j.source, j.source_external_id, j.company_id
             FROM jobs j JOIN companies c ON c.id = j.company_id
             WHERE j.id = ?1",
            params![job_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(map_sqlite)?;
    let Some((id, title, company_name, url, source, source_external_id, company_id)) = row else {
        return Err(AppError::from("Job not found"));
    };

    let mut stmt = conn
        .prepare(
            "SELECT provider, board_slug FROM company_watches
             WHERE company_id = ?1 ORDER BY created_at, id",
        )
        .map_err(map_sqlite)?;
    let watches = stmt
        .query_map(params![company_id], |r| {
            Ok(WatchBoard {
                provider: r.get(0)?,
                board_slug: r.get(1)?,
            })
        })
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    Ok(PostingCheckInput {
        identity: JobIdentity {
            job_id: id,
            title,
            company_name,
            posting_url: url,
        },
        source,
        source_external_id,
        watches,
    })
}

/// Network-only portion. Do not hold a DB mutex across this call.
pub async fn evaluate_posting<F: PostingFetcher>(
    input: &PostingCheckInput,
    fetcher: &F,
) -> CheckEvidence {
    let attempted_at = now_iso();
    let cache = ProviderListingCache::new();
    match tokio::time::timeout(
        POSTING_EVAL_TIMEOUT,
        evaluate(input, fetcher, &cache, attempted_at.clone()),
    )
    .await
    {
        Ok(evidence) => evidence,
        Err(_elapsed) => CheckEvidence::timeout(&input.identity, attempted_at).normalized(),
    }
}

/// Classify `evidence` and persist it with `run_id = NULL`. Run inside the
/// caller's transaction.
pub fn apply_posting_evidence(
    conn: &Connection,
    job_id: &str,
    evidence: &CheckEvidence,
) -> AppResult<CheckPostingResult> {
    let classification = classify(evidence);
    let applied = apply_classified_check(conn, None, job_id, evidence, &classification)?;
    Ok(CheckPostingResult {
        posting_state: applied.posting_state.as_str().to_string(),
        last_check_result: applied.last_check_result,
        last_checked_at: applied.last_checked_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;
    use crate::jobs::posting_check::evidence::Provider;
    use crate::jobs::posting_check::fetch::PageFetch;
    use crate::jobs::posting_check::provider::ListingFetch;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate(&conn).unwrap();
        let t = "2025-01-01T00:00:00.000Z";
        conn.execute(
            "INSERT INTO companies (id, name, created_at, updated_at) VALUES ('c1', 'Acme', ?1, ?1)",
            params![t],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO jobs (id, company_id, title, url, canonical_url, posting_state, source,
                 source_external_id, created_at, updated_at)
             VALUES ('j1', 'c1', 'Engineer', 'https://boards.greenhouse.io/acme/jobs/42',
                 'https://boards.greenhouse.io/acme/jobs/42', 'unknown', 'greenhouse', '42', ?1, ?1)",
            params![t],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO company_watches (id, company_id, provider, board_slug, created_at, updated_at)
             VALUES ('w1', 'c1', 'greenhouse', 'acme', ?1, ?1)",
            params![t],
        )
        .unwrap();
        conn
    }

    /// Returns a fixed listing; any page request panics (proves it was skipped).
    struct ListingOnly(&'static str);

    impl PostingFetcher for ListingOnly {
        async fn fetch_page(&self, url: &str) -> PageFetch {
            panic!("unexpected page fetch {url}")
        }
        async fn fetch_listing(&self, _provider: Provider, _slug: &str) -> ListingFetch {
            ListingFetch::Fetched {
                http_status: 200,
                body: self.0.into(),
            }
        }
    }

    /// Never resolves.
    struct Hangs;

    impl PostingFetcher for Hangs {
        async fn fetch_page(&self, _url: &str) -> PageFetch {
            std::future::pending().await
        }
        async fn fetch_listing(&self, _provider: Provider, _slug: &str) -> ListingFetch {
            std::future::pending().await
        }
    }

    #[test]
    fn load_input_reads_identity_provider_fields_and_watches() {
        let conn = db();
        let input = load_posting_check_input(&conn, "j1").unwrap();
        assert_eq!(input.identity.job_id, "j1");
        assert_eq!(input.identity.title, "Engineer");
        assert_eq!(input.identity.company_name, "Acme");
        assert_eq!(
            input.identity.posting_url,
            "https://boards.greenhouse.io/acme/jobs/42"
        );
        assert_eq!(input.source.as_deref(), Some("greenhouse"));
        assert_eq!(input.source_external_id.as_deref(), Some("42"));
        assert_eq!(
            input.watches,
            vec![WatchBoard {
                provider: "greenhouse".into(),
                board_slug: "acme".into()
            }]
        );
        let err = load_posting_check_input(&conn, "missing").unwrap_err();
        assert_eq!(err.to_string(), "Job not found");
    }

    #[tokio::test]
    async fn shim_evaluates_classifies_and_persists_without_a_run() {
        let conn = db();
        let input = load_posting_check_input(&conn, "j1").unwrap();
        let evidence = evaluate_posting(
            &input,
            &ListingOnly(r#"{"jobs":[{"id":42,"title":"Engineer"}]}"#),
        )
        .await;
        let result = apply_posting_evidence(&conn, "j1", &evidence).unwrap();

        assert_eq!(result.posting_state, "active");
        assert!(
            result.last_check_result.starts_with("active: "),
            "{}",
            result.last_check_result
        );
        assert_eq!(result.last_checked_at, evidence.attempted_at);

        let json = serde_json::to_value(&result).unwrap();
        let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["lastCheckResult", "lastCheckedAt", "postingState"]);

        let (run_id, kind): (Option<String>, String) = conn
            .query_row(
                "SELECT run_id, kind FROM posting_check_evidence WHERE job_id = 'j1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((run_id, kind.as_str()), (None, "authoritative"));
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM job_events WHERE type = 'posting_state_changed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(events, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn evaluation_is_bounded_by_the_timeout() {
        let conn = db();
        let input = load_posting_check_input(&conn, "j1").unwrap();
        let evidence = evaluate_posting(&input, &Hangs).await;
        let result = apply_posting_evidence(&conn, "j1", &evidence).unwrap();
        assert_eq!(result.posting_state, "unknown");
        assert!(
            result.last_check_result.starts_with("unknown: "),
            "{}",
            result.last_check_result
        );
    }
}
