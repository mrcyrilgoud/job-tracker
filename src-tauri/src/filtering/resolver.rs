//! Filter_Resolver: settings access and effective criteria resolution.
//!
//! This module reads and writes the structured `FilterCriteria` for the two
//! configuration scopes:
//! - **Global**: stored as JSON in `app_settings` under [`GLOBAL_CRITERIA_KEY`].
//! - **Per-watch**: stored as JSON in the `company_watches.filter_criteria`
//!   column (added by the schema migration in task 6.1), or `NULL` to inherit
//!   the global criteria.
//!
//! It also loads the [`AliasTable`] from `app_settings` under
//! [`ALIAS_TABLE_KEY`], seeding the default table when absent or invalid.
//!
//! Fail-open behavior (Req 16.1, 16.2): if stored criteria fail to deserialize
//! or carry an unknown `version`, the resolver logs a warning and falls back to
//! [`FilterCriteria::match_all`] for the global scope, or `None` (inherit
//! global) for a per-watch scope. If the alias table is absent or invalid, the
//! default seed is used in memory.
//!
//! Setters normalize/trim tokens and drop empties before persisting (Req 17.3).
//!
//! [`resolve_effective_criteria`] (task 5.2) produces the effective criteria
//! for a watch identified by `(company_id, source)`: the per-watch override if
//! present, else the global criteria, else [`FilterCriteria::match_all`]. The
//! override *replaces* the global criteria — it is never blended field-by-field
//! (Req 7.1). [`CriteriaCache`] wraps that resolution so a single list call
//! resolves each `(company_id, source)` group once and reuses the value for the
//! remainder of the call; criteria changes mid-call do not affect the value
//! already resolved for a group (Req 7.4, 7.5).
//!
//! [`migrate_legacy_settings`] (task 6.2) runs once at startup to seed the
//! default alias table when absent and to convert the legacy
//! `location_country` / `location_cities` / `watch_role_keywords` settings into
//! structured global criteria. Both steps are guarded by key-absent checks so
//! repeated runs are idempotent and never overwrite user-modified data, and the
//! legacy keys are retained for safe rollback (Req 14.1–14.7).

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::util::now_iso;

use super::aliases::AliasTable;
use super::model::{self, FilterCriteria, MatchMode};

/// Schema version this resolver knows how to read. Stored criteria carrying any
/// other version are treated as invalid and fail open (Req 16.1).
pub const FILTER_CRITERIA_VERSION: u32 = 1;

/// `app_settings` key for the structured global filter criteria JSON.
pub const GLOBAL_CRITERIA_KEY: &str = "filter_criteria_v1";

/// `app_settings` key for the serialized location alias table JSON.
pub const ALIAS_TABLE_KEY: &str = "location_aliases_v1";

/// Read the global filter criteria from `app_settings`.
///
/// - Absent key => [`FilterCriteria::match_all`] (no criteria configured yet).
/// - Present but fails to deserialize OR carries an unknown `version` =>
///   log a warning and return [`FilterCriteria::match_all`] (fail open, Req 16.1).
pub fn get_global_criteria(conn: &Connection) -> AppResult<FilterCriteria> {
    let stored = get_setting(conn, GLOBAL_CRITERIA_KEY)?;
    let Some(raw) = stored else {
        return Ok(FilterCriteria::match_all());
    };
    Ok(parse_criteria_or_match_all(&raw, "global"))
}

/// Persist the global filter criteria to `app_settings`.
///
/// Tokens are trimmed and empties dropped before serializing (Req 17.3).
pub fn set_global_criteria(conn: &Connection, c: &FilterCriteria) -> AppResult<()> {
    let normalized = normalize_criteria(c);
    let value = serde_json::to_string(&normalized)
        .map_err(|e| crate::error::AppError::from(format!("failed to serialize criteria: {e}")))?;
    set_setting(conn, GLOBAL_CRITERIA_KEY, &value)
}

/// Read the per-watch override criteria for `watch_id`.
///
/// - Watch row absent, or `filter_criteria` column NULL/empty => `None`
///   (the watch inherits the global criteria).
/// - Present but invalid/unknown-version => log a warning and return `None`
///   so the watch fails open by inheriting the global scope (Req 16.1).
pub fn get_watch_criteria(conn: &Connection, watch_id: &str) -> AppResult<Option<FilterCriteria>> {
    // `query_row` yields no row when the watch id is unknown; the inner
    // `Option<String>` distinguishes a NULL column from a present value.
    let stored: Option<Option<String>> = conn
        .query_row(
            "SELECT filter_criteria FROM company_watches WHERE id = ?1",
            params![watch_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(map_sqlite)?;

    let raw = match stored {
        // Watch row does not exist => no override.
        None => return Ok(None),
        // Watch exists but column is NULL => inherit global.
        Some(None) => return Ok(None),
        Some(Some(raw)) => raw,
    };

    if raw.trim().is_empty() {
        return Ok(None);
    }

    match parse_criteria(&raw) {
        Some(criteria) => Ok(Some(criteria)),
        None => {
            log::warn!(
                "per-watch filter criteria for watch {watch_id} were invalid or had an unknown version; \
                 inheriting global criteria (fail open)"
            );
            Ok(None)
        }
    }
}

/// Set or clear the per-watch override for `watch_id`.
///
/// - `None` clears the override (sets the column to `NULL`) so the watch
///   inherits the global criteria.
/// - `Some` normalizes/trims tokens (Req 17.3) then serializes and writes.
pub fn set_watch_criteria(
    conn: &Connection,
    watch_id: &str,
    c: Option<&FilterCriteria>,
) -> AppResult<()> {
    match c {
        None => {
            conn.execute(
                "UPDATE company_watches SET filter_criteria = NULL, updated_at = ?2 WHERE id = ?1",
                params![watch_id, now_iso()],
            )
            .map_err(map_sqlite)?;
        }
        Some(criteria) => {
            let normalized = normalize_criteria(criteria);
            let value = serde_json::to_string(&normalized).map_err(|e| {
                crate::error::AppError::from(format!("failed to serialize criteria: {e}"))
            })?;
            conn.execute(
                "UPDATE company_watches SET filter_criteria = ?2, updated_at = ?3 WHERE id = ?1",
                params![watch_id, value, now_iso()],
            )
            .map_err(map_sqlite)?;
        }
    }
    Ok(())
}

/// Resolve the effective filter criteria for the watch identified by
/// `(company_id, source)`.
///
/// Resolution follows the two-level hierarchy (Req 7.1–7.3):
/// 1. If a watch row exists for this company with `provider == source` and it
///    carries a per-watch override, return **exactly** that override. The
///    override replaces the global criteria and is never blended field-by-field
///    with it (Req 7.1).
/// 2. Otherwise return the global criteria (Req 7.2). A `null`/absent per-watch
///    override yields exactly the global criteria.
/// 3. If the global criteria are absent, [`get_global_criteria`] already yields
///    [`FilterCriteria::match_all`], so an unconfigured system resolves to
///    match-all (Req 7.3).
///
/// A company may be watched on several providers; only the watch whose
/// `provider` equals `source` contributes an override. When no watch row
/// matches, resolution falls back to the global criteria.
///
/// This function is intentionally free of caching so it can be composed; use
/// [`CriteriaCache`] to resolve each group once per list call (Req 7.4, 7.5).
pub fn resolve_effective_criteria(
    conn: &Connection,
    company_id: &str,
    source: &str,
) -> AppResult<FilterCriteria> {
    // Per-watch overrides are keyed by watch id in storage, but query-time
    // integration groups by (company_id, source). A watch is identified by
    // company_id + provider + board_slug; for resolution the (company_id,
    // provider) pair selects the relevant watch row. If several board slugs
    // exist for the same provider we take the first that carries an override.
    let watch_ids = watch_ids_for(conn, company_id, source)?;
    for watch_id in watch_ids {
        if let Some(override_criteria) = get_watch_criteria(conn, &watch_id)? {
            return Ok(override_criteria);
        }
    }

    // No matching watch, or the matching watch has no override: inherit global
    // (which is itself match_all() when unconfigured).
    get_global_criteria(conn)
}

/// Return the ids of watch rows for `company_id` whose `provider` equals
/// `source`, ordered oldest-first for a stable resolution order.
fn watch_ids_for(conn: &Connection, company_id: &str, source: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM company_watches
             WHERE company_id = ?1 AND provider = ?2
             ORDER BY created_at, id",
        )
        .map_err(map_sqlite)?;
    let ids = stmt
        .query_map(params![company_id, source], |row| row.get::<_, String>(0))
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;
    Ok(ids)
}

/// A per-call cache over [`resolve_effective_criteria`].
///
/// A single list call constructs one cache and resolves each `(company_id,
/// source)` group through it. The first resolution for a key computes and
/// stores the value; every subsequent resolution for the same key returns that
/// stored value. Because the value is captured at first resolution, criteria
/// that change later within the same call do not affect the value already
/// resolved for a group (Req 7.4, 7.5).
///
/// The cache is scoped to one list call and then dropped, so a later call sees
/// the current criteria again.
#[derive(Debug, Default)]
pub struct CriteriaCache {
    map: HashMap<(String, String), FilterCriteria>,
}

impl CriteriaCache {
    /// Create an empty cache for a single list call.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve the effective criteria for `(company_id, source)`, computing it
    /// via [`resolve_effective_criteria`] on the first request for the key and
    /// returning the cached value on every subsequent request.
    pub fn resolve(
        &mut self,
        conn: &Connection,
        company_id: &str,
        source: &str,
    ) -> AppResult<FilterCriteria> {
        let key = (company_id.to_string(), source.to_string());
        if let Some(cached) = self.map.get(&key) {
            return Ok(cached.clone());
        }
        let resolved = resolve_effective_criteria(conn, company_id, source)?;
        self.map.insert(key, resolved.clone());
        Ok(resolved)
    }
}

/// Load the location alias table from `app_settings`.
///
/// Returns [`AliasTable::default_seed`] when the key is absent or the stored
/// value fails to deserialize (Req 16.2).
pub fn load_alias_table(conn: &Connection) -> AppResult<AliasTable> {
    let stored = get_setting(conn, ALIAS_TABLE_KEY)?;
    let Some(raw) = stored else {
        return Ok(AliasTable::default_seed());
    };
    match serde_json::from_str::<AliasTable>(&raw) {
        Ok(table) => Ok(table),
        Err(e) => {
            log::warn!("alias table was invalid ({e}); using default seed");
            Ok(AliasTable::default_seed())
        }
    }
}

/// Parse stored criteria JSON, returning `None` on any deserialize failure or
/// unknown `version`.
fn parse_criteria(raw: &str) -> Option<FilterCriteria> {
    let criteria: FilterCriteria = serde_json::from_str(raw).ok()?;
    if criteria.version != FILTER_CRITERIA_VERSION {
        return None;
    }
    Some(criteria)
}

/// Parse stored criteria, falling back to [`FilterCriteria::match_all`] with a
/// logged warning on failure (Req 16.1). `scope` names the failing scope for
/// the log line.
fn parse_criteria_or_match_all(raw: &str, scope: &str) -> FilterCriteria {
    match parse_criteria(raw) {
        Some(criteria) => criteria,
        None => {
            log::warn!(
                "{scope} filter criteria were invalid or had an unknown version; \
                 falling back to match-all (fail open)"
            );
            FilterCriteria::match_all()
        }
    }
}

/// Return a copy of `c` with every token list trimmed and emptied of blanks
/// (Req 17.3), leaving the structural fields (version, country, modes) intact.
fn normalize_criteria(c: &FilterCriteria) -> FilterCriteria {
    let mut out = c.clone();
    out.title.include = model::normalize_tokens(&out.title.include);
    out.title.exclude = model::normalize_tokens(&out.title.exclude);
    out.location.include = model::normalize_tokens(&out.location.include);
    out.location.exclude = model::normalize_tokens(&out.location.exclude);
    out
}

/// Read a single `app_settings` value by key.
fn get_setting(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(map_sqlite)
}

/// Upsert a single `app_settings` value, mirroring the shared settings pattern.
fn set_setting(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now_iso()],
    )
    .map_err(map_sqlite)?;
    Ok(())
}

/// `app_settings` key holding the legacy country string.
const LEGACY_COUNTRY_KEY: &str = "location_country";
/// `app_settings` key holding the legacy comma-separated cities string.
const LEGACY_CITIES_KEY: &str = "location_cities";
/// `app_settings` key holding the legacy comma/newline-separated keywords.
const LEGACY_KEYWORDS_KEY: &str = "watch_role_keywords";

/// One-time, idempotent migration run at startup (Req 14.1–14.7).
///
/// 1. **Alias table seed (Req 14.7):** when [`ALIAS_TABLE_KEY`] is absent, the
///    default [`AliasTable::default_seed`] is serialized and stored. An existing
///    (possibly user-edited) table is never overwritten.
/// 2. **Legacy -> structured criteria (Req 14.1–14.4, 14.6):** when
///    [`GLOBAL_CRITERIA_KEY`] is absent, the legacy `location_country`,
///    `location_cities`, and `watch_role_keywords` settings are mapped into a
///    structured [`FilterCriteria`]: keywords become `title.include` with
///    [`MatchMode::Word`], a non-empty country becomes `location.country`,
///    cities become `location.include`, and remote stays [`RemoteMode::Any`].
///    Non-empty legacy values are preserved without loss or reordering; absent
///    or empty keywords yield an empty title include list and are NOT treated
///    as a failure (Req 14.3).
///
/// Both writes are guarded by key-absent checks, so re-running produces
/// identical output for identical legacy input and never overwrites
/// user-modified criteria (Req 14.4). The legacy keys are retained (Req 14.5).
/// Each persisted value is fully computed before it is written, so a
/// serialization failure surfaces as an error without leaving inconsistent
/// partial state (Req 14.6).
pub fn migrate_legacy_settings(conn: &Connection) -> AppResult<()> {
    seed_alias_table_if_absent(conn)?;
    migrate_global_criteria_if_absent(conn)?;
    Ok(())
}

/// Seed the default alias table when [`ALIAS_TABLE_KEY`] is absent (Req 14.7).
fn seed_alias_table_if_absent(conn: &Connection) -> AppResult<()> {
    if get_setting(conn, ALIAS_TABLE_KEY)?.is_some() {
        // A table already exists (default or user-edited); never overwrite it.
        return Ok(());
    }
    let value = serde_json::to_string(&AliasTable::default_seed())
        .map_err(|e| AppError::from(format!("failed to serialize alias table seed: {e}")))?;
    set_setting(conn, ALIAS_TABLE_KEY, &value)
}

/// Build structured global criteria from legacy settings when
/// [`GLOBAL_CRITERIA_KEY`] is absent (Req 14.1–14.4, 14.6).
fn migrate_global_criteria_if_absent(conn: &Connection) -> AppResult<()> {
    if get_setting(conn, GLOBAL_CRITERIA_KEY)?.is_some() {
        // Structured criteria already exist (from a prior run or user edit);
        // leave them unchanged (Req 14.4).
        return Ok(());
    }

    let country = get_setting(conn, LEGACY_COUNTRY_KEY)?.unwrap_or_default();
    let cities = get_setting(conn, LEGACY_CITIES_KEY)?.unwrap_or_default();
    let keywords = get_setting(conn, LEGACY_KEYWORDS_KEY)?.unwrap_or_default();

    let criteria = build_legacy_criteria(&country, &cities, &keywords);

    // Compute the serialized value fully before writing so a failure cannot
    // leave inconsistent state (Req 14.6).
    let value = serde_json::to_string(&criteria).map_err(|e| {
        AppError::from(format!(
            "failed to serialize migrated global criteria: {e}"
        ))
    })?;
    set_setting(conn, GLOBAL_CRITERIA_KEY, &value)
}

/// Map legacy `country` / `cities` / `keywords` strings into a structured
/// [`FilterCriteria`], preserving order and dropping empties (Req 14.1–14.3).
///
/// - `keywords` split on comma and newline -> `title.include`, [`MatchMode::Word`].
/// - `country`, when non-empty after trimming, -> `location.country`.
/// - `cities` split on comma -> `location.include`.
/// - remote is [`RemoteMode::Any`] (the [`FilterCriteria::match_all`] default).
fn build_legacy_criteria(country: &str, cities: &str, keywords: &str) -> FilterCriteria {
    let mut criteria = FilterCriteria::match_all();

    // Keywords: split on comma AND newline, matching the legacy reader.
    criteria.title.include = split_and_trim(keywords, &[',', '\n']);
    criteria.title.match_mode = MatchMode::Word;

    // Cities: comma-separated.
    criteria.location.include = split_and_trim(cities, &[',']);

    // Country: only set when non-empty after trimming.
    let country_trimmed = country.trim();
    criteria.location.country = if country_trimmed.is_empty() {
        None
    } else {
        Some(country_trimmed.to_string())
    };

    criteria
}

/// Split `input` on any of `delims`, trim each piece, and drop empties while
/// preserving order.
fn split_and_trim(input: &str, delims: &[char]) -> Vec<String> {
    input
        .split(|c| delims.contains(&c))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;
    use crate::filtering::model::RemoteMode;
    use crate::util::{create_id, now_iso};

    /// An in-memory connection with the schema applied plus the per-watch
    /// `filter_criteria` column (added here to stand in for the task 6.1
    /// migration so these accessor tests are self-contained).
    fn test_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        // The task 6.1 migration adds `company_watches.filter_criteria`. Guard
        // the ALTER so these tests stay self-contained whether or not that
        // migration is present (avoids a duplicate-column error).
        let has_column: bool = conn
            .prepare("PRAGMA table_info(company_watches)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .any(|name| name == "filter_criteria");
        if !has_column {
            conn.execute(
                "ALTER TABLE company_watches ADD COLUMN filter_criteria TEXT",
                [],
            )
            .unwrap();
        }
        conn
    }

    /// Insert a company + watch and return the watch id.
    fn seed_watch(conn: &Connection) -> String {
        seed_company_watch(conn, "greenhouse", "acme").1
    }

    /// Insert a company and one watch for `provider`/`board_slug`, returning
    /// `(company_id, watch_id)`.
    fn seed_company_watch(conn: &Connection, provider: &str, board_slug: &str) -> (String, String) {
        let company_id = create_id();
        let ts = now_iso();
        conn.execute(
            "INSERT INTO companies (id, name, careers_url, created_at, updated_at)
             VALUES (?1, 'Acme', NULL, ?2, ?2)",
            params![company_id, ts],
        )
        .unwrap();
        let watch_id = insert_watch_row(conn, &company_id, provider, board_slug);
        (company_id, watch_id)
    }

    /// Insert an additional watch row for an existing company and return its id.
    fn insert_watch_row(
        conn: &Connection,
        company_id: &str,
        provider: &str,
        board_slug: &str,
    ) -> String {
        let watch_id = create_id();
        let ts = now_iso();
        conn.execute(
            "INSERT INTO company_watches
               (id, company_id, provider, board_slug, last_synced_at,
                consecutive_sync_failures, last_sync_error, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, NULL, 0, NULL, ?5, ?5)",
            params![watch_id, company_id, provider, board_slug, ts],
        )
        .unwrap();
        watch_id
    }

    fn sample_criteria() -> FilterCriteria {
        let mut c = FilterCriteria::match_all();
        c.title.include = vec!["engineer".to_string()];
        c.location.include = vec!["san francisco".to_string()];
        c.remote = RemoteMode::RemoteOnly;
        c
    }

    #[test]
    fn absent_global_returns_match_all() {
        let conn = test_connection();
        let criteria = get_global_criteria(&conn).unwrap();
        assert!(criteria.is_match_all());
    }

    #[test]
    fn global_round_trips_through_set_and_get() {
        let conn = test_connection();
        let criteria = sample_criteria();
        set_global_criteria(&conn, &criteria).unwrap();
        let loaded = get_global_criteria(&conn).unwrap();
        assert_eq!(loaded, criteria);
    }

    #[test]
    fn set_global_trims_and_drops_empty_tokens() {
        let conn = test_connection();
        let mut criteria = FilterCriteria::match_all();
        criteria.title.include = vec!["  engineer ".to_string(), "  ".to_string()];
        criteria.location.exclude = vec!["".to_string(), " remote ".to_string()];
        set_global_criteria(&conn, &criteria).unwrap();

        let loaded = get_global_criteria(&conn).unwrap();
        assert_eq!(loaded.title.include, vec!["engineer".to_string()]);
        assert_eq!(loaded.location.exclude, vec!["remote".to_string()]);
    }

    #[test]
    fn malformed_global_json_falls_back_to_match_all() {
        let conn = test_connection();
        set_setting(&conn, GLOBAL_CRITERIA_KEY, "{ not valid json").unwrap();
        let criteria = get_global_criteria(&conn).unwrap();
        assert!(criteria.is_match_all());
    }

    #[test]
    fn unknown_version_global_falls_back_to_match_all() {
        let conn = test_connection();
        set_setting(
            &conn,
            GLOBAL_CRITERIA_KEY,
            r#"{"version":999,"title":{"include":["engineer"]}}"#,
        )
        .unwrap();
        let criteria = get_global_criteria(&conn).unwrap();
        assert!(criteria.is_match_all());
    }

    #[test]
    fn set_watch_some_then_get_returns_it() {
        let conn = test_connection();
        let watch_id = seed_watch(&conn);
        let criteria = sample_criteria();

        set_watch_criteria(&conn, &watch_id, Some(&criteria)).unwrap();
        let loaded = get_watch_criteria(&conn, &watch_id).unwrap();
        assert_eq!(loaded, Some(criteria));
    }

    #[test]
    fn set_watch_none_clears_override() {
        let conn = test_connection();
        let watch_id = seed_watch(&conn);

        set_watch_criteria(&conn, &watch_id, Some(&sample_criteria())).unwrap();
        assert!(get_watch_criteria(&conn, &watch_id).unwrap().is_some());

        set_watch_criteria(&conn, &watch_id, None).unwrap();
        assert_eq!(get_watch_criteria(&conn, &watch_id).unwrap(), None);
    }

    #[test]
    fn get_watch_criteria_absent_watch_returns_none() {
        let conn = test_connection();
        assert_eq!(get_watch_criteria(&conn, "does-not-exist").unwrap(), None);
    }

    #[test]
    fn malformed_watch_json_fails_open_to_none() {
        let conn = test_connection();
        let watch_id = seed_watch(&conn);
        conn.execute(
            "UPDATE company_watches SET filter_criteria = ?2 WHERE id = ?1",
            params![watch_id, "{ not valid"],
        )
        .unwrap();
        assert_eq!(get_watch_criteria(&conn, &watch_id).unwrap(), None);
    }

    #[test]
    fn set_watch_trims_and_drops_empty_tokens() {
        let conn = test_connection();
        let watch_id = seed_watch(&conn);
        let mut criteria = FilterCriteria::match_all();
        criteria.title.include = vec![" manager ".to_string(), "".to_string()];
        set_watch_criteria(&conn, &watch_id, Some(&criteria)).unwrap();

        let loaded = get_watch_criteria(&conn, &watch_id).unwrap().unwrap();
        assert_eq!(loaded.title.include, vec!["manager".to_string()]);
    }

    #[test]
    fn load_alias_table_returns_default_seed_when_absent() {
        let conn = test_connection();
        let table = load_alias_table(&conn).unwrap();
        let seed = AliasTable::default_seed();
        assert_eq!(table.version, seed.version);
        assert_eq!(table.remote_tokens, seed.remote_tokens);
        assert!(table.regions.contains_key("bay area"));
    }

    #[test]
    fn load_alias_table_falls_back_on_invalid_json() {
        let conn = test_connection();
        set_setting(&conn, ALIAS_TABLE_KEY, "not json").unwrap();
        let table = load_alias_table(&conn).unwrap();
        assert_eq!(table.version, AliasTable::default_seed().version);
    }

    /// A distinct global criteria value, so tests can tell it apart from an
    /// override and from match_all().
    fn global_only_criteria() -> FilterCriteria {
        let mut c = FilterCriteria::match_all();
        c.title.include = vec!["director".to_string()];
        c
    }

    #[test]
    fn resolve_returns_watch_override_exactly_not_merged() {
        let conn = test_connection();
        let (company_id, watch_id) = seed_company_watch(&conn, "greenhouse", "acme");

        // A global value that would blend visibly if the resolver merged.
        set_global_criteria(&conn, &global_only_criteria()).unwrap();
        let override_criteria = sample_criteria();
        set_watch_criteria(&conn, &watch_id, Some(&override_criteria)).unwrap();

        let resolved = resolve_effective_criteria(&conn, &company_id, "greenhouse").unwrap();
        // Exactly the override, with no trace of the global title include.
        assert_eq!(resolved, override_criteria);
        assert_eq!(resolved.title.include, vec!["engineer".to_string()]);
    }

    #[test]
    fn resolve_without_override_returns_global() {
        let conn = test_connection();
        let (company_id, _watch_id) = seed_company_watch(&conn, "greenhouse", "acme");
        let global = global_only_criteria();
        set_global_criteria(&conn, &global).unwrap();

        let resolved = resolve_effective_criteria(&conn, &company_id, "greenhouse").unwrap();
        assert_eq!(resolved, global);
    }

    #[test]
    fn resolve_no_matching_watch_falls_back_to_global() {
        let conn = test_connection();
        let (company_id, watch_id) = seed_company_watch(&conn, "greenhouse", "acme");
        // Override exists only for greenhouse; a different source must ignore it.
        set_watch_criteria(&conn, &watch_id, Some(&sample_criteria())).unwrap();
        let global = global_only_criteria();
        set_global_criteria(&conn, &global).unwrap();

        let resolved = resolve_effective_criteria(&conn, &company_id, "lever").unwrap();
        assert_eq!(resolved, global);
    }

    #[test]
    fn resolve_no_watch_and_no_global_returns_match_all() {
        let conn = test_connection();
        let resolved = resolve_effective_criteria(&conn, "unknown-company", "greenhouse").unwrap();
        assert!(resolved.is_match_all());
    }

    #[test]
    fn resolve_matches_provider_among_multiple_watches() {
        let conn = test_connection();
        let (company_id, gh_watch) = seed_company_watch(&conn, "greenhouse", "acme");
        let lever_watch = insert_watch_row(&conn, &company_id, "lever", "acme");

        let mut gh_criteria = FilterCriteria::match_all();
        gh_criteria.title.include = vec!["greenhouse-role".to_string()];
        let mut lever_criteria = FilterCriteria::match_all();
        lever_criteria.title.include = vec!["lever-role".to_string()];
        set_watch_criteria(&conn, &gh_watch, Some(&gh_criteria)).unwrap();
        set_watch_criteria(&conn, &lever_watch, Some(&lever_criteria)).unwrap();

        assert_eq!(
            resolve_effective_criteria(&conn, &company_id, "greenhouse").unwrap(),
            gh_criteria
        );
        assert_eq!(
            resolve_effective_criteria(&conn, &company_id, "lever").unwrap(),
            lever_criteria
        );
    }

    #[test]
    fn cache_resolves_and_returns_cached_value() {
        let conn = test_connection();
        let (company_id, watch_id) = seed_company_watch(&conn, "greenhouse", "acme");
        let override_criteria = sample_criteria();
        set_watch_criteria(&conn, &watch_id, Some(&override_criteria)).unwrap();

        let mut cache = CriteriaCache::new();
        let first = cache.resolve(&conn, &company_id, "greenhouse").unwrap();
        let second = cache.resolve(&conn, &company_id, "greenhouse").unwrap();
        assert_eq!(first, override_criteria);
        assert_eq!(second, override_criteria);
    }

    #[test]
    fn cache_keeps_first_resolved_value_when_criteria_change_mid_call() {
        let conn = test_connection();
        let (company_id, watch_id) = seed_company_watch(&conn, "greenhouse", "acme");

        // Initial override that the cache should capture.
        let initial = sample_criteria();
        set_watch_criteria(&conn, &watch_id, Some(&initial)).unwrap();

        let mut cache = CriteriaCache::new();
        let resolved_first = cache.resolve(&conn, &company_id, "greenhouse").unwrap();
        assert_eq!(resolved_first, initial);

        // Underlying criteria change mid-call.
        let mut changed = FilterCriteria::match_all();
        changed.title.include = vec!["changed-role".to_string()];
        set_watch_criteria(&conn, &watch_id, Some(&changed)).unwrap();

        // Same cache still yields the value captured at first resolution (Req 7.5).
        let resolved_again = cache.resolve(&conn, &company_id, "greenhouse").unwrap();
        assert_eq!(resolved_again, initial);
        assert_ne!(resolved_again, changed);

        // A fresh cache reflects the new state, confirming the cache is the
        // only reason the value stayed stable.
        let mut fresh_cache = CriteriaCache::new();
        let fresh = fresh_cache.resolve(&conn, &company_id, "greenhouse").unwrap();
        assert_eq!(fresh, changed);
    }

    #[test]
    fn cache_isolates_distinct_keys() {
        let conn = test_connection();
        let (company_id, gh_watch) = seed_company_watch(&conn, "greenhouse", "acme");
        let lever_watch = insert_watch_row(&conn, &company_id, "lever", "acme");

        let mut gh_criteria = FilterCriteria::match_all();
        gh_criteria.title.include = vec!["gh".to_string()];
        let mut lever_criteria = FilterCriteria::match_all();
        lever_criteria.title.include = vec!["lever".to_string()];
        set_watch_criteria(&conn, &gh_watch, Some(&gh_criteria)).unwrap();
        set_watch_criteria(&conn, &lever_watch, Some(&lever_criteria)).unwrap();

        let mut cache = CriteriaCache::new();
        assert_eq!(
            cache.resolve(&conn, &company_id, "greenhouse").unwrap(),
            gh_criteria
        );
        assert_eq!(
            cache.resolve(&conn, &company_id, "lever").unwrap(),
            lever_criteria
        );
    }

    // --- migrate_legacy_settings (task 6.2) -------------------------------

    /// A bare connection with only the `app_settings` table and none of the
    /// migration side effects. `test_connection()` runs the full `migrate()`,
    /// which now invokes `migrate_legacy_settings` itself and would pre-seed
    /// the very keys these tests exercise; this helper isolates the unit under
    /// test so it starts from a clean, unseeded settings store.
    fn settings_only_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (
               key TEXT PRIMARY KEY NOT NULL,
               value TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );",
        )
        .unwrap();
        conn
    }

    /// Seed the three legacy settings keys directly in `app_settings`.
    fn seed_legacy(conn: &Connection, country: &str, cities: &str, keywords: &str) {
        set_setting(conn, LEGACY_COUNTRY_KEY, country).unwrap();
        set_setting(conn, LEGACY_CITIES_KEY, cities).unwrap();
        set_setting(conn, LEGACY_KEYWORDS_KEY, keywords).unwrap();
    }

    #[test]
    fn migration_maps_legacy_fields_into_structured_criteria() {
        let conn = settings_only_connection();
        seed_legacy(&conn, "United States", "San Francisco, Seattle", "engineer, manager");

        migrate_legacy_settings(&conn).unwrap();

        let criteria = get_global_criteria(&conn).unwrap();
        assert_eq!(
            criteria.title.include,
            vec!["engineer".to_string(), "manager".to_string()]
        );
        assert_eq!(criteria.title.match_mode, MatchMode::Word);
        assert_eq!(criteria.location.country, Some("United States".to_string()));
        assert_eq!(
            criteria.location.include,
            vec!["San Francisco".to_string(), "Seattle".to_string()]
        );
        assert_eq!(criteria.remote, RemoteMode::Any);
    }

    #[test]
    fn migration_splits_keywords_on_comma_and_newline() {
        let conn = settings_only_connection();
        seed_legacy(&conn, "", "", "engineer,\n manager \n, staff");

        migrate_legacy_settings(&conn).unwrap();

        let criteria = get_global_criteria(&conn).unwrap();
        assert_eq!(
            criteria.title.include,
            vec![
                "engineer".to_string(),
                "manager".to_string(),
                "staff".to_string()
            ]
        );
    }

    #[test]
    fn migration_empty_keywords_yields_empty_title_include_without_error() {
        let conn = settings_only_connection();
        // Country/cities present, keywords empty (Req 14.3).
        seed_legacy(&conn, "Canada", "Toronto", "   ");

        migrate_legacy_settings(&conn).unwrap();

        let criteria = get_global_criteria(&conn).unwrap();
        assert!(criteria.title.include.is_empty());
        assert_eq!(criteria.location.country, Some("Canada".to_string()));
        assert_eq!(criteria.location.include, vec!["Toronto".to_string()]);
    }

    #[test]
    fn migration_absent_legacy_keys_produce_match_all_criteria() {
        let conn = settings_only_connection();
        // No legacy keys at all.
        migrate_legacy_settings(&conn).unwrap();

        let criteria = get_global_criteria(&conn).unwrap();
        assert!(criteria.title.include.is_empty());
        assert_eq!(criteria.location.country, None);
        assert!(criteria.location.include.is_empty());
    }

    #[test]
    fn migration_empty_country_leaves_country_none() {
        let conn = settings_only_connection();
        seed_legacy(&conn, "   ", "Austin", "engineer");

        migrate_legacy_settings(&conn).unwrap();

        let criteria = get_global_criteria(&conn).unwrap();
        assert_eq!(criteria.location.country, None);
    }

    #[test]
    fn migration_is_idempotent_and_stable() {
        let conn = settings_only_connection();
        seed_legacy(&conn, "United States", "San Francisco, Seattle", "engineer, manager");

        migrate_legacy_settings(&conn).unwrap();
        let first = get_setting(&conn, GLOBAL_CRITERIA_KEY).unwrap();

        migrate_legacy_settings(&conn).unwrap();
        let second = get_setting(&conn, GLOBAL_CRITERIA_KEY).unwrap();

        // Identical serialized output on repeated runs (Req 14.4).
        assert_eq!(first, second);
    }

    #[test]
    fn migration_does_not_overwrite_user_modified_criteria() {
        let conn = settings_only_connection();
        // A user has already customized structured criteria.
        let mut user = FilterCriteria::match_all();
        user.title.include = vec!["director".to_string()];
        set_global_criteria(&conn, &user).unwrap();

        // Legacy values that would map to something different.
        seed_legacy(&conn, "United States", "Seattle", "engineer");

        migrate_legacy_settings(&conn).unwrap();

        // The user's criteria are left untouched (Req 14.4).
        let loaded = get_global_criteria(&conn).unwrap();
        assert_eq!(loaded, user);
        assert_eq!(loaded.title.include, vec!["director".to_string()]);
    }

    #[test]
    fn migration_retains_legacy_keys() {
        let conn = settings_only_connection();
        seed_legacy(&conn, "United States", "Seattle", "engineer");

        migrate_legacy_settings(&conn).unwrap();

        // Legacy keys survive for rollback (Req 14.5).
        assert_eq!(
            get_setting(&conn, LEGACY_COUNTRY_KEY).unwrap(),
            Some("United States".to_string())
        );
        assert_eq!(
            get_setting(&conn, LEGACY_CITIES_KEY).unwrap(),
            Some("Seattle".to_string())
        );
        assert_eq!(
            get_setting(&conn, LEGACY_KEYWORDS_KEY).unwrap(),
            Some("engineer".to_string())
        );
    }

    #[test]
    fn migration_seeds_alias_table_when_absent() {
        let conn = settings_only_connection();
        assert_eq!(get_setting(&conn, ALIAS_TABLE_KEY).unwrap(), None);

        migrate_legacy_settings(&conn).unwrap();

        let stored = get_setting(&conn, ALIAS_TABLE_KEY).unwrap();
        assert!(stored.is_some());
        // Round-trips to the default seed's coverage.
        let table = load_alias_table(&conn).unwrap();
        assert_eq!(table.version, AliasTable::default_seed().version);
        assert!(table.regions.contains_key("bay area"));
    }

    #[test]
    fn migration_does_not_overwrite_existing_alias_table() {
        let conn = settings_only_connection();
        // Simulate a user-edited alias table already present.
        let mut edited = AliasTable::default_seed();
        edited.remote_tokens = vec!["custom-remote".to_string()];
        let raw = serde_json::to_string(&edited).unwrap();
        set_setting(&conn, ALIAS_TABLE_KEY, &raw).unwrap();

        migrate_legacy_settings(&conn).unwrap();

        let loaded = load_alias_table(&conn).unwrap();
        assert_eq!(loaded.remote_tokens, vec!["custom-remote".to_string()]);
    }

    // --- Property tests ---------------------------------------------------

    use proptest::prelude::*;

    /// Build a small [`FilterCriteria`] from a title-include token and an
    /// optional location country. Kept intentionally tiny so the generated
    /// space stays cheap and the distinctness check is easy to reason about.
    fn criteria_from(title: &str, country: Option<&str>) -> FilterCriteria {
        let mut c = FilterCriteria::match_all();
        c.title.include = vec![title.to_string()];
        c.location.country = country.map(|s| s.to_string());
        c
    }

    proptest! {
        /// **Property 9: Override replaces, not merges** (Req 7.1, 7.2).
        ///
        /// Given a global criteria G and a per-watch override O that differ,
        /// `resolve_effective_criteria` for the watched (company, source) returns
        /// **exactly** O — never a field-blend of G and O. With no override, it
        /// returns **exactly** G.
        #[test]
        fn prop_override_replaces_not_merges(
            // Distinct title tokens so G and O never coincide.
            g_title in "[a-z]{3,8}",
            o_title in "[a-z]{3,8}",
            // Optional differing country context.
            g_country in proptest::option::of("[A-Z][a-z]{2,8}"),
            o_country in proptest::option::of("[A-Z][a-z]{2,8}"),
        ) {
            // Constrain to genuinely distinct criteria: differ in title and/or
            // location. If the random draw made them identical, nudge O.
            let global = criteria_from(&g_title, g_country.as_deref());
            let mut override_criteria = criteria_from(&o_title, o_country.as_deref());
            if override_criteria == global {
                override_criteria.title.include = vec![format!("{o_title}x")];
            }
            prop_assume!(override_criteria != global);

            let conn = test_connection();
            let (company_id, watch_id) = seed_company_watch(&conn, "greenhouse", "acme");

            set_global_criteria(&conn, &global).unwrap();

            // With an override present: resolve == O exactly (never blended).
            set_watch_criteria(&conn, &watch_id, Some(&override_criteria)).unwrap();
            let resolved = resolve_effective_criteria(&conn, &company_id, "greenhouse").unwrap();
            // Normalized override is what was persisted; compare against it.
            let expected_override = normalize_criteria(&override_criteria);
            prop_assert_eq!(&resolved, &expected_override);
            // No trace of the global title include leaked through.
            prop_assert!(!resolved
                .title
                .include
                .iter()
                .any(|t| global.title.include.contains(t))
                || expected_override.title.include == global.title.include);
            // Country is exactly the override's, never the global's (when they differ).
            if expected_override.location.country != global.location.country {
                prop_assert_eq!(&resolved.location.country, &expected_override.location.country);
            }

            // With NO override: resolve == G exactly.
            set_watch_criteria(&conn, &watch_id, None).unwrap();
            let resolved_global = resolve_effective_criteria(&conn, &company_id, "greenhouse").unwrap();
            let expected_global = normalize_criteria(&global);
            prop_assert_eq!(&resolved_global, &expected_global);
        }

        /// **Property 8: Idempotent migration** (Req 14.4).
        ///
        /// For arbitrary bounded legacy country/cities/keywords strings,
        /// running `migrate_legacy_settings` twice leaves the stored global
        /// criteria identical after the second run as after the first.
        #[test]
        fn prop_migration_is_idempotent(
            country in "[A-Za-z ,]{0,20}",
            cities in "[A-Za-z ,\n]{0,30}",
            keywords in "[A-Za-z ,\n]{0,30}",
        ) {
            let conn = settings_only_connection();
            seed_legacy(&conn, &country, &cities, &keywords);

            migrate_legacy_settings(&conn).unwrap();
            let first = get_setting(&conn, GLOBAL_CRITERIA_KEY).unwrap();

            migrate_legacy_settings(&conn).unwrap();
            let second = get_setting(&conn, GLOBAL_CRITERIA_KEY).unwrap();

            // Stored value is byte-identical across the repeated run (Req 14.4).
            prop_assert_eq!(first, second);
        }

        /// **Property 8 (guard): migration never overwrites user-modified
        /// criteria** (Req 14.4). When a structured `filter_criteria_v1` value
        /// already exists, running the migration over arbitrary legacy input
        /// leaves that value untouched.
        #[test]
        fn prop_migration_never_overwrites_existing_criteria(
            user_title in "[a-z]{3,10}",
            country in "[A-Za-z ,]{0,20}",
            cities in "[A-Za-z ,\n]{0,30}",
            keywords in "[A-Za-z ,\n]{0,30}",
        ) {
            let conn = settings_only_connection();

            // A user has already customized structured criteria.
            let mut user = FilterCriteria::match_all();
            user.title.include = vec![user_title];
            set_global_criteria(&conn, &user).unwrap();
            let before = get_setting(&conn, GLOBAL_CRITERIA_KEY).unwrap();

            // Arbitrary legacy values that would otherwise map to something else.
            seed_legacy(&conn, &country, &cities, &keywords);
            migrate_legacy_settings(&conn).unwrap();

            let after = get_setting(&conn, GLOBAL_CRITERIA_KEY).unwrap();
            prop_assert_eq!(before, after);
        }
    }
}
