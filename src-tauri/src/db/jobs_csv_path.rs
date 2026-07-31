use std::env;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Serialize;

use crate::db::settings::{delete_setting, get_setting, set_setting};
use crate::error::{AppError, AppResult};
use crate::jobs::csv::{
    export_jobs_csv, import_jobs_csv, jobs_csv_sync_path, ImportMode, ImportResult,
};

pub const JOBS_CSV_PATH_SETTING_KEY: &str = "jobs_csv_path";
pub const JOBS_CSV_ENV_VAR: &str = "JOB_TRACKER_JOBS_CSV";

/// Default CSV location under the fixed data directory.
pub fn default_jobs_csv_path(data_dir: &Path) -> PathBuf {
    data_dir.join("jobs.csv")
}

/// Resolution order: env (absolute) → app_settings → `{dataDir}/jobs.csv`.
pub fn resolve_jobs_csv_path(data_dir: &Path, conn: &Connection) -> AppResult<PathBuf> {
    if let Some(path) = env_jobs_csv_path()? {
        return Ok(path);
    }
    if let Some(stored) = get_setting(conn, JOBS_CSV_PATH_SETTING_KEY)? {
        let path = PathBuf::from(&stored);
        if path.is_absolute() {
            return Ok(path);
        }
        log::warn!(
            "ignoring non-absolute {JOBS_CSV_PATH_SETTING_KEY} setting: {stored}"
        );
    }
    Ok(default_jobs_csv_path(data_dir))
}

pub fn env_jobs_csv_path() -> AppResult<Option<PathBuf>> {
    match env::var(JOBS_CSV_ENV_VAR) {
        Ok(value) if !value.trim().is_empty() => {
            let path = PathBuf::from(value.trim());
            if !path.is_absolute() {
                return Err(AppError::from(format!(
                    "{JOBS_CSV_ENV_VAR} must be an absolute path"
                )));
            }
            Ok(Some(path))
        }
        Ok(_) | Err(_) => Ok(None),
    }
}

/// Expand `~` once and require an absolute path. Store absolute only.
pub fn normalize_absolute_jobs_csv_path(input: &str) -> AppResult<PathBuf> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AppError::from("CSV path must not be empty"));
    }
    let expanded = expand_tilde(trimmed);
    let path = PathBuf::from(&expanded);
    if !path.is_absolute() {
        return Err(AppError::from(
            "CSV path must be absolute (relative paths are rejected)",
        ));
    }
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Ok(canonical_parent) = parent.canonicalize() {
                if let Some(name) = path.file_name() {
                    return Ok(canonical_parent.join(name));
                }
            }
        }
    }
    Ok(path)
}

fn expand_tilde(input: &str) -> String {
    if input == "~" {
        return home_dir()
            .map(|h| h.display().to_string())
            .unwrap_or_else(|| input.to_string());
    }
    if let Some(rest) = input.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest).display().to_string();
        }
    }
    input.to_string()
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetJobsCsvMode {
    RelocateExport,
    LinkWithSidecar,
    LinkWithoutSidecar,
}

impl SetJobsCsvMode {
    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "relocate_export" => Ok(Self::RelocateExport),
            "link_with_sidecar" => Ok(Self::LinkWithSidecar),
            "link_without_sidecar" => Ok(Self::LinkWithoutSidecar),
            other => Err(AppError::from(format!(
                "Unknown CSV path mode '{other}' (expected relocate_export, link_with_sidecar, or link_without_sidecar)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithoutSidecarAction {
    ExportOverwrite,
    OverwriteEditable,
}

impl WithoutSidecarAction {
    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "export_overwrite" => Ok(Self::ExportOverwrite),
            "overwrite_editable" => Ok(Self::OverwriteEditable),
            other => Err(AppError::from(format!(
                "Unknown without-sidecar action '{other}' (expected export_overwrite or overwrite_editable)"
            ))),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobsCsvPathInfo {
    pub path: String,
    pub is_default: bool,
    pub env_override: bool,
    pub default_path: String,
    pub has_sidecar: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetJobsCsvPathResult {
    pub path: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import: Option<ImportResult>,
    pub env_override: bool,
}

pub fn jobs_csv_path_info(data_dir: &Path, conn: &Connection) -> AppResult<JobsCsvPathInfo> {
    let default_path = default_jobs_csv_path(data_dir);
    let env_override = env_jobs_csv_path()?.is_some();
    let path = resolve_jobs_csv_path(data_dir, conn)?;
    let is_default = path == default_path && !env_override && get_setting(conn, JOBS_CSV_PATH_SETTING_KEY)?.is_none();
    let has_sidecar = jobs_csv_sync_path(&path).exists();
    Ok(JobsCsvPathInfo {
        path: path.display().to_string(),
        is_default,
        env_override,
        default_path: default_path.display().to_string(),
        has_sidecar,
    })
}

pub fn set_jobs_csv_path(
    data_dir: &Path,
    db_path: &Path,
    conn: &Connection,
    path_input: &str,
    mode: SetJobsCsvMode,
    dry_run: bool,
    confirm: bool,
    without_sidecar_action: Option<WithoutSidecarAction>,
) -> AppResult<SetJobsCsvPathResult> {
    let new_path = normalize_absolute_jobs_csv_path(path_input)?;
    let env_override = env_jobs_csv_path()?.is_some();
    let sidecar = jobs_csv_sync_path(&new_path);
    let sidecar_present = sidecar.exists();

    let (action, import) = match mode {
        SetJobsCsvMode::RelocateExport => {
            export_jobs_csv(conn, &new_path, None)?;
            ("relocated_export".to_string(), None)
        }
        SetJobsCsvMode::LinkWithSidecar => {
            if !new_path.exists() {
                return Err(AppError::from(
                    "link_with_sidecar requires an existing CSV file",
                ));
            }
            if !sidecar_present {
                return Err(AppError::from(
                    "No sync sidecar found beside this CSV. Use link_without_sidecar with export_overwrite or confirmed overwrite_editable instead.",
                ));
            }
            if dry_run {
                let preview = import_jobs_csv(db_path, &new_path, None, true, ImportMode::Merge)?;
                return Ok(SetJobsCsvPathResult {
                    path: new_path.display().to_string(),
                    action: "dry_run".to_string(),
                    import: Some(preview),
                    env_override,
                });
            }
            if !confirm {
                return Err(AppError::from(
                    "link_with_sidecar requires dry_run=true (preview) or confirm=true (apply merge)",
                ));
            }
            let imported = import_jobs_csv(db_path, &new_path, None, false, ImportMode::Merge)?;
            ("linked_with_sidecar".to_string(), Some(imported))
        }
        SetJobsCsvMode::LinkWithoutSidecar => {
            if sidecar_present {
                return Err(AppError::from(
                    "A sync sidecar exists for this CSV. Use link_with_sidecar instead.",
                ));
            }
            let action = without_sidecar_action.ok_or_else(|| {
                AppError::from(
                    "Refusing silent merge without a sync sidecar. Pass withoutSidecarAction: export_overwrite (replace file from DB) or overwrite_editable (destructive import after confirm).",
                )
            })?;
            match action {
                WithoutSidecarAction::ExportOverwrite => {
                    export_jobs_csv(conn, &new_path, None)?;
                    ("export_overwrite".to_string(), None)
                }
                WithoutSidecarAction::OverwriteEditable => {
                    if !confirm {
                        return Err(AppError::from(
                            "overwrite_editable requires confirm=true (destructive import)",
                        ));
                    }
                    if !new_path.exists() {
                        return Err(AppError::from(
                            "overwrite_editable requires an existing CSV file",
                        ));
                    }
                    let imported = import_jobs_csv(
                        db_path,
                        &new_path,
                        None,
                        false,
                        ImportMode::OverwriteEditable,
                    )?;
                    ("overwrite_editable".to_string(), Some(imported))
                }
            }
        }
    };

    set_setting(
        conn,
        JOBS_CSV_PATH_SETTING_KEY,
        &new_path.display().to_string(),
    )?;

    Ok(SetJobsCsvPathResult {
        path: resolve_jobs_csv_path(data_dir, conn)?.display().to_string(),
        action,
        import,
        env_override,
    })
}

/// Export current DB to the default CSV path, clear the override setting, return resolved path.
pub fn reset_jobs_csv_path(data_dir: &Path, conn: &Connection) -> AppResult<SetJobsCsvPathResult> {
    let default_path = default_jobs_csv_path(data_dir);
    export_jobs_csv(conn, &default_path, None)?;
    delete_setting(conn, JOBS_CSV_PATH_SETTING_KEY)?;
    let env_override = env_jobs_csv_path()?.is_some();
    let path = resolve_jobs_csv_path(data_dir, conn)?;
    Ok(SetJobsCsvPathResult {
        path: path.display().to_string(),
        action: "reset_to_default".to_string(),
        import: None,
        env_override,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate;
    use crate::jobs::csv::jobs_csv_sync_path;
    use rusqlite::Connection;
    use std::fs;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn open_test_db(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        migrate::migrate(&conn).unwrap();
        conn
    }

    fn seed_job(conn: &Connection) {
        let now = "2026-01-01T00:00:00Z";
        conn.execute(
            "INSERT INTO companies (id, name, careers_url, created_at, updated_at) VALUES (?1,?2,NULL,?3,?3)",
            rusqlite::params!["c1", "Acme", now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO jobs (id, company_id, title, url, canonical_url, status, source, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?4,'wishlist','manual',?5,?5)",
            rusqlite::params![
                "j1",
                "c1",
                "Engineer",
                "https://example.com/jobs/1",
                now
            ],
        )
        .unwrap();
    }

    #[test]
    fn resolve_uses_default_then_setting_then_env() {
        let _guard = env_lock();
        env::remove_var(JOBS_CSV_ENV_VAR);
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        let db_path = data_dir.join("job-tracker.db");
        let conn = open_test_db(&db_path);

        assert_eq!(
            resolve_jobs_csv_path(&data_dir, &conn).unwrap(),
            default_jobs_csv_path(&data_dir)
        );

        let custom = data_dir.join("custom").join("jobs.csv");
        set_setting(
            &conn,
            JOBS_CSV_PATH_SETTING_KEY,
            &custom.display().to_string(),
        )
        .unwrap();
        assert_eq!(resolve_jobs_csv_path(&data_dir, &conn).unwrap(), custom);

        let env_path = data_dir.join("from-env.csv");
        env::set_var(JOBS_CSV_ENV_VAR, env_path.display().to_string());
        assert_eq!(resolve_jobs_csv_path(&data_dir, &conn).unwrap(), env_path);
        env::remove_var(JOBS_CSV_ENV_VAR);
    }

    #[test]
    fn relative_path_rejected() {
        let err = normalize_absolute_jobs_csv_path("jobs.csv").unwrap_err();
        assert!(err.to_string().contains("absolute"));
    }

    #[test]
    fn tilde_expanded_at_set_time() {
        let _guard = env_lock();
        let home = tempdir().unwrap();
        env::set_var("HOME", home.path());
        let path = normalize_absolute_jobs_csv_path("~/Dropbox/jobs.csv").unwrap();
        assert_eq!(path, home.path().join("Dropbox/jobs.csv"));
        assert!(path.is_absolute());
    }

    #[test]
    fn empty_sidecar_refuses_silent_merge() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        let db_path = data_dir.join("job-tracker.db");
        let conn = open_test_db(&db_path);
        seed_job(&conn);

        let csv = data_dir.join("external.csv");
        fs::write(&csv, "id,csv_rev,url,canonical_url,title,company,status,applied_at,notes,location,latest_note,source,posting_state,updated_at\n").unwrap();
        assert!(!jobs_csv_sync_path(&csv).exists());

        let err = set_jobs_csv_path(
            &data_dir,
            &db_path,
            &conn,
            &csv.display().to_string(),
            SetJobsCsvMode::LinkWithoutSidecar,
            false,
            false,
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("Refusing silent merge"));
        assert!(get_setting(&conn, JOBS_CSV_PATH_SETTING_KEY)
            .unwrap()
            .is_none());
    }

    #[test]
    fn reset_exports_to_default_and_clears_setting() {
        let _guard = env_lock();
        env::remove_var(JOBS_CSV_ENV_VAR);
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        fs::create_dir_all(&data_dir).unwrap();
        let db_path = data_dir.join("job-tracker.db");
        let conn = open_test_db(&db_path);
        seed_job(&conn);

        let custom_dir = data_dir.join("dropbox");
        fs::create_dir_all(&custom_dir).unwrap();
        let custom = custom_dir.join("jobs.csv");
        set_jobs_csv_path(
            &data_dir,
            &db_path,
            &conn,
            &custom.display().to_string(),
            SetJobsCsvMode::RelocateExport,
            false,
            false,
            None,
        )
        .unwrap();
        assert!(custom.exists());
        assert!(get_setting(&conn, JOBS_CSV_PATH_SETTING_KEY)
            .unwrap()
            .is_some());

        let result = reset_jobs_csv_path(&data_dir, &conn).unwrap();
        let default = default_jobs_csv_path(&data_dir);
        assert_eq!(result.path, default.display().to_string());
        assert!(default.exists());
        assert!(jobs_csv_sync_path(&default).exists());
        assert!(get_setting(&conn, JOBS_CSV_PATH_SETTING_KEY)
            .unwrap()
            .is_none());
    }

    #[test]
    fn env_override_visible_in_info() {
        let _guard = env_lock();
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        let db_path = data_dir.join("job-tracker.db");
        let conn = open_test_db(&db_path);
        let env_path = data_dir.join("env-jobs.csv");
        env::set_var(JOBS_CSV_ENV_VAR, env_path.display().to_string());
        let info = jobs_csv_path_info(&data_dir, &conn).unwrap();
        assert!(info.env_override);
        assert_eq!(info.path, env_path.display().to_string());
        env::remove_var(JOBS_CSV_ENV_VAR);
    }
}
