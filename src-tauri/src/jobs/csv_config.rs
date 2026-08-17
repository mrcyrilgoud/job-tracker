use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{map_sqlite, AppError, AppResult};
use crate::util::now_iso;

const JOBS_CSV_PATH_KEY: &str = "jobs_csv_path";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvConfig {
    pub path: String,
    pub default_path: String,
    pub is_custom: bool,
}

pub fn csv_lock_path(csv_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.lock", csv_path.display()))
}

pub fn get_csv_config(conn: &Connection, default_path: &Path) -> AppResult<CsvConfig> {
    let configured = get_setting(conn, JOBS_CSV_PATH_KEY)?;
    let path = configured
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_path.to_path_buf());
    Ok(CsvConfig {
        path: path.display().to_string(),
        default_path: default_path.display().to_string(),
        is_custom: configured.is_some(),
    })
}

pub fn active_csv_path(conn: &Connection, default_path: &Path) -> AppResult<PathBuf> {
    Ok(PathBuf::from(get_csv_config(conn, default_path)?.path))
}

pub fn set_custom_csv_path(
    conn: &Connection,
    default_path: &Path,
    path: &Path,
) -> AppResult<CsvConfig> {
    let path = validate_csv_path(path)?;
    set_setting(conn, JOBS_CSV_PATH_KEY, &path.display().to_string())?;
    get_csv_config(conn, default_path)
}

pub fn clear_custom_csv_path(conn: &Connection, default_path: &Path) -> AppResult<CsvConfig> {
    conn.execute(
        "DELETE FROM app_settings WHERE key = ?1",
        params![JOBS_CSV_PATH_KEY],
    )
    .map_err(map_sqlite)?;
    get_csv_config(conn, default_path)
}

pub fn validate_csv_path(path: &Path) -> AppResult<PathBuf> {
    if !path.is_absolute() {
        return Err(AppError::from("CSV location must be an absolute path"));
    }
    if path.file_name().is_none() {
        return Err(AppError::from("CSV location must include a filename"));
    }
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
    {
        return Err(AppError::from("CSV location must end with .csv"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| AppError::from("CSV location must include a parent folder"))?;
    if !parent.is_dir() {
        return Err(AppError::from("CSV parent folder does not exist"));
    }
    let parent = parent.canonicalize()?;
    let path = parent.join(path.file_name().expect("validated filename"));

    if path.exists() {
        OpenOptions::new().write(true).open(&path).map_err(|error| {
            AppError::from(format!("CSV location is not writable: {error}"))
        })?;
    } else {
        let probe = parent.join(format!(
            ".job-tracker-csv-write-check-{}",
            uuid::Uuid::new_v4()
        ));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .map_err(|error| AppError::from(format!("CSV folder is not writable: {error}")))?;
        let _ = std::fs::remove_file(probe);
    }
    Ok(path)
}

fn get_setting(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(map_sqlite)
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, now_iso()],
    )
    .map_err(map_sqlite)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;
    use tempfile::tempdir;

    #[test]
    fn resolves_default_custom_and_reset_paths() {
        let directory = tempdir().unwrap();
        let default = directory.path().join("jobs.csv");
        let custom = directory.path().join("custom.csv");
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        assert!(!get_csv_config(&conn, &default).unwrap().is_custom);
        set_custom_csv_path(&conn, &default, &custom).unwrap();
        assert_eq!(
            active_csv_path(&conn, &default).unwrap(),
            directory.path().canonicalize().unwrap().join("custom.csv")
        );
        assert!(get_csv_config(&conn, &default).unwrap().is_custom);
        clear_custom_csv_path(&conn, &default).unwrap();
        assert_eq!(active_csv_path(&conn, &default).unwrap(), default);
    }

    #[test]
    fn rejects_invalid_paths() {
        let directory = tempdir().unwrap();
        assert!(validate_csv_path(Path::new("jobs.csv")).is_err());
        assert!(validate_csv_path(&directory.path().join("jobs.txt")).is_err());
        assert!(validate_csv_path(Path::new("/missing/folder/jobs.csv")).is_err());
    }
}
