use anyhow::Result;
use rusqlite::Connection;

/// Canonical SQLite schema for Job Tracker, including the
/// partial unique index on `(source, source_external_id)`.
pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
    CREATE TABLE IF NOT EXISTS companies (
      id TEXT PRIMARY KEY NOT NULL,
      name TEXT NOT NULL,
      careers_url TEXT,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS jobs (
      id TEXT PRIMARY KEY NOT NULL,
      company_id TEXT NOT NULL REFERENCES companies(id),
      title TEXT NOT NULL,
      url TEXT NOT NULL,
      canonical_url TEXT NOT NULL,
      source_external_id TEXT,
      status TEXT NOT NULL DEFAULT 'wishlist',
      applied_at TEXT,
      posting_state TEXT NOT NULL DEFAULT 'unknown',
      last_checked_at TEXT,
      last_check_result TEXT,
      source TEXT NOT NULL DEFAULT 'manual',
      notes TEXT,
      description TEXT,
      location TEXT,
      is_new_from_watch INTEGER NOT NULL DEFAULT 0,
      watch_disposition TEXT,
      missing_from_sync_count INTEGER NOT NULL DEFAULT 0,
      is_favorite BOOLEAN NOT NULL DEFAULT 0,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      appeal INTEGER CHECK (appeal IS NULL OR (appeal >= 1 AND appeal <= 5))
    );

    CREATE UNIQUE INDEX IF NOT EXISTS jobs_canonical_url_uidx ON jobs(canonical_url);
    CREATE UNIQUE INDEX IF NOT EXISTS jobs_source_external_uidx ON jobs(source, source_external_id)
      WHERE source_external_id IS NOT NULL;

    CREATE TABLE IF NOT EXISTS company_watches (
      id TEXT PRIMARY KEY NOT NULL,
      company_id TEXT NOT NULL REFERENCES companies(id),
      provider TEXT NOT NULL,
      board_slug TEXT NOT NULL,
      last_synced_at TEXT,
      consecutive_sync_failures INTEGER NOT NULL DEFAULT 0,
      last_sync_error TEXT,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS job_events (
      id TEXT PRIMARY KEY NOT NULL,
      job_id TEXT NOT NULL REFERENCES jobs(id),
      type TEXT NOT NULL,
      note TEXT,
      occurred_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS documents (
      id TEXT PRIMARY KEY NOT NULL,
      original_filename TEXT NOT NULL,
      stored_filename TEXT NOT NULL,
      mime_type TEXT NOT NULL,
      checksum TEXT NOT NULL,
      size_bytes INTEGER NOT NULL,
      imported_at TEXT NOT NULL
    );

    CREATE UNIQUE INDEX IF NOT EXISTS documents_checksum_uidx ON documents(checksum);

    CREATE TABLE IF NOT EXISTS job_documents (
      id TEXT PRIMARY KEY NOT NULL,
      job_id TEXT NOT NULL REFERENCES jobs(id),
      document_id TEXT NOT NULL REFERENCES documents(id),
      kind TEXT NOT NULL,
      used_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS careers_page_snapshots (
      id TEXT PRIMARY KEY NOT NULL,
      company_id TEXT NOT NULL REFERENCES companies(id),
      content_hash TEXT NOT NULL,
      normalized_text TEXT NOT NULL,
      captured_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS careers_page_reviews (
      id TEXT PRIMARY KEY NOT NULL,
      company_id TEXT NOT NULL REFERENCES companies(id),
      previous_hash TEXT,
      current_hash TEXT NOT NULL,
      summary TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'pending',
      created_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS app_settings (
      key TEXT PRIMARY KEY NOT NULL,
      value TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE UNIQUE INDEX IF NOT EXISTS company_watches_company_provider_slug_uidx
      ON company_watches(company_id, provider, board_slug);
    CREATE UNIQUE INDEX IF NOT EXISTS job_documents_job_document_kind_uidx
      ON job_documents(job_id, document_id, kind);
    CREATE INDEX IF NOT EXISTS job_events_job_id_type_idx ON job_events(job_id, type);
    CREATE INDEX IF NOT EXISTS job_events_occurred_at_idx ON job_events(occurred_at);
    CREATE INDEX IF NOT EXISTS jobs_company_id_updated_at_idx ON jobs(company_id, updated_at);
    CREATE INDEX IF NOT EXISTS jobs_status_updated_at_idx ON jobs(status, updated_at);
    CREATE INDEX IF NOT EXISTS jobs_posting_state_updated_at_idx ON jobs(posting_state, updated_at);
    CREATE INDEX IF NOT EXISTS jobs_is_new_from_watch_updated_at_idx ON jobs(is_new_from_watch, updated_at);
    CREATE INDEX IF NOT EXISTS jobs_watch_state_updated_at_idx ON jobs(is_new_from_watch, posting_state, updated_at);
    CREATE INDEX IF NOT EXISTS job_documents_document_id_idx ON job_documents(document_id);
    CREATE INDEX IF NOT EXISTS job_documents_job_id_idx ON job_documents(job_id);
    CREATE INDEX IF NOT EXISTS company_watches_company_id_idx ON company_watches(company_id);
    "#,
    )?;

    conn.execute_batch(
        r#"
        DROP TABLE IF EXISTS email_matches;
        DELETE FROM app_settings
        WHERE key IN (
          'gmail_client_id',
          'gmail_client_secret',
          'gmail_redirect_uri',
          'gmail_oauth_state',
          'gmail_oauth_verifier',
          'gmail_history_checkpoint'
        );
        "#,
    )?;

    // `CREATE TABLE IF NOT EXISTS` does not evolve databases created by older
    // releases, so add columns separately before reading or writing them.
    let table_columns = conn
        .prepare("PRAGMA table_info(jobs)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;

    let has_disposition = table_columns.iter().any(|name| name == "watch_disposition");
    if !has_disposition {
        conn.execute("ALTER TABLE jobs ADD COLUMN watch_disposition TEXT", [])?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS jobs_company_open_watch_idx ON jobs(company_id, posting_state, watch_disposition)",
        [],
    )?;

    let has_favorite = table_columns.iter().any(|name| name == "is_favorite");
    if !has_favorite {
        conn.execute(
            "ALTER TABLE jobs ADD COLUMN is_favorite BOOLEAN NOT NULL DEFAULT 0",
            [],
        )?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS jobs_is_favorite_updated_at_idx ON jobs(is_favorite, updated_at)",
        [],
    )?;

    let has_description = table_columns.iter().any(|name| name == "description");
    if !has_description {
        conn.execute("ALTER TABLE jobs ADD COLUMN description TEXT", [])?;
    }

    // Overall appeal is one nullable 1–5 score (5 = most appealing). Existing
    // rows stay unscored (NULL); there is no default of 3. The CHECK rejects
    // values outside 1–5 and still allows NULL. Idempotent for older DBs.
    let has_appeal = table_columns.iter().any(|name| name == "appeal");
    if !has_appeal {
        conn.execute(
            "ALTER TABLE jobs ADD COLUMN appeal INTEGER CHECK (appeal IS NULL OR (appeal >= 1 AND appeal <= 5))",
            [],
        )?;
    }

    // Watchlist job filtering: nullable hint recording the sync-time filter
    // evaluation result. NULL means not-yet-evaluated (Req 9.2). Additive,
    // idempotent, and non-destructive.
    let has_watch_filtered = table_columns.iter().any(|name| name == "watch_filtered");
    if !has_watch_filtered {
        conn.execute("ALTER TABLE jobs ADD COLUMN watch_filtered INTEGER", [])?;
    }

    // Watchlist job filtering: nullable per-watch FilterCriteria JSON. NULL
    // means inherit the global criteria. Additive, idempotent, no data loss.
    let watch_columns = conn
        .prepare("PRAGMA table_info(company_watches)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    let has_filter_criteria = watch_columns.iter().any(|name| name == "filter_criteria");
    if !has_filter_criteria {
        conn.execute(
            "ALTER TABLE company_watches ADD COLUMN filter_criteria TEXT",
            [],
        )?;
    }

    // Job check run visibility: persisted runs, per-run posting ledger, and
    // posting-check evidence history. New tables only, so `IF NOT EXISTS`
    // keeps this additive and idempotent.
    // - `runs_single_lock_owner_uidx` allows at most one lock-owning run (Req 4.8).
    // - `run_postings(run_id, job_id)` is the primary key, so each job appears
    //   once per run (Req 9.2). `job_id` deliberately has no FK to `jobs` so
    //   run history survives job deletion.
    // - `posting_check_evidence` holds authoritative and supplementary
    //   evidence (Req 7.1, 7.7); at most one authoritative row per run and job.
    conn.execute_batch(
        r#"
    CREATE TABLE IF NOT EXISTS runs (
      id TEXT PRIMARY KEY NOT NULL,
      run_type TEXT NOT NULL,
      status TEXT NOT NULL,
      trigger TEXT NOT NULL,
      source_run_id TEXT REFERENCES runs(id),
      owner_pid INTEGER NOT NULL,
      owns_runner_lock INTEGER NOT NULL DEFAULT 0,
      started_at TEXT NOT NULL,
      activated_at TEXT,
      cancel_requested_at TEXT,
      finished_at TEXT,
      duration_ms INTEGER,
      error_reason TEXT,
      stages_json TEXT NOT NULL,
      summary_json TEXT,
      last_seq INTEGER NOT NULL DEFAULT 0,
      dismissed_at TEXT,
      updated_at TEXT NOT NULL
    );
    CREATE UNIQUE INDEX IF NOT EXISTS runs_single_lock_owner_uidx
      ON runs(owns_runner_lock) WHERE owns_runner_lock = 1;
    CREATE INDEX IF NOT EXISTS runs_status_started_idx ON runs(status, started_at);

    CREATE TABLE IF NOT EXISTS run_postings (
      run_id TEXT NOT NULL REFERENCES runs(id),
      job_id TEXT NOT NULL,
      ordinal INTEGER NOT NULL,
      job_title TEXT NOT NULL,
      company_name TEXT NOT NULL,
      posting_url TEXT NOT NULL,
      state_at_start TEXT NOT NULL,
      status TEXT NOT NULL,
      posting_state TEXT,
      reason_code TEXT,
      reason TEXT,
      failure_category TEXT,
      attempted_at TEXT,
      finished_at TEXT,
      PRIMARY KEY (run_id, job_id)
    );
    CREATE INDEX IF NOT EXISTS run_postings_run_status_idx ON run_postings(run_id, status);

    CREATE TABLE IF NOT EXISTS posting_check_evidence (
      id TEXT PRIMARY KEY NOT NULL,
      run_id TEXT,
      job_id TEXT NOT NULL,
      kind TEXT NOT NULL,
      attempted_at TEXT NOT NULL,
      posting_state TEXT NOT NULL,
      reason_code TEXT NOT NULL,
      reason TEXT NOT NULL,
      evidence_version INTEGER NOT NULL,
      evidence_json TEXT NOT NULL,
      created_at TEXT NOT NULL
    );
    CREATE UNIQUE INDEX IF NOT EXISTS pce_one_authoritative_uidx
      ON posting_check_evidence(run_id, job_id)
      WHERE kind = 'authoritative' AND run_id IS NOT NULL;
    CREATE INDEX IF NOT EXISTS pce_job_attempted_idx
      ON posting_check_evidence(job_id, attempted_at);
    "#,
    )?;

    // Existing watch jobs used an event plus `is_new_from_watch` to represent
    // triage. Preserve that history in the explicit state introduced above.
    conn.execute_batch(
        r#"
        UPDATE jobs
        SET watch_disposition = CASE
          WHEN is_new_from_watch = 1 THEN 'new'
          WHEN EXISTS (
            SELECT 1 FROM job_events e
            WHERE e.job_id = jobs.id AND e.type = 'dismissed_from_watch'
          ) THEN 'dismissed'
          ELSE 'saved'
        END
        WHERE watch_disposition IS NULL
          AND source IN ('greenhouse', 'lever', 'ashby');
        "#,
    )?;

    // Watchlist job filtering: seed the default alias table and convert legacy
    // country/cities/keyword settings into structured global criteria. Both
    // steps are idempotent and guarded by key-absent checks (Req 14.1–14.7).
    crate::filtering::resolver::migrate_legacy_settings(conn)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_database_without_is_favorite_and_description() {
        let conn = Connection::open_in_memory().unwrap();
        // Simulate a legacy schema before is_favorite and description were introduced
        conn.execute_batch(
            r#"
            CREATE TABLE companies (
              id TEXT PRIMARY KEY NOT NULL,
              name TEXT NOT NULL,
              careers_url TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE jobs (
              id TEXT PRIMARY KEY NOT NULL,
              company_id TEXT NOT NULL REFERENCES companies(id),
              title TEXT NOT NULL,
              url TEXT NOT NULL,
              canonical_url TEXT NOT NULL,
              source_external_id TEXT,
              status TEXT NOT NULL DEFAULT 'wishlist',
              applied_at TEXT,
              posting_state TEXT NOT NULL DEFAULT 'unknown',
              last_checked_at TEXT,
              last_check_result TEXT,
              source TEXT NOT NULL DEFAULT 'manual',
              notes TEXT,
              location TEXT,
              is_new_from_watch INTEGER NOT NULL DEFAULT 0,
              missing_from_sync_count INTEGER NOT NULL DEFAULT 0,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE company_watches (
              id TEXT PRIMARY KEY NOT NULL,
              company_id TEXT NOT NULL REFERENCES companies(id),
              provider TEXT NOT NULL,
              board_slug TEXT NOT NULL,
              last_synced_at TEXT,
              consecutive_sync_failures INTEGER NOT NULL DEFAULT 0,
              last_sync_error TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            "#,
        )
        .unwrap();

        // Run migration on existing legacy database
        migrate(&conn).unwrap();

        // Verify column and index exist
        let cols = conn
            .prepare("PRAGMA table_info(jobs)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(cols.contains(&"is_favorite".to_string()));
        assert!(cols.contains(&"watch_disposition".to_string()));
        assert!(cols.contains(&"description".to_string()));
        assert!(cols.contains(&"watch_filtered".to_string()));
        assert!(cols.contains(&"appeal".to_string()));

        let watch_cols = conn
            .prepare("PRAGMA table_info(company_watches)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(watch_cols.contains(&"filter_criteria".to_string()));
    }

    #[test]
    fn migrate_is_idempotent_for_additive_columns() {
        let conn = Connection::open_in_memory().unwrap();
        // First run creates the canonical schema.
        migrate(&conn).unwrap();
        // Second run must not error (ALTER TABLE guards prevent duplicate columns).
        migrate(&conn).unwrap();

        let cols = conn
            .prepare("PRAGMA table_info(jobs)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            cols.iter().filter(|name| *name == "watch_filtered").count(),
            1
        );
        assert_eq!(cols.iter().filter(|name| *name == "appeal").count(), 1);

        let watch_cols = conn
            .prepare("PRAGMA table_info(company_watches)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            watch_cols
                .iter()
                .filter(|name| *name == "filter_criteria")
                .count(),
            1
        );
    }

    #[test]
    fn adds_nullable_appeal_without_scoring_existing_jobs() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE companies (
              id TEXT PRIMARY KEY NOT NULL,
              name TEXT NOT NULL,
              careers_url TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE jobs (
              id TEXT PRIMARY KEY NOT NULL,
              company_id TEXT NOT NULL,
              title TEXT NOT NULL,
              url TEXT NOT NULL,
              canonical_url TEXT NOT NULL,
              source_external_id TEXT,
              status TEXT NOT NULL DEFAULT 'wishlist',
              posting_state TEXT NOT NULL DEFAULT 'unknown',
              source TEXT NOT NULL DEFAULT 'manual',
              is_new_from_watch INTEGER NOT NULL DEFAULT 0,
              missing_from_sync_count INTEGER NOT NULL DEFAULT 0,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            INSERT INTO companies (id, name, created_at, updated_at)
              VALUES ('c1', 'Acme', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
            INSERT INTO jobs (
              id, company_id, title, url, canonical_url, status, posting_state, source,
              is_new_from_watch, missing_from_sync_count, created_at, updated_at
            ) VALUES (
              'j1', 'c1', 'Engineer', 'https://example.com/j1', 'https://example.com/j1',
              'wishlist', 'unknown', 'manual', 0, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'
            );
            "#,
        )
        .unwrap();

        migrate(&conn).unwrap();

        let appeal: Option<i64> = conn
            .query_row("SELECT appeal FROM jobs WHERE id = 'j1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(appeal, None);

        conn.execute("UPDATE jobs SET appeal = 5 WHERE id = 'j1'", [])
            .unwrap();
        assert!(conn
            .execute("UPDATE jobs SET appeal = 0 WHERE id = 'j1'", [])
            .is_err());
        assert!(conn
            .execute("UPDATE jobs SET appeal = 6 WHERE id = 'j1'", [])
            .is_err());
        conn.execute("UPDATE jobs SET appeal = NULL WHERE id = 'j1'", [])
            .unwrap();
        let cleared: Option<i64> = conn
            .query_row("SELECT appeal FROM jobs WHERE id = 'j1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(cleared, None);

        migrate(&conn).unwrap();
        let cols = conn
            .prepare("PRAGMA table_info(jobs)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(cols.iter().filter(|name| *name == "appeal").count(), 1);
    }

    fn schema_object_count(conn: &Connection, kind: &str, name: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
            rusqlite::params![kind, name],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn run_tables_and_indexes_exist_once_after_repeated_migrate() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();

        for table in ["runs", "run_postings", "posting_check_evidence"] {
            assert_eq!(schema_object_count(&conn, "table", table), 1, "{table}");
        }
        for index in [
            "runs_single_lock_owner_uidx",
            "runs_status_started_idx",
            "run_postings_run_status_idx",
            "pce_one_authoritative_uidx",
            "pce_job_attempted_idx",
        ] {
            assert_eq!(schema_object_count(&conn, "index", index), 1, "{index}");
        }
    }

    #[test]
    fn run_tables_are_added_to_legacy_database() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE companies (
              id TEXT PRIMARY KEY NOT NULL,
              name TEXT NOT NULL,
              careers_url TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
        migrate(&conn).unwrap();
        for table in ["runs", "run_postings", "posting_check_evidence"] {
            assert_eq!(schema_object_count(&conn, "table", table), 1, "{table}");
        }
    }

    fn insert_run(conn: &Connection, id: &str, owns_lock: i64) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO runs (id, run_type, status, trigger, owner_pid, owns_runner_lock,
                               started_at, stages_json, updated_at)
             VALUES (?1, 'posting_check', 'queued', 'desktop', 1, ?2,
                     '2026-01-01T00:00:00Z', '[]', '2026-01-01T00:00:00Z')",
            rusqlite::params![id, owns_lock],
        )
    }

    #[test]
    fn second_lock_owning_run_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        insert_run(&conn, "run-a", 1).unwrap();
        assert!(insert_run(&conn, "run-b", 1).is_err());
        // Non-owning runs are unconstrained.
        insert_run(&conn, "run-c", 0).unwrap();
        insert_run(&conn, "run-d", 0).unwrap();

        // Once the owner releases, another run may take the lock.
        conn.execute(
            "UPDATE runs SET owns_runner_lock = 0 WHERE id = 'run-a'",
            [],
        )
        .unwrap();
        insert_run(&conn, "run-b", 1).unwrap();
    }

    #[test]
    fn one_authoritative_evidence_row_per_run_and_job() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let insert = |id: &str, run_id: Option<&str>, kind: &str| {
            conn.execute(
                "INSERT INTO posting_check_evidence (id, run_id, job_id, kind, attempted_at,
                     posting_state, reason_code, reason, evidence_version, evidence_json, created_at)
                 VALUES (?1, ?2, 'job-1', ?3, 't', 'unknown', 'timeout', 'r', 1, '{}', 't')",
                rusqlite::params![id, run_id, kind],
            )
        };
        insert("e1", Some("run-1"), "authoritative").unwrap();
        assert!(insert("e2", Some("run-1"), "authoritative").is_err());
        insert("e3", Some("run-1"), "supplementary").unwrap();
        // check_job_posting rows have no run id and are unconstrained.
        insert("e4", None, "authoritative").unwrap();
        insert("e5", None, "authoritative").unwrap();
    }
}
