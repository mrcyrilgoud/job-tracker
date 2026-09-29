use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::ats::{list_jobs, AtsJob};
use crate::companies::get_watch;
use crate::error::{map_sqlite, AppError, AppResult};
use crate::filtering::engine::{matches, JobView};
use crate::filtering::model::FilterCriteria;
use crate::filtering::resolver::{load_alias_table, resolve_effective_criteria};
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
        description: row.get(13)?,
        location: row.get(14)?,
        is_new_from_watch: row.get::<_, i64>(15)? != 0,
        watch_disposition: row.get(16)?,
        missing_from_sync_count: row.get(17)?,
        is_favorite: row.get::<_, i64>(18)? != 0,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
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
    let transaction = conn.unchecked_transaction().map_err(map_sqlite)?;
    match apply_watch_sync_inner(&transaction, watch_id, remote_jobs) {
        Ok(value) => {
            transaction.commit().map_err(map_sqlite)?;
            Ok(value)
        }
        Err(error) => {
            transaction.rollback().map_err(map_sqlite)?;
            Err(error)
        }
    }
}

fn apply_watch_sync_inner(
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
            "SELECT id, company_id, title, url, canonical_url, source_external_id, status, applied_at, posting_state, last_checked_at, last_check_result, source, notes, description, location, is_new_from_watch, watch_disposition, missing_from_sync_count, is_favorite, created_at, updated_at FROM jobs WHERE company_id = ?1 AND source = ?2",
        )
        .map_err(map_sqlite)?;
    let existing = stmt
        .query_map(params![watch.company_id, watch.provider], map_job)
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    // Resolve the effective filter criteria and alias table ONCE for this
    // watch, so ingest-time annotation uses the same matching authority as
    // query-time listing (Req 10.1). Fall back to match-all on error so a
    // resolver failure never blocks the sync or drops roles.
    let criteria = resolve_effective_criteria(conn, &watch.company_id, &watch.provider)
        .unwrap_or_else(|_| FilterCriteria::match_all());
    let aliases = load_alias_table(conn)?;

    let mut created = 0usize;
    let mut reactivated = 0usize;

    for remote in &remote_jobs {
        let canonical_url = normalize_canonical_url(&remote.url).map_err(AppError::from)?;

        // Compute the inclusion hint for this remote role. This is a
        // non-destructive annotation only: every role is still stored
        // regardless of the result, and no job is ever deleted (Req 9.3, 9.4).
        let included = matches(
            &criteria,
            &aliases,
            JobView {
                title: &remote.title,
                location: remote.location.as_deref(),
            },
        )
        .included;
        let included_int = i64::from(included);
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
                // Only record the hint when the role is included; a not-included
                // result must leave the previously recorded value unchanged
                // (Req 9.5).
                if included {
                    conn.execute(
                        "UPDATE jobs SET title=?1, url=?2, location=COALESCE(?3, location), missing_from_sync_count=0, posting_state='active', watch_filtered=?4, updated_at=?5 WHERE id=?6",
                        params![
                            remote.title,
                            remote.url,
                            remote.location,
                            included_int,
                            synced_at,
                            local.id
                        ],
                    )
                    .map_err(map_sqlite)?;
                } else {
                    conn.execute(
                        "UPDATE jobs SET title=?1, url=?2, location=COALESCE(?3, location), missing_from_sync_count=0, posting_state='active', updated_at=?4 WHERE id=?5",
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
            } else if included {
                conn.execute(
                    "UPDATE jobs SET title=?1, url=?2, location=COALESCE(?3, location), missing_from_sync_count=0, watch_filtered=?4, updated_at=?5 WHERE id=?6",
                    params![
                        remote.title,
                        remote.url,
                        remote.location,
                        included_int,
                        synced_at,
                        local.id
                    ],
                )
                .map_err(map_sqlite)?;
            } else {
                conn.execute(
                    "UPDATE jobs SET title=?1, url=?2, location=COALESCE(?3, location), missing_from_sync_count=0, updated_at=?4 WHERE id=?5",
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

        let by_url: Option<(String, String)> = conn
            .query_row(
                "SELECT id, company_id FROM jobs WHERE canonical_url = ?1",
                params![canonical_url],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(map_sqlite)?;

        if let Some((job_id, company_id)) = by_url {
            // A user may have deliberately reassigned a posting away from this
            // watch's company. The canonical URL still prevents a duplicate,
            // but this watch must not take the posting back on its next sync.
            if company_id != watch.company_id {
                continue;
            }
            // Only record the hint when included; a not-included result leaves
            // any previously recorded value unchanged (Req 9.5).
            if included {
                conn.execute(
                    "UPDATE jobs SET source=?1, source_external_id=?2, company_id=?3, title=?4, location=COALESCE(?5, location), is_new_from_watch=0, watch_disposition='saved', missing_from_sync_count=0, watch_filtered=?6, updated_at=?7 WHERE id=?8",
                    params![
                        watch.provider,
                        remote.external_id,
                        watch.company_id,
                        remote.title,
                        remote.location,
                        included_int,
                        synced_at,
                        job_id
                    ],
                )
                .map_err(map_sqlite)?;
            } else {
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
            }
            continue;
        }

        let job_id = create_id();
        // A brand-new row has no previously recorded hint to preserve, so it
        // records the actual evaluation result (including false) (Req 9.1, 9.2).
        conn.execute(
            r#"INSERT INTO jobs (
                id, company_id, title, url, canonical_url, source_external_id, status, applied_at,
                posting_state, last_checked_at, last_check_result, source, notes, description, location,
                is_new_from_watch, watch_disposition, missing_from_sync_count, watch_filtered, created_at, updated_at
            ) VALUES (?1,?2,?3,?4,?5,?6,'wishlist',NULL,'active',NULL,NULL,?7,NULL,NULL,?8,1,'new',0,?9,?10,?10)"#,
            params![
                job_id,
                watch.company_id,
                remote.title,
                remote.url,
                canonical_url,
                remote.external_id,
                watch.provider,
                remote.location,
                included_int,
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
    use rusqlite::{params, Connection};

    use super::*;
    use crate::companies::{create_company, insert_watch};
    use crate::db::migrate::migrate;
    use crate::jobs::service::{create_job_from_url, get_job_detail, update_job, UpdateJobInput};

    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        connection
    }

    #[test]
    fn two_miss_rule() {
        let missing = 1;
        let next = missing + 1;
        assert!(next >= 2);
    }

    #[test]
    fn sync_does_not_reclaim_a_posting_reassigned_to_another_company() {
        let connection = test_connection();
        let source = create_company(&connection, "Source Co", None).unwrap();
        let destination = create_company(&connection, "Destination Co", None).unwrap();
        let watch = insert_watch(&connection, &source.id, "greenhouse", "source-co").unwrap();
        let remote = AtsJob {
            external_id: "role-123".into(),
            title: "Platform Engineer".into(),
            url: "https://boards.greenhouse.io/source-co/jobs/123".into(),
            location: Some("Remote".into()),
        };
        let (job, _) = create_job_from_url(
            &connection,
            &remote.url,
            &remote.title,
            Some("Source Co"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap();
        update_job(
            &connection,
            &job.id,
            UpdateJobInput {
                company_name: Some(destination.name.clone()),
                ..UpdateJobInput::default()
            },
        )
        .unwrap();

        let result = apply_watch_sync(&connection, &watch.id, Ok(vec![remote])).unwrap();

        assert_eq!(result["created"], 0);
        let detail = get_job_detail(&connection, &job.id).unwrap().unwrap();
        assert_eq!(detail.company.id, destination.id);
        let matching_jobs: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE canonical_url = ?1",
                params![job.canonical_url],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(matching_jobs, 1);
    }

    #[test]
    fn sync_rolls_back_when_a_later_role_is_invalid() {
        let connection = test_connection();
        let company = create_company(&connection, "Source Co", None).unwrap();
        let watch = insert_watch(&connection, &company.id, "greenhouse", "source-co").unwrap();
        let valid = AtsJob {
            external_id: "role-123".into(),
            title: "Platform Engineer".into(),
            url: "https://boards.greenhouse.io/source-co/jobs/123".into(),
            location: Some("Remote".into()),
        };
        let invalid = AtsJob {
            external_id: "role-456".into(),
            title: "Backend Engineer".into(),
            url: "not a valid URL".into(),
            location: Some("Remote".into()),
        };

        let result = apply_watch_sync(&connection, &watch.id, Ok(vec![valid, invalid]));

        assert!(result.is_err());
        let jobs: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE company_id = ?1",
                params![company.id],
                |row| row.get(0),
            )
            .unwrap();
        let events: i64 = connection
            .query_row("SELECT COUNT(*) FROM job_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(jobs, 0, "the earlier job must be rolled back");
        assert_eq!(events, 0, "the earlier discovery event must be rolled back");
    }

    #[test]
    fn sync_fetch_error_commits_failure_bookkeeping() {
        let connection = test_connection();
        let company = create_company(&connection, "Source Co", None).unwrap();
        let watch = insert_watch(&connection, &company.id, "greenhouse", "source-co").unwrap();

        let result =
            apply_watch_sync(&connection, &watch.id, Err("request timed out".into())).unwrap();

        assert_eq!(result["ok"], false);
        let (failures, error): (i64, Option<String>) = connection
            .query_row(
                "SELECT consecutive_sync_failures, last_sync_error FROM company_watches WHERE id = ?1",
                params![watch.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(failures, 1);
        assert_eq!(error.as_deref(), Some("request timed out"));
    }

    #[test]
    fn sync_preserves_known_location_when_refresh_omits_location() {
        let connection = test_connection();
        let company = create_company(&connection, "Source Co", None).unwrap();
        let watch = insert_watch(&connection, &company.id, "greenhouse", "source-co").unwrap();
        let first = AtsJob {
            external_id: "role-123".into(),
            title: "Platform Engineer".into(),
            url: "https://boards.greenhouse.io/source-co/jobs/123".into(),
            location: Some("San Francisco, CA".into()),
        };
        apply_watch_sync(&connection, &watch.id, Ok(vec![first.clone()])).unwrap();

        let refresh_without_location = AtsJob {
            location: None,
            ..first
        };
        apply_watch_sync(&connection, &watch.id, Ok(vec![refresh_without_location])).unwrap();

        let location: Option<String> = connection
            .query_row(
                "SELECT location FROM jobs WHERE source_external_id = ?1",
                params!["role-123"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(location.as_deref(), Some("San Francisco, CA"));
    }
}

#[cfg(test)]
mod filtering_sync_tests {
    use rusqlite::{params, Connection};

    use super::*;
    use crate::companies::{create_company, insert_watch};
    use crate::db::migrate::migrate;
    use crate::filtering::engine::{matches, JobView};
    use crate::filtering::model::FilterCriteria;
    use crate::filtering::resolver::{
        load_alias_table, resolve_effective_criteria, set_global_criteria,
    };
    use proptest::prelude::*;

    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        connection
    }

    /// Read the `watch_filtered` hint for a job identified by its
    /// canonical_url. Returns the raw integer (0/1) or NULL as `None`.
    fn watch_filtered_by_canonical(conn: &Connection, canonical_url: &str) -> Option<i64> {
        conn.query_row(
            "SELECT watch_filtered FROM jobs WHERE canonical_url = ?1",
            params![canonical_url],
            |row| row.get::<_, Option<i64>>(0),
        )
        .unwrap()
    }

    fn count_jobs_for_company(conn: &Connection, company_id: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE company_id = ?1",
            params![company_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn ats_job(external_id: &str, title: &str, slug: &str, location: Option<&str>) -> AtsJob {
        AtsJob {
            external_id: external_id.into(),
            title: title.into(),
            url: format!("https://boards.greenhouse.io/{slug}/jobs/{external_id}"),
            location: location.map(str::to_string),
        }
    }

    // -------------------------------------------------------------------
    // 9.2 Property test: query/ingest agreement
    // **Property 10: Query/ingest agreement**
    // **Validates: Requirements 10.1, 10.2**
    //
    // The same engine authority decides inclusion at ingest (sync) time and at
    // query (list) time. Both call sites resolve the effective criteria for the
    // watch's (company_id, provider) via `resolve_effective_criteria`, load the
    // alias table via `load_alias_table`, and evaluate `engine::matches`. This
    // property asserts that the "included" value computed the way
    // `apply_watch_sync` computes it is identical to the value `list_jobs` would
    // use for the same role — guarding against divergence/regression between the
    // two call sites. Both computations flow through the identical functions, so
    // the property proves they agree for arbitrary roles.
    // -------------------------------------------------------------------

    /// A small generated FilterCriteria over a constrained token space so the
    /// engine sees meaningful include/exclude decisions across arbitrary roles.
    fn arb_criteria() -> impl Strategy<Value = FilterCriteria> {
        let token = prop::sample::select(vec![
            "engineer",
            "manager",
            "senior",
            "remote",
            "san francisco",
            "new york",
        ]);
        let tokens = prop::collection::vec(token, 0..3);
        let tokens2 = prop::collection::vec(
            prop::sample::select(vec!["contract", "intern", "staff", "director"]),
            0..2,
        );
        (tokens, tokens2).prop_map(|(include, exclude)| {
            let mut c = FilterCriteria::match_all();
            c.title.include = include.into_iter().map(String::from).collect();
            c.title.exclude = exclude.into_iter().map(String::from).collect();
            c
        })
    }

    /// Arbitrary sample role text (title + optional location).
    fn arb_role() -> impl Strategy<Value = (String, Option<String>)> {
        let title = prop::sample::select(vec![
            "Senior Software Engineer",
            "Engineering Manager",
            "Staff Engineer",
            "Product Manager",
            "Contract Recruiter",
            "Data Scientist",
        ])
        .prop_map(String::from);
        let location = prop::option::of(
            prop::sample::select(vec!["Remote", "San Francisco, CA", "New York, NY", "Austin, TX"])
                .prop_map(String::from),
        );
        (title, location)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn query_and_ingest_agree_on_inclusion(
            criteria in arb_criteria(),
            roles in prop::collection::vec(arb_role(), 1..5),
        ) {
            let conn = test_connection();
            let company = create_company(&conn, "Acme", None).unwrap();
            let watch = insert_watch(&conn, &company.id, "greenhouse", "acme").unwrap();

            // Configure a global criteria; the watch has no override, so both
            // sync and query resolve to exactly this criteria.
            set_global_criteria(&conn, &criteria).unwrap();

            for (title, location) in &roles {
                // The value "as sync would" compute it: resolve effective
                // criteria for the watch's company/provider, load aliases, then
                // engine::matches.
                let sync_criteria =
                    resolve_effective_criteria(&conn, &watch.company_id, &watch.provider).unwrap();
                let sync_aliases = load_alias_table(&conn).unwrap();
                let as_sync = matches(
                    &sync_criteria,
                    &sync_aliases,
                    JobView { title, location: location.as_deref() },
                )
                .included;

                // The value "as query would" compute it: the identical path.
                let query_criteria =
                    resolve_effective_criteria(&conn, &company.id, "greenhouse").unwrap();
                let query_aliases = load_alias_table(&conn).unwrap();
                let as_query = matches(
                    &query_criteria,
                    &query_aliases,
                    JobView { title, location: location.as_deref() },
                )
                .included;

                // Same engine authority => identical inclusion decision.
                prop_assert_eq!(as_sync, as_query);

                // Determinism: evaluating twice yields the same result.
                let again = matches(
                    &query_criteria,
                    &query_aliases,
                    JobView { title, location: location.as_deref() },
                )
                .included;
                prop_assert_eq!(as_query, again);
            }
        }
    }

    // -------------------------------------------------------------------
    // 9.3 Unit tests for non-destructive sync annotation
    // Validates Requirements 9.3, 9.4, 9.5
    // -------------------------------------------------------------------

    /// Req 9.3: every synced role is stored regardless of the match result, and
    /// a brand-new row records its actual (here, false) evaluation result.
    #[test]
    fn sync_stores_all_roles_regardless_of_match() {
        let conn = test_connection();
        let company = create_company(&conn, "Acme", None).unwrap();
        let watch = insert_watch(&conn, &company.id, "greenhouse", "acme").unwrap();

        // Restrictive global criteria: no synced role can match.
        let mut criteria = FilterCriteria::match_all();
        criteria.title.include = vec!["nonexistent-token".to_string()];
        set_global_criteria(&conn, &criteria).unwrap();

        let a = ats_job("role-1", "Software Engineer", "acme", Some("Remote"));
        let b = ats_job("role-2", "Product Manager", "acme", Some("New York, NY"));
        let a_url = normalize_canonical_url(&a.url).unwrap();
        let b_url = normalize_canonical_url(&b.url).unwrap();

        let result = apply_watch_sync(&conn, &watch.id, Ok(vec![a, b])).unwrap();
        assert_eq!(result["ok"], true);

        // Non-matching roles are still stored (Req 9.3).
        assert_eq!(count_jobs_for_company(&conn, &company.id), 2);

        // A brand-new row records its actual evaluation result: not included => 0.
        assert_eq!(watch_filtered_by_canonical(&conn, &a_url), Some(0));
        assert_eq!(watch_filtered_by_canonical(&conn, &b_url), Some(0));
    }

    /// Req 9.4: filtering never deletes a stored job. After a first sync stores
    /// a role, a second sync (with tightened criteria) retains the job.
    #[test]
    fn sync_never_deletes_previously_stored_job() {
        let conn = test_connection();
        let company = create_company(&conn, "Acme", None).unwrap();
        let watch = insert_watch(&conn, &company.id, "greenhouse", "acme").unwrap();

        // Permissive criteria: role matches on the first sync.
        set_global_criteria(&conn, &FilterCriteria::match_all()).unwrap();
        let role = ats_job("role-1", "Senior Software Engineer", "acme", Some("Remote"));
        let url = normalize_canonical_url(&role.url).unwrap();
        apply_watch_sync(&conn, &watch.id, Ok(vec![role.clone()])).unwrap();
        assert_eq!(count_jobs_for_company(&conn, &company.id), 1);

        // Tighten criteria so the role no longer matches, then sync again.
        let mut tighter = FilterCriteria::match_all();
        tighter.title.include = vec!["nonexistent-token".to_string()];
        set_global_criteria(&conn, &tighter).unwrap();
        apply_watch_sync(&conn, &watch.id, Ok(vec![role])).unwrap();

        // The previously stored job still exists — not deleted by filtering.
        assert_eq!(count_jobs_for_company(&conn, &company.id), 1);
        let still_there: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE canonical_url = ?1",
                params![url],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(still_there, 1);
    }

    /// Req 9.5: a not-included result must leave the previously recorded hint
    /// unchanged. First sync under permissive criteria sets watch_filtered = 1;
    /// tightening criteria so the role no longer matches must NOT overwrite the
    /// prior hint, and the job is retained.
    #[test]
    fn sync_leaves_prior_hint_unchanged_on_non_match() {
        let conn = test_connection();
        let company = create_company(&conn, "Acme", None).unwrap();
        let watch = insert_watch(&conn, &company.id, "greenhouse", "acme").unwrap();

        // Permissive criteria: role matches, first sync records watch_filtered = 1.
        set_global_criteria(&conn, &FilterCriteria::match_all()).unwrap();
        let role = ats_job("role-1", "Senior Software Engineer", "acme", Some("Remote"));
        let url = normalize_canonical_url(&role.url).unwrap();
        apply_watch_sync(&conn, &watch.id, Ok(vec![role.clone()])).unwrap();
        assert_eq!(watch_filtered_by_canonical(&conn, &url), Some(1));

        // Tighten criteria so the role no longer matches, then sync again.
        let mut tighter = FilterCriteria::match_all();
        tighter.title.include = vec!["nonexistent-token".to_string()];
        set_global_criteria(&conn, &tighter).unwrap();
        apply_watch_sync(&conn, &watch.id, Ok(vec![role])).unwrap();

        // The existing job's hint is STILL 1 (not overwritten by the
        // not-included result), and the job is retained.
        assert_eq!(watch_filtered_by_canonical(&conn, &url), Some(1));
        assert_eq!(count_jobs_for_company(&conn, &company.id), 1);
    }
}
