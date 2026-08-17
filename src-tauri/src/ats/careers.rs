use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::safe_fetch::safe_fetch;
use crate::util::{create_id, now_iso};

const CAREERS_HASH_VERSION: &str = "v2:";

fn normalize_careers_text(html: &str) -> String {
    let no_script = regex::Regex::new(r"(?is)<script[\s\S]*?</script>")
        .unwrap()
        .replace_all(html, " ");
    let no_style = regex::Regex::new(r"(?is)<style[\s\S]*?</style>")
        .unwrap()
        .replace_all(&no_script, " ");
    // A main region is the strongest signal that the page is describing the
    // employer's careers content. Many marketing sites do not use one, so
    // fall back to the document after removing common site chrome.
    let main = regex::Regex::new(r"(?is)<main\b[^>]*>(.*?)</main>")
        .unwrap()
        .captures(&no_style)
        .and_then(|captures| captures.get(1).map(|content| content.as_str()))
        .unwrap_or(&no_style);
    let without_chrome = regex::Regex::new(
        r"(?is)<nav\b[^>]*>.*?</nav>|<header\b[^>]*>.*?</header>|<footer\b[^>]*>.*?</footer>|<aside\b[^>]*>.*?</aside>",
    )
    .unwrap()
    .replace_all(main, " ");
    let no_tags = regex::Regex::new(r"(?is)<[^>]+>")
        .unwrap()
        .replace_all(&without_chrome, " ");
    no_tags
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
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
    let hash = format!(
        "{CAREERS_HASH_VERSION}{}",
        hex::encode(Sha256::digest(normalized.as_bytes()))
    );
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

    // The prior algorithm included site navigation and footer copy. Treat its
    // first v2 replacement as a baseline, not a user-facing content change.
    if !prev_hash.starts_with(CAREERS_HASH_VERSION) {
        return Ok(serde_json::json!({
            "changed": false,
            "reason": "Careers content baseline refreshed"
        }));
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;

    #[test]
    fn ignores_navigation_changes_without_a_main_region() {
        let before = "<nav>Careers Exercises Contact</nav><div><h1>Work here</h1><p>Open roles</p></div><footer>© 2026</footer>";
        let after = "<nav>Careers Contact</nav><div><h1>Work here</h1><p>Open roles</p></div><footer>© 2027</footer>";

        assert_eq!(
            normalize_careers_text(before),
            normalize_careers_text(after)
        );
        assert_eq!(normalize_careers_text(after), "work here open roles");
    }

    #[test]
    fn prefers_main_content_over_surrounding_page_text() {
        let html = "<header>Changing promo</header><main><h1>Careers</h1><p>Build important things</p></main><footer>Changing legal copy</footer>";

        assert_eq!(
            normalize_careers_text(html),
            "careers build important things"
        );
    }

    #[test]
    fn first_v2_snapshot_replaces_a_legacy_baseline_without_an_alert() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO companies (id, name, careers_url, created_at, updated_at) VALUES ('company', 'Acme', NULL, 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO careers_page_snapshots (id, company_id, content_hash, normalized_text, captured_at) VALUES ('old', 'company', 'legacy-hash', 'old text', 'now')",
            [],
        )
        .unwrap();

        let result =
            apply_careers_check(&conn, "company", "Acme", "v2:new-hash", "new text").unwrap();

        assert_eq!(result["changed"], false);
        assert_eq!(result["reason"], "Careers content baseline refreshed");
        let reviews: i64 = conn
            .query_row("SELECT COUNT(*) FROM careers_page_reviews", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(reviews, 0);
    }
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
    .map(|row| row.and_then(|(name, url)| url.map(|u| (name, u))))
}
