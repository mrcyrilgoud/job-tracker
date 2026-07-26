use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::safe_fetch::safe_fetch;
use crate::util::{create_id, now_iso};

fn normalize_careers_text(html: &str) -> String {
    let no_script = regex::Regex::new(r"(?is)<script[\s\S]*?</script>")
        .unwrap()
        .replace_all(html, " ");
    let no_style = regex::Regex::new(r"(?is)<style[\s\S]*?</style>")
        .unwrap()
        .replace_all(&no_script, " ");
    let no_tags = regex::Regex::new(r"(?is)<[^>]+>")
        .unwrap()
        .replace_all(&no_style, " ");
    no_tags.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub async fn fetch_careers_hash(careers_url: &str) -> AppResult<(String, String)> {
    let result = safe_fetch(careers_url, Some("GET"), None).await;
    if !result.ok {
        return Err(AppError::from(
            result
                .error
                .unwrap_or_else(|| format!("HTTP {}", result.status)),
        ));
    }
    let normalized = normalize_careers_text(&result.body_text);
    let hash = hex::encode(Sha256::digest(normalized.as_bytes()));
    Ok((hash, normalized))
}

pub fn apply_careers_check(
    conn: &Connection,
    company_id: &str,
    company_name: &str,
    content_hash: &str,
    normalized_text: &str,
) -> AppResult<serde_json::Value> {
    let previous: Option<(String, String)> = conn
        .query_row(
            "SELECT content_hash, captured_at FROM careers_page_snapshots WHERE company_id = ?1 ORDER BY captured_at DESC LIMIT 1",
            params![company_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(map_sqlite)?;

    let captured_at = now_iso();
    let text_slice: String = normalized_text.chars().take(20_000).collect();
    conn.execute(
        "INSERT INTO careers_page_snapshots (id, company_id, content_hash, normalized_text, captured_at) VALUES (?1,?2,?3,?4,?5)",
        params![create_id(), company_id, content_hash, text_slice, captured_at],
    )
    .map_err(map_sqlite)?;

    let Some((prev_hash, _)) = previous else {
        return Ok(serde_json::json!({
            "changed": false,
            "reason": "Initial snapshot captured"
        }));
    };

    if prev_hash == content_hash {
        return Ok(serde_json::json!({
            "changed": false,
            "reason": "No change detected"
        }));
    }

    conn.execute(
        "INSERT INTO careers_page_reviews (id, company_id, previous_hash, current_hash, summary, status, created_at) VALUES (?1,?2,?3,?4,?5,'pending',?6)",
        params![
            create_id(),
            company_id,
            prev_hash,
            content_hash,
            format!("Careers page content changed for {company_name}. Review manually; no jobs were auto-created."),
            captured_at
        ],
    )
    .map_err(map_sqlite)?;

    Ok(serde_json::json!({
        "changed": true,
        "currentHash": content_hash
    }))
}

pub fn company_careers_url(
    conn: &Connection,
    company_id: &str,
) -> AppResult<Option<(String, String)>> {
    conn.query_row(
        "SELECT name, careers_url FROM companies WHERE id = ?1",
        params![company_id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
    )
    .optional()
    .map_err(map_sqlite)
    .map(|row| {
        row.and_then(|(name, url)| url.map(|u| (name, u)))
    })
}
