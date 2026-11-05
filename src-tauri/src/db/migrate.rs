use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

const COMPANY_STABILITY_BACKFILL_MARKER: &str = "company_employment_stability_backfill_v1";
const BACKFILL_MAX_ATTEMPTS: usize = 3;

fn company_columns(conn: &Connection) -> Result<Vec<String>> {
    Ok(conn
        .prepare("PRAGMA table_info(companies)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?)
}

/// Add one company score column if it is absent.
///
/// The schema is inspected immediately before each step rather than once for
/// the whole migration. Each ALTER is independent, so a later migration run
/// can resume after a previous run added only a prefix of the columns.
fn add_company_column(conn: &Connection, column: &str, definition: &str) -> Result<()> {
    let step = format!("company schema migration step add companies.{column}");
    if company_columns(conn)
        .with_context(|| format!("{step} failed while reading table_info"))?
        .iter()
        .any(|name| name == column)
    {
        return Ok(());
    }

    conn.execute(
        &format!("ALTER TABLE companies ADD COLUMN {column} {definition}"),
        [],
    )
    .with_context(|| format!("{step} failed"))?;

    if !company_columns(conn)
        .with_context(|| format!("{step} failed while refreshing table_info"))?
        .iter()
        .any(|name| name == column)
    {
        anyhow::bail!("{step} failed: column was not added");
    }

    Ok(())
}

fn add_company_score_columns(conn: &Connection) -> Result<()> {
    add_company_column(
        conn,
        "company_stability",
        "INTEGER CHECK (company_stability IS NULL OR (typeof(company_stability) = 'integer' AND company_stability BETWEEN 1 AND 5))",
    )?;
    add_company_column(
        conn,
        "employment_stability",
        "INTEGER CHECK (employment_stability IS NULL OR (typeof(employment_stability) = 'integer' AND employment_stability BETWEEN 1 AND 5))",
    )?;
    add_company_column(conn, "company_stability_rationale", "TEXT")?;
    add_company_column(conn, "company_stability_citations", "TEXT")?;
    add_company_column(conn, "company_stability_scored_at", "TEXT")?;
    add_company_column(
        conn,
        "company_stability_scored_by",
        "TEXT CHECK (company_stability_scored_by IS NULL OR company_stability_scored_by IN ('manual','import'))",
    )?;
    add_company_column(conn, "employment_stability_rationale", "TEXT")?;
    add_company_column(conn, "employment_stability_citations", "TEXT")?;
    add_company_column(conn, "employment_stability_scored_at", "TEXT")?;
    add_company_column(
        conn,
        "employment_stability_scored_by",
        "TEXT CHECK (employment_stability_scored_by IS NULL OR employment_stability_scored_by IN ('manual','import'))",
    )?;
    Ok(())
}

fn ensure_backfill_conflict_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS company_stability_backfill_conflicts (
          company_id TEXT PRIMARY KEY NOT NULL,
          company_name TEXT NOT NULL,
          values_json TEXT NOT NULL,
          reported INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL,
          reported_at TEXT
        );
        "#,
    )?;
    Ok(())
}

/// Apply the legacy job-cache backfill exactly once. The marker, company
/// changes, cache fan-out, and conflict rows are one transaction so an
/// interrupted or failed attempt can never make a partial migration durable.
fn backfill_company_stability_once(conn: &Connection) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .context("company stability backfill transaction failed")?;

    let result = backfill_company_stability_in_transaction(conn);
    match result {
        Ok(()) => match conn.execute_batch("COMMIT") {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error).context("company stability backfill commit failed")
            }
        },
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn backfill_company_stability_in_transaction(conn: &Connection) -> Result<()> {
    if conn
        .query_row(
            "SELECT 1 FROM app_settings WHERE key = ?1",
            [COMPANY_STABILITY_BACKFILL_MARKER],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    {
        return Ok(());
    }

    // Generate this after acquiring the write transaction so all companies
    // scored by this run receive exactly one shared UTC timestamp.
    let backfill_started_at = crate::util::now_iso();
    let companies = {
        let mut statement =
            conn.prepare("SELECT id, name, company_stability FROM companies ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    for (company_id, company_name, company_stability) in companies {
        // Existing company-owned scores are authoritative. In particular, a
        // later migration must never overwrite a score that was already set.
        if company_stability.is_some() {
            continue;
        }

        let legacy_values = {
            let mut statement = conn.prepare(
                "SELECT company_stability
                 FROM jobs
                 WHERE company_id = ?1 AND company_stability IS NOT NULL
                 GROUP BY company_stability
                 ORDER BY company_stability ASC",
            )?;
            let rows = statement.query_map([&company_id], |row| row.get::<_, i64>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        match legacy_values.as_slice() {
            [] => {}
            [value] => {
                conn.execute(
                    "UPDATE companies
                     SET company_stability = ?1,
                         company_stability_rationale = NULL,
                         company_stability_citations = NULL,
                         company_stability_scored_at = ?2,
                         company_stability_scored_by = 'manual'
                     WHERE id = ?3 AND company_stability IS NULL",
                    rusqlite::params![value, backfill_started_at, company_id],
                )?;
                // Keep the fan-out explicit even though all source values are
                // equal. This also repairs any cache row changed by a trigger
                // or concurrent legacy writer before the transaction began.
                conn.execute(
                    "UPDATE jobs SET company_stability = ?1 WHERE company_id = ?2",
                    rusqlite::params![value, company_id],
                )?;
            }
            values => {
                let values_json = serde_json::to_string(values)?;
                conn.execute(
                    "INSERT INTO company_stability_backfill_conflicts
                       (company_id, company_name, values_json, reported, created_at, reported_at)
                     VALUES (?1, ?2, ?3, 0, ?4, NULL)
                     ON CONFLICT(company_id) DO UPDATE SET
                       company_name = excluded.company_name,
                       values_json = excluded.values_json,
                       reported = 0,
                       created_at = excluded.created_at,
                       reported_at = NULL",
                    rusqlite::params![company_id, company_name, values_json, backfill_started_at],
                )?;
            }
        }
    }

    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES (?1, '1', ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        rusqlite::params![COMPANY_STABILITY_BACKFILL_MARKER, backfill_started_at],
    )?;
    Ok(())
}

fn backfill_company_stability(conn: &Connection) -> Result<()> {
    let mut last_error = None;
    for _attempt in 0..BACKFILL_MAX_ATTEMPTS {
        match backfill_company_stability_once(conn) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.expect("backfill must make at least one attempt"))
}

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
      updated_at TEXT NOT NULL,
      company_stability INTEGER CHECK (company_stability IS NULL OR (typeof(company_stability) = 'integer' AND company_stability BETWEEN 1 AND 5)),
      employment_stability INTEGER CHECK (employment_stability IS NULL OR (typeof(employment_stability) = 'integer' AND employment_stability BETWEEN 1 AND 5)),
      company_stability_rationale TEXT,
      company_stability_citations TEXT,
      company_stability_scored_at TEXT,
      company_stability_scored_by TEXT CHECK (company_stability_scored_by IS NULL OR company_stability_scored_by IN ('manual','import')),
      employment_stability_rationale TEXT,
      employment_stability_citations TEXT,
      employment_stability_scored_at TEXT,
      employment_stability_scored_by TEXT CHECK (employment_stability_scored_by IS NULL OR employment_stability_scored_by IN ('manual','import'))
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
      appeal INTEGER CHECK (appeal IS NULL OR (appeal >= 1 AND appeal <= 5)),
      application_ease INTEGER CHECK (application_ease IS NULL OR (application_ease >= 1 AND application_ease <= 5)),
      company_stability INTEGER CHECK (company_stability IS NULL OR (typeof(company_stability) = 'integer' AND company_stability BETWEEN 1 AND 5)),
      salary_min INTEGER,
      salary_max INTEGER,
      CHECK (salary_min IS NULL OR salary_min >= 0),
      CHECK (salary_max IS NULL OR salary_max >= 0),
      CHECK (salary_min IS NULL OR salary_max IS NULL OR salary_min <= salary_max)
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
    CREATE INDEX IF NOT EXISTS jobs_watch_cursor_idx
      ON jobs(is_new_from_watch, updated_at DESC, id DESC);
    CREATE INDEX IF NOT EXISTS jobs_company_watch_cursor_idx
      ON jobs(company_id, is_new_from_watch, updated_at DESC, id DESC);
    CREATE INDEX IF NOT EXISTS jobs_company_open_watch_cursor_idx
      ON jobs(company_id, posting_state, source, updated_at DESC, id DESC);
    CREATE INDEX IF NOT EXISTS job_documents_document_id_idx ON job_documents(document_id);
    CREATE INDEX IF NOT EXISTS job_documents_job_id_idx ON job_documents(job_id);
    CREATE INDEX IF NOT EXISTS company_watches_company_id_idx ON company_watches(company_id);
    "#,
    )?;

    // `CREATE TABLE IF NOT EXISTS` does not evolve an existing companies
    // table. Add each score column independently so a failed step leaves its
    // column absent while earlier successful steps remain available for the
    // next migration run.
    add_company_score_columns(conn)?;

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

    // Application ease is one nullable 1–5 score (5 = easiest to apply).
    // Existing rows stay unscored (NULL); there is no default. The CHECK
    // rejects values outside 1–5 and still allows NULL. Idempotent for older DBs.
    let has_application_ease = table_columns.iter().any(|name| name == "application_ease");
    if !has_application_ease {
        conn.execute(
            "ALTER TABLE jobs ADD COLUMN application_ease INTEGER CHECK (application_ease IS NULL OR (application_ease >= 1 AND application_ease <= 5))",
            [],
        )?;
    }
    // Company stability is one nullable 1–5 score (5 = most stable). Existing
    // rows stay unscored (NULL); there is no default and no backfill. Unlike
    // the appeal/ease CHECKs, this one also requires an INTEGER storage type,
    // so REAL values such as 2.5 (or text like '2.5') are rejected. The single
    // ALTER is atomic: on failure the column stays absent and the next run
    // retries. Idempotent for older DBs.
    let has_company_stability = table_columns.iter().any(|name| name == "company_stability");
    if !has_company_stability {
        conn.execute(
            "ALTER TABLE jobs ADD COLUMN company_stability INTEGER CHECK (company_stability IS NULL OR (typeof(company_stability) = 'integer' AND company_stability BETWEEN 1 AND 5))",
            [],
        )?;
    }

    let has_salary_min = table_columns.iter().any(|name| name == "salary_min");
    if !has_salary_min {
        conn.execute("ALTER TABLE jobs ADD COLUMN salary_min INTEGER CHECK (salary_min IS NULL OR salary_min >= 0)", [])?;
    }
    let has_salary_max = table_columns.iter().any(|name| name == "salary_max");
    if !has_salary_max {
        conn.execute("ALTER TABLE jobs ADD COLUMN salary_max INTEGER CHECK (salary_max IS NULL OR salary_max >= 0)", [])?;
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
    CREATE INDEX IF NOT EXISTS run_postings_run_ordinal_job_idx
      ON run_postings(run_id, ordinal, job_id);

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

    ensure_backfill_conflict_schema(conn)?;
    backfill_company_stability(conn)?;

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
        assert!(cols.contains(&"application_ease".to_string()));

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
        assert_eq!(
            cols.iter()
                .filter(|name| *name == "application_ease")
                .count(),
            1
        );

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
    fn company_score_schema_is_complete_strict_and_job_scoped() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        let expected = [
            "company_stability",
            "employment_stability",
            "company_stability_rationale",
            "company_stability_citations",
            "company_stability_scored_at",
            "company_stability_scored_by",
            "employment_stability_rationale",
            "employment_stability_citations",
            "employment_stability_scored_at",
            "employment_stability_scored_by",
        ];
        let columns = company_columns(&conn).unwrap();
        for column in expected {
            assert_eq!(
                columns
                    .iter()
                    .filter(|name| name.as_str() == column)
                    .count(),
                1
            );
        }

        let job_columns = conn
            .prepare("PRAGMA table_info(jobs)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!job_columns
            .iter()
            .any(|name| name == "employment_stability"));
        assert!(!job_columns
            .iter()
            .any(|name| name == "company_stability_scored_by"));

        conn.execute(
            "INSERT INTO companies (id, name, created_at, updated_at) VALUES ('c1', 'Acme', 't', 't')",
            [],
        )
        .unwrap();

        for (column, value) in [
            ("company_stability", "0"),
            ("company_stability", "6"),
            ("company_stability", "2.5"),
            ("employment_stability", "'high'"),
        ] {
            let result = conn.execute(
                &format!(
                    "UPDATE companies SET {column} = {value}, name = 'changed' WHERE id = 'c1'"
                ),
                [],
            );
            assert!(result.is_err(), "{column} = {value} should be rejected");
            let name: String = conn
                .query_row("SELECT name FROM companies WHERE id = 'c1'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(name, "Acme");
        }

        for column in [
            "company_stability_scored_by",
            "employment_stability_scored_by",
        ] {
            let result = conn.execute(
                &format!(
                    "UPDATE companies SET {column} = 'Manual', name = 'changed' WHERE id = 'c1'"
                ),
                [],
            );
            assert!(result.is_err(), "{column} should reject invalid source");
        }

        conn.execute(
            "UPDATE companies SET company_stability = 4, employment_stability = 2,
             company_stability_scored_by = 'manual', employment_stability_scored_by = 'import'
             WHERE id = 'c1'",
            [],
        )
        .unwrap();
        migrate(&conn).unwrap();
        assert_eq!(company_columns(&conn).unwrap(), columns);
        let scores: (i64, i64, String, String) = conn
            .query_row(
                "SELECT company_stability, employment_stability,
                        company_stability_scored_by, employment_stability_scored_by
                 FROM companies WHERE id = 'c1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(scores, (4, 2, "manual".to_string(), "import".to_string()));
    }

    #[test]
    fn failed_company_score_alter_reports_named_step_and_can_retry() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE companies (id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);",
        )
        .unwrap();

        add_company_column(&conn, "company_stability", "INTEGER").unwrap();
        let error = add_company_column(&conn, "employment_stability", "INTEGER CHECK (")
            .unwrap_err()
            .to_string();
        assert!(error.contains("companies.employment_stability"));
        assert!(error.contains("migration step"));
        assert!(!company_columns(&conn)
            .unwrap()
            .iter()
            .any(|name| name == "employment_stability"));

        add_company_column(
            &conn,
            "employment_stability",
            "INTEGER CHECK (employment_stability IS NULL OR (typeof(employment_stability) = 'integer' AND employment_stability BETWEEN 1 AND 5))",
        )
        .unwrap();
        assert!(company_columns(&conn)
            .unwrap()
            .iter()
            .any(|name| name == "employment_stability"));
    }

    #[test]
    fn adds_nullable_appeal_and_salary_fields_to_existing_jobs() {
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
        let salary: (Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT salary_min, salary_max FROM jobs WHERE id = 'j1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(salary, (None, None));
        conn.execute(
            "UPDATE jobs SET salary_min=100000, salary_max=150000 WHERE id='j1'",
            [],
        )
        .unwrap();
        assert!(conn
            .execute("UPDATE jobs SET salary_min=-1 WHERE id='j1'", [])
            .is_err());

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

        let application_ease: Option<i64> = conn
            .query_row(
                "SELECT application_ease FROM jobs WHERE id = 'j1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(application_ease, None);
        conn.execute("UPDATE jobs SET application_ease = 5 WHERE id = 'j1'", [])
            .unwrap();
        assert!(conn
            .execute("UPDATE jobs SET application_ease = 0 WHERE id = 'j1'", [])
            .is_err());
        assert!(conn
            .execute("UPDATE jobs SET application_ease = 6 WHERE id = 'j1'", [])
            .is_err());
        conn.execute(
            "UPDATE jobs SET application_ease = NULL WHERE id = 'j1'",
            [],
        )
        .unwrap();
        let cleared_ease: Option<i64> = conn
            .query_row(
                "SELECT application_ease FROM jobs WHERE id = 'j1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cleared_ease, None);

        migrate(&conn).unwrap();
        let cols = conn
            .prepare("PRAGMA table_info(jobs)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(cols.iter().filter(|name| *name == "appeal").count(), 1);
        assert_eq!(
            cols.iter()
                .filter(|name| *name == "application_ease")
                .count(),
            1
        );
        assert_eq!(cols.iter().filter(|name| *name == "salary_min").count(), 1);
        assert_eq!(cols.iter().filter(|name| *name == "salary_max").count(), 1);
    }

    const LEGACY_STABILITY_SCHEMA: &str = r#"
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
          notes TEXT,
          is_new_from_watch INTEGER NOT NULL DEFAULT 0,
          missing_from_sync_count INTEGER NOT NULL DEFAULT 0,
          is_favorite BOOLEAN NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          appeal INTEGER CHECK (appeal IS NULL OR (appeal >= 1 AND appeal <= 5)),
          application_ease INTEGER CHECK (application_ease IS NULL OR (application_ease >= 1 AND application_ease <= 5))
        );
        INSERT INTO companies (id, name, created_at, updated_at)
          VALUES ('c1', 'Acme', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
        INSERT INTO jobs (id, company_id, title, url, canonical_url, status, notes,
                          is_favorite, created_at, updated_at, appeal, application_ease)
        VALUES
          ('j1', 'c1', 'Engineer', 'https://e.com/1', 'https://e.com/1', 'wishlist', NULL, 0, 't1', 't1', NULL, NULL),
          ('j2', 'c1', 'Designer', 'https://e.com/2', 'https://e.com/2', 'applied', 'hi', 1, 't2', 't3', 4, 2),
          ('j3', 'c1', 'PM', 'https://e.com/3', 'https://e.com/3', 'closed', 'n', 0, 't4', 't5', 5, NULL);
    "#;

    /// Legacy `jobs` columns, read in a stable order for before/after snapshots.
    const LEGACY_JOB_COLUMNS: &str = "id, company_id, title, url, canonical_url, \
        source_external_id, status, posting_state, source, notes, is_new_from_watch, \
        missing_from_sync_count, is_favorite, created_at, updated_at, appeal, application_ease";

    fn legacy_jobs_snapshot(conn: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
        let width = LEGACY_JOB_COLUMNS.split(',').count();
        conn.prepare(&format!(
            "SELECT {LEGACY_JOB_COLUMNS} FROM jobs ORDER BY id"
        ))
        .unwrap()
        .query_map([], |r| (0..width).map(|i| r.get(i)).collect())
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    }

    fn stability_values(conn: &Connection) -> Vec<(String, Option<i64>)> {
        conn.prepare("SELECT id, company_stability FROM jobs ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn stability_column_count(conn: &Connection) -> usize {
        conn.prepare("PRAGMA table_info(jobs)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter(|name| name.as_deref() == Ok("company_stability"))
            .count()
    }

    #[test]
    fn fresh_database_has_company_stability_column() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        assert_eq!(stability_column_count(&conn), 1);
    }

    #[test]
    fn legacy_database_gains_null_company_stability_and_rerun_is_noop() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(LEGACY_STABILITY_SCHEMA).unwrap();
        let before = legacy_jobs_snapshot(&conn);

        migrate(&conn).unwrap();
        assert_eq!(stability_column_count(&conn), 1);
        assert_eq!(legacy_jobs_snapshot(&conn), before);
        assert!(stability_values(&conn).iter().all(|(_, v)| v.is_none()));

        // Values written between runs survive a second migrate unchanged.
        conn.execute("UPDATE jobs SET company_stability = 1 WHERE id = 'j1'", [])
            .unwrap();
        conn.execute("UPDATE jobs SET company_stability = 5 WHERE id = 'j2'", [])
            .unwrap();
        let stored = stability_values(&conn);
        migrate(&conn).unwrap();
        assert_eq!(stability_column_count(&conn), 1);
        assert_eq!(stability_values(&conn), stored);
        assert_eq!(legacy_jobs_snapshot(&conn), before);
    }

    #[test]
    fn invalid_company_stability_writes_are_rejected_and_row_unchanged() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(LEGACY_STABILITY_SCHEMA).unwrap();
        migrate(&conn).unwrap();
        conn.execute("UPDATE jobs SET company_stability = 3 WHERE id = 'j2'", [])
            .unwrap();
        let before = (legacy_jobs_snapshot(&conn), stability_values(&conn));

        for bad in ["0", "6", "-1", "2.5", "'2.5'", "'high'"] {
            let result = conn.execute(
                &format!(
                    "UPDATE jobs SET company_stability = {bad}, notes = 'changed' WHERE id = 'j2'"
                ),
                [],
            );
            assert!(result.is_err(), "{bad} should be rejected");
            assert_eq!(
                (legacy_jobs_snapshot(&conn), stability_values(&conn)),
                before,
                "{bad} changed the row"
            );
        }
    }

    #[test]
    fn failed_company_stability_alter_leaves_column_absent() {
        let path = std::env::temp_dir().join(format!(
            "jt-migrate-readonly-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let before = {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(LEGACY_STABILITY_SCHEMA).unwrap();
            legacy_jobs_snapshot(&conn)
        };

        // A read-only connection makes the schema change fail.
        let ro =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert!(migrate(&ro).is_err());
        drop(ro);

        let conn = Connection::open(&path).unwrap();
        assert_eq!(stability_column_count(&conn), 0);
        assert_eq!(legacy_jobs_snapshot(&conn), before);
        // The next writable run retries and adds the column.
        migrate(&conn).unwrap();
        assert_eq!(stability_column_count(&conn), 1);
        drop(conn);
        let _ = std::fs::remove_file(&path);
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
            "run_postings_run_ordinal_job_idx",
            "pce_one_authoritative_uidx",
            "pce_job_attempted_idx",
            "jobs_watch_cursor_idx",
            "jobs_company_watch_cursor_idx",
            "jobs_company_open_watch_cursor_idx",
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

#[cfg(test)]
mod task_14_tests {
    use super::*;
    use proptest::prelude::*;
    use rusqlite::types::Value;
    use rusqlite::Connection;
    use std::collections::BTreeSet;
    use std::time::Duration;

    const SCORE_COLUMNS: [(&str, &str); 10] = [
        (
            "company_stability",
            "INTEGER CHECK (company_stability IS NULL OR (typeof(company_stability) = 'integer' AND company_stability BETWEEN 1 AND 5))",
        ),
        (
            "employment_stability",
            "INTEGER CHECK (employment_stability IS NULL OR (typeof(employment_stability) = 'integer' AND employment_stability BETWEEN 1 AND 5))",
        ),
        ("company_stability_rationale", "TEXT"),
        ("company_stability_citations", "TEXT"),
        ("company_stability_scored_at", "TEXT"),
        (
            "company_stability_scored_by",
            "TEXT CHECK (company_stability_scored_by IS NULL OR company_stability_scored_by IN ('manual','import'))",
        ),
        ("employment_stability_rationale", "TEXT"),
        ("employment_stability_citations", "TEXT"),
        ("employment_stability_scored_at", "TEXT"),
        (
            "employment_stability_scored_by",
            "TEXT CHECK (employment_stability_scored_by IS NULL OR employment_stability_scored_by IN ('manual','import'))",
        ),
    ];

    fn minimal_companies(mask: &[bool]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE companies (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                careers_url TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             );
             INSERT INTO companies (id, name, created_at, updated_at)
             VALUES ('c1', 'Acme', 'before', 'before');",
        )
        .unwrap();
        for ((column, definition), present) in SCORE_COLUMNS.iter().zip(mask.iter()) {
            if *present {
                conn.execute(
                    &format!("ALTER TABLE companies ADD COLUMN {column} {definition}"),
                    [],
                )
                .unwrap();
            }
        }
        conn
    }

    fn decl_type(conn: &Connection, column: &str) -> String {
        conn.query_row(
            "SELECT type FROM pragma_table_info('companies') WHERE name = ?1",
            [column],
            |row| row.get(0),
        )
        .unwrap()
    }

    // Feature: company-employment-stability, Property 1: Company schema migration is additive and idempotent
    #[test]
    fn fresh_company_schema_has_all_score_columns() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        for (column, _) in SCORE_COLUMNS {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('companies') WHERE name = ?1",
                    [column],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "{column}");
        }
        let job_columns = conn
            .prepare("SELECT name FROM pragma_table_info('jobs')")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!job_columns
            .iter()
            .any(|name| name == "employment_stability"));
    }

    // Feature: company-employment-stability, Property 1: Company schema migration is additive and idempotent
    #[test]
    fn every_missing_company_column_subset_is_additive_and_retryable() {
        let mut masks = Vec::new();
        for bits in 0u16..(1u16 << SCORE_COLUMNS.len()) {
            masks.push(
                (0..SCORE_COLUMNS.len())
                    .map(|index| bits & (1 << index) != 0)
                    .collect::<Vec<_>>(),
            );
        }
        for mask in masks {
            let conn = minimal_companies(&mask);
            let before: (String, String) = conn
                .query_row(
                    "SELECT name, updated_at FROM companies WHERE id = 'c1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            add_company_score_columns(&conn).unwrap();
            add_company_score_columns(&conn).unwrap();
            for (column, definition) in SCORE_COLUMNS {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM pragma_table_info('companies') WHERE name = ?1",
                        [column],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(count, 1, "{column}");
                let expected_type = if definition.starts_with("INTEGER") {
                    "INTEGER"
                } else {
                    "TEXT"
                };
                assert_eq!(decl_type(&conn, column), expected_type, "{column}");
            }
            let after: (String, String) = conn
                .query_row(
                    "SELECT name, updated_at FROM companies WHERE id = 'c1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(after, before);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        // Feature: company-employment-stability, Property 1: Company schema migration is additive and idempotent
        #[test]
        fn prop_generated_legacy_company_schemas_are_additive_and_idempotent(
            mask in prop::collection::vec(any::<bool>(), SCORE_COLUMNS.len())
        ) {
            let conn = minimal_companies(&mask);
            add_company_score_columns(&conn).unwrap();
            let first_columns = company_columns(&conn).unwrap();
            add_company_score_columns(&conn).unwrap();
            prop_assert_eq!(company_columns(&conn).unwrap(), first_columns.clone());
            for (column, _) in SCORE_COLUMNS {
                prop_assert_eq!(first_columns.iter().filter(|name| name.as_str() == column).count(), 1);
            }
        }
    }

    #[test]
    fn sqlite_score_and_source_constraints_fail_atomically() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO companies (id, name, created_at, updated_at) VALUES ('c1', 'Acme', 't', 't')",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE companies SET company_stability = 3, employment_stability = 4,
             company_stability_rationale = 'keep', employment_stability_scored_by = 'manual'
             WHERE id = 'c1'",
            [],
        )
        .unwrap();

        let before: Vec<Value> = conn
            .query_row(
                "SELECT name, company_stability, employment_stability,
                        company_stability_rationale, employment_stability_scored_by
                 FROM companies WHERE id = 'c1'",
                [],
                |row| (0..5).map(|index| row.get(index)).collect(),
            )
            .unwrap();
        for statement in [
            "UPDATE companies SET company_stability = 0, name = 'changed' WHERE id = 'c1'",
            "UPDATE companies SET company_stability = 6, name = 'changed' WHERE id = 'c1'",
            "UPDATE companies SET company_stability = 2.5, name = 'changed' WHERE id = 'c1'",
            "UPDATE companies SET company_stability = '2.5', name = 'changed' WHERE id = 'c1'",
            "UPDATE companies SET company_stability = 'high', name = 'changed' WHERE id = 'c1'",
            "UPDATE companies SET employment_stability = 0, name = 'changed' WHERE id = 'c1'",
            "UPDATE companies SET employment_stability = 6, name = 'changed' WHERE id = 'c1'",
            "UPDATE companies SET company_stability_scored_by = '', name = 'changed' WHERE id = 'c1'",
            "UPDATE companies SET company_stability_scored_by = 'Manual', name = 'changed' WHERE id = 'c1'",
            "UPDATE companies SET employment_stability_scored_by = 'csv', name = 'changed' WHERE id = 'c1'",
        ] {
            assert!(conn.execute(statement, []).is_err(), "{statement}");
            let after: Vec<Value> = conn
                .query_row(
                    "SELECT name, company_stability, employment_stability,
                            company_stability_rationale, employment_stability_scored_by
                     FROM companies WHERE id = 'c1'",
                    [],
                    |row| (0..5).map(|index| row.get(index)).collect(),
                )
                .unwrap();
            assert_eq!(after, before, "failed statement changed the row");
        }
        conn.execute("UPDATE companies SET company_stability = NULL, employment_stability = 1, company_stability_scored_by = 'import' WHERE id = 'c1'", []).unwrap();
    }

    #[test]
    fn backfill_marker_rolls_back_with_company_and_cache_changes_and_retries() {
        let conn = backfill_test_database();
        insert_company(&conn, "c1", "Acme");
        insert_job(&conn, "j1", "c1", Some(4), "wishlist", 0, None);
        conn.execute_batch(
            "CREATE TRIGGER fail_backfill_cache BEFORE UPDATE OF company_stability ON jobs
             WHEN NEW.company_stability = 4 BEGIN SELECT RAISE(ABORT, 'forced backfill failure'); END;",
        )
        .unwrap();

        assert!(backfill_company_stability(&conn).is_err());
        assert_eq!(company_score(&conn, "c1"), None);
        assert_eq!(job_stability(&conn, "j1"), Some(4));
        assert!(!marker_exists(&conn));

        conn.execute_batch("DROP TRIGGER fail_backfill_cache")
            .unwrap();
        backfill_company_stability(&conn).unwrap();
        assert_eq!(company_score(&conn, "c1"), Some(4));
        assert!(marker_exists(&conn));
    }

    #[test]
    fn backfill_busy_lock_leaves_marker_absent_and_succeeds_after_retry() {
        let path = std::env::temp_dir().join(format!(
            "jt-backfill-lock-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let conn = Connection::open(&path).unwrap();
            let _ = backfill_test_database_on(&conn);
            insert_company(&conn, "c1", "Acme");
            insert_job(&conn, "j1", "c1", Some(5), "wishlist", 0, None);
            conn.execute(
                "DELETE FROM app_settings WHERE key = ?1",
                [COMPANY_STABILITY_BACKFILL_MARKER],
            )
            .unwrap();
        }
        let lock = Connection::open(&path).unwrap();
        lock.execute_batch("BEGIN IMMEDIATE").unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.busy_timeout(Duration::from_millis(1)).unwrap();
        assert!(backfill_company_stability(&conn).is_err());
        assert!(!marker_exists(&conn));
        drop(lock);
        backfill_company_stability(&conn).unwrap();
        assert_eq!(company_score(&conn, "c1"), Some(5));
        assert!(marker_exists(&conn));
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn backfill_counts_all_statuses_and_persists_sorted_conflicts() {
        let conn = backfill_test_database();
        insert_company(&conn, "empty", "Empty Co");
        insert_company(&conn, "all", "All Statuses");
        insert_company(&conn, "conflict", "Conflict Co");
        let statuses = [
            ("wishlist", 0, None),
            ("applied", 0, None),
            ("interviewing", 0, None),
            ("offer", 0, None),
            ("rejected", 0, None),
            ("withdrawn", 0, None),
            ("closed", 0, None),
            ("archived", 0, None),
            ("wishlist", 1, Some("new")),
            ("wishlist", 0, Some("dismissed")),
        ];
        for (index, (status, is_new, disposition)) in statuses.into_iter().enumerate() {
            insert_job(
                &conn,
                &format!("all-{index}"),
                "all",
                Some(4),
                status,
                is_new,
                disposition,
            );
        }
        insert_job(&conn, "conflict-1", "conflict", Some(1), "closed", 0, None);
        insert_job(
            &conn,
            "conflict-2",
            "conflict",
            Some(5),
            "archived",
            0,
            None,
        );

        backfill_company_stability(&conn).unwrap();
        assert_eq!(company_score(&conn, "empty"), None);
        assert_eq!(company_score(&conn, "all"), Some(4));
        assert!(all_job_stability(&conn, "all")
            .iter()
            .all(|value| *value == Some(4)));
        assert_eq!(company_score(&conn, "conflict"), None);
        assert_eq!(all_job_stability(&conn, "conflict"), vec![Some(1), Some(5)]);
        let conflict: (String, String, i64) = conn.query_row(
            "SELECT company_name, values_json, reported FROM company_stability_backfill_conflicts WHERE company_id = 'conflict'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(conflict, ("Conflict Co".into(), "[1,5]".into(), 0));
        assert!(marker_exists(&conn));
        let before = database_backfill_snapshot(&conn);
        backfill_company_stability(&conn).unwrap();
        assert_eq!(database_backfill_snapshot(&conn), before);
    }

    fn backfill_test_database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        backfill_test_database_on(&conn);
        conn
    }

    fn backfill_test_database_on(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE companies (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                company_stability INTEGER CHECK (company_stability IS NULL OR (typeof(company_stability) = 'integer' AND company_stability BETWEEN 1 AND 5)),
                employment_stability INTEGER CHECK (employment_stability IS NULL OR (typeof(employment_stability) = 'integer' AND employment_stability BETWEEN 1 AND 5)),
                company_stability_rationale TEXT,
                company_stability_citations TEXT,
                company_stability_scored_at TEXT,
                company_stability_scored_by TEXT CHECK (company_stability_scored_by IS NULL OR company_stability_scored_by IN ('manual','import')),
                employment_stability_rationale TEXT,
                employment_stability_citations TEXT,
                employment_stability_scored_at TEXT,
                employment_stability_scored_by TEXT CHECK (employment_stability_scored_by IS NULL OR employment_stability_scored_by IN ('manual','import'))
             );
             CREATE TABLE jobs (
                id TEXT PRIMARY KEY NOT NULL,
                company_id TEXT NOT NULL,
                status TEXT NOT NULL,
                is_new_from_watch INTEGER NOT NULL DEFAULT 0,
                watch_disposition TEXT,
                company_stability INTEGER CHECK (company_stability IS NULL OR (typeof(company_stability) = 'integer' AND company_stability BETWEEN 1 AND 5))
             );
             CREATE TABLE app_settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL, updated_at TEXT NOT NULL);",
        ).unwrap();
        ensure_backfill_conflict_schema(conn).unwrap();
    }

    fn insert_company(conn: &Connection, id: &str, name: &str) {
        conn.execute(
            "INSERT INTO companies (id, name) VALUES (?1, ?2)",
            [id, name],
        )
        .unwrap();
    }

    fn insert_job(
        conn: &Connection,
        id: &str,
        company_id: &str,
        stability: Option<i64>,
        status: &str,
        is_new: i64,
        disposition: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO jobs (id, company_id, status, is_new_from_watch, watch_disposition, company_stability) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![id, company_id, status, is_new, disposition, stability],
        ).unwrap();
    }

    fn marker_exists(conn: &Connection) -> bool {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM app_settings WHERE key = ?1)",
            [COMPANY_STABILITY_BACKFILL_MARKER],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn company_score(conn: &Connection, id: &str) -> Option<i64> {
        conn.query_row(
            "SELECT company_stability FROM companies WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn job_stability(conn: &Connection, id: &str) -> Option<i64> {
        conn.query_row(
            "SELECT company_stability FROM jobs WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn all_job_stability(conn: &Connection, company_id: &str) -> Vec<Option<i64>> {
        conn.prepare("SELECT company_stability FROM jobs WHERE company_id = ?1 ORDER BY id")
            .unwrap()
            .query_map([company_id], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn database_backfill_snapshot(
        conn: &Connection,
    ) -> (
        Vec<(String, Option<i64>, Option<String>)>,
        Vec<(String, Option<i64>, String)>,
    ) {
        let companies = conn.prepare("SELECT id, company_stability, company_stability_scored_at FROM companies ORDER BY id").unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        let conflicts = conn.prepare("SELECT company_id, reported, values_json FROM company_stability_backfill_conflicts ORDER BY company_id").unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        (companies, conflicts)
    }

    fn cache_states_strategy() -> impl Strategy<Value = Vec<Vec<Option<i64>>>> {
        prop::collection::vec(
            prop::collection::vec(prop::option::of(1i64..=5), 0..=4),
            1..=4,
        )
    }

    // Feature: company-employment-stability, Property 6: Backfill matches a reference model and is idempotent
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_backfill_matches_reference_model_and_is_idempotent(states in cache_states_strategy()) {
            let conn = backfill_test_database();
            for (company_index, caches) in states.iter().enumerate() {
                let company_id = format!("c{company_index}");
                insert_company(&conn, &company_id, &format!("Company {company_index}"));
                for (job_index, cache) in caches.iter().enumerate() {
                    insert_job(&conn, &format!("j{company_index}-{job_index}"), &company_id, *cache, "wishlist", 0, None);
                }
            }

            backfill_company_stability(&conn).unwrap();
            for (company_index, caches) in states.iter().enumerate() {
                let company_id = format!("c{company_index}");
                let distinct = caches.iter().flatten().copied().collect::<BTreeSet<_>>();
                match distinct.len() {
                    0 => {
                        prop_assert_eq!(company_score(&conn, &company_id), None);
                        prop_assert_eq!(all_job_stability(&conn, &company_id), caches.clone());
                    }
                    1 => {
                        let value = *distinct.iter().next().unwrap();
                        prop_assert_eq!(company_score(&conn, &company_id), Some(value));
                        prop_assert!(all_job_stability(&conn, &company_id).iter().all(|cache| *cache == Some(value)));
                    }
                    _ => {
                        prop_assert_eq!(company_score(&conn, &company_id), None);
                        prop_assert_eq!(all_job_stability(&conn, &company_id), caches.clone());
                        let values_json: String = conn.query_row(
                            "SELECT values_json FROM company_stability_backfill_conflicts WHERE company_id = ?1",
                            [&company_id], |row| row.get(0)
                        ).unwrap();
                        prop_assert_eq!(serde_json::from_str::<Vec<i64>>(&values_json).unwrap(), distinct.into_iter().collect::<Vec<_>>());
                    }
                }
            }
            prop_assert!(marker_exists(&conn));
            let snapshot = database_backfill_snapshot(&conn);
            backfill_company_stability(&conn).unwrap();
            prop_assert_eq!(database_backfill_snapshot(&conn), snapshot);
        }
    }
}
