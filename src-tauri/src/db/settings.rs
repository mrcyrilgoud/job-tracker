use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{map_sqlite, AppResult};
use crate::util::now_iso;

pub fn get_setting(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
    .map_err(map_sqlite)
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    let updated_at = now_iso();
    let existing = get_setting(conn, key)?;
    if existing.is_some() {
        conn.execute(
            "UPDATE app_settings SET value = ?1, updated_at = ?2 WHERE key = ?3",
            params![value, updated_at, key],
        )
        .map_err(map_sqlite)?;
    } else {
        conn.execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1,?2,?3)",
            params![key, value, updated_at],
        )
        .map_err(map_sqlite)?;
    }
    Ok(())
}

pub fn delete_setting(conn: &Connection, key: &str) -> AppResult<()> {
    conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])
        .map_err(map_sqlite)?;
    Ok(())
}
