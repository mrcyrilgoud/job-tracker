use std::fs::OpenOptions;
use std::path::Path;

use fs2::FileExt;
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::gmail::classify::classify_email;
use crate::gmail::oauth::{get_access_token_with_config, get_checkpoint, set_checkpoint};
use crate::models::EmailMatch;
use crate::util::{create_id, now_iso};

/// Poll Gmail using a connection that is only used between awaits (not held across them).
pub async fn poll_gmail_matches(db_path: &Path, lock_path: &Path) -> AppResult<serde_json::Value> {
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|e| AppError::from(e.to_string()))?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| AppError::from("operation_in_progress:gmail-poll"))?;
    let (access_token, checkpoint, tracked) = {
        let conn = open_conn(db_path)?;
        let config = crate::gmail::oauth::get_gmail_config(&conn)?;
        let access_token = get_access_token_with_config(&config).await?;
        let checkpoint = get_checkpoint(&conn)?;
        let mut tracked = Vec::new();
        let mut stmt = conn
            .prepare(
                "SELECT j.id, j.title, c.name FROM jobs j INNER JOIN companies c ON j.company_id = c.id",
            )
            .map_err(map_sqlite)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(map_sqlite)?;
        for row in rows {
            tracked.push(row.map_err(map_sqlite)?);
        }
        (access_token, checkpoint, tracked)
    };

    let after_query = if let Some(cp) = &checkpoint {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(cp) {
            format!("after:{}", dt.timestamp())
        } else {
            "newer_than:30d".to_string()
        }
    } else {
        "newer_than:30d".to_string()
    };

    let client = reqwest::Client::new();
    let list_url = format!(
        "https://gmail.googleapis.com/gmail/v1/users/me/messages?q={}&maxResults=50",
        urlencoding::encode(&after_query)
    );
    let list_resp = client
        .get(&list_url)
        .bearer_auth(&access_token)
        .send()
        .await
        .map_err(|e| AppError::from(e.to_string()))?;
    let list_body: serde_json::Value = list_resp
        .json()
        .await
        .map_err(|e| AppError::from(e.to_string()))?;

    let messages = list_body
        .get("messages")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();

    let mut linked = 0usize;
    let mut triaged = 0usize;
    let mut newest = checkpoint.unwrap_or_default();
    let mut pending_writes: Vec<PendingWrite> = Vec::new();

    for item in messages {
        let Some(id) = item
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
        else {
            continue;
        };

        {
            let conn = open_conn(db_path)?;
            let existing: Option<String> = conn
                .query_row(
                    "SELECT id FROM email_matches WHERE gmail_message_id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(map_sqlite)?;
            if existing.is_some() {
                continue;
            }
        }

        let msg_url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=metadata&metadataHeaders=From&metadataHeaders=Subject&metadataHeaders=Date",
            urlencoding::encode(&id)
        );
        let msg_resp = client
            .get(&msg_url)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(|e| AppError::from(e.to_string()))?;
        let message: serde_json::Value = msg_resp
            .json()
            .await
            .map_err(|e| AppError::from(e.to_string()))?;

        let headers = message
            .pointer("/payload/headers")
            .and_then(|h| h.as_array())
            .cloned()
            .unwrap_or_default();
        let header = |name: &str| -> String {
            headers
                .iter()
                .find(|h| h.get("name").and_then(|n| n.as_str()) == Some(name))
                .and_then(|h| h.get("value").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_string()
        };
        let subject = header("Subject");
        let from_address = header("From");
        let date_header = header("Date");
        let received_at = if date_header.is_empty() {
            now_iso()
        } else if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(&date_header) {
            dt.with_timezone(&chrono::Utc).to_rfc3339()
        } else {
            now_iso()
        };
        let snippet = message
            .get("snippet")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let thread_id = message
            .get("threadId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if newest.is_empty() || received_at > newest {
            newest = received_at.clone();
        }

        let mut best: Option<(String, &'static str)> = None;
        for (job_id, title, company) in &tracked {
            let Some(confidence) =
                classify_email(&subject, &snippet, &from_address, company, title)
            else {
                continue;
            };
            let better = match (&best, confidence) {
                (None, _) => true,
                (Some((_, "high")), _) => false,
                (Some((_, "medium")), "high") => true,
                (Some((_, "low")), "high" | "medium") => true,
                _ => false,
            };
            if better {
                best = Some((job_id.clone(), confidence));
            }
        }

        let Some((job_id, confidence)) = best else {
            continue;
        };

        pending_writes.push(PendingWrite {
            job_id,
            gmail_message_id: id,
            thread_id,
            subject,
            snippet,
            from_address,
            received_at,
            confidence,
            auto_link: confidence == "high",
        });
    }

    {
        let conn = open_conn(db_path)?;
        conn.execute_batch("BEGIN IMMEDIATE").map_err(map_sqlite)?;
        for write in pending_writes {
            let created_at = now_iso();
            if write.auto_link {
                let inserted = conn.execute(
                    "INSERT OR IGNORE INTO email_matches (id, job_id, gmail_message_id, thread_id, subject, snippet, from_address, received_at, confidence, triage_status, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'auto_linked',?10)",
                    params![
                        create_id(),
                        write.job_id,
                        write.gmail_message_id,
                        write.thread_id,
                        write.subject,
                        write.snippet,
                        write.from_address,
                        write.received_at,
                        write.confidence,
                        created_at
                    ],
                )
                .map_err(map_sqlite)?;
                if inserted == 0 {
                    continue;
                }
                conn.execute(
                    "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1,?2,'email_update',?3,?4)",
                    params![
                        create_id(),
                        write.job_id,
                        format!("Gmail: {}", write.subject),
                        write.received_at
                    ],
                )
                .map_err(map_sqlite)?;
                linked += 1;
            } else {
                let inserted = conn.execute(
                    "INSERT OR IGNORE INTO email_matches (id, job_id, gmail_message_id, thread_id, subject, snippet, from_address, received_at, confidence, triage_status, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending',?10)",
                    params![
                        create_id(),
                        write.job_id,
                        write.gmail_message_id,
                        write.thread_id,
                        write.subject,
                        write.snippet,
                        write.from_address,
                        write.received_at,
                        write.confidence,
                        created_at
                    ],
                )
                .map_err(map_sqlite)?;
                triaged += inserted;
            }
        }
        if !newest.is_empty() {
            set_checkpoint(&conn, &newest)?;
        }
        conn.execute_batch("COMMIT").map_err(map_sqlite)?;
    }

    Ok(serde_json::json!({
        "linked": linked,
        "triaged": triaged,
        "checkpoint": if newest.is_empty() { serde_json::Value::Null } else { newest.into() }
    }))
}

struct PendingWrite {
    job_id: String,
    gmail_message_id: String,
    thread_id: Option<String>,
    subject: String,
    snippet: String,
    from_address: String,
    received_at: String,
    confidence: &'static str,
    auto_link: bool,
}

fn open_conn(db_path: &std::path::Path) -> AppResult<Connection> {
    let conn = Connection::open(db_path).map_err(map_sqlite)?;
    conn.pragma_update(None, "busy_timeout", 5000i32)
        .map_err(map_sqlite)?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(map_sqlite)?;
    Ok(conn)
}

pub fn confirm_email_match(
    conn: &Connection,
    match_id: &str,
    job_id: Option<&str>,
) -> AppResult<serde_json::Value> {
    let match_row: Option<(Option<String>, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT job_id, subject, received_at FROM email_matches WHERE id = ?1",
            params![match_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(map_sqlite)?;
    let Some((_old_job, subject, received_at)) = match_row else {
        return Err(AppError::from("Email match not found"));
    };

    if job_id.is_none() {
        conn.execute(
            "UPDATE email_matches SET triage_status='dismissed', job_id=NULL WHERE id=?1",
            params![match_id],
        )
        .map_err(map_sqlite)?;
        return Ok(serde_json::json!({ "dismissed": true }));
    }

    let job_id = job_id.unwrap();
    conn.execute(
        "UPDATE email_matches SET triage_status='confirmed', job_id=?1 WHERE id=?2",
        params![job_id, match_id],
    )
    .map_err(map_sqlite)?;
    conn.execute(
        "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1,?2,'email_update',?3,?4)",
        params![
            create_id(),
            job_id,
            format!(
                "Gmail (confirmed): {}",
                subject.unwrap_or_else(|| "Update".into())
            ),
            received_at.unwrap_or_else(now_iso)
        ],
    )
    .map_err(map_sqlite)?;
    Ok(serde_json::json!({ "confirmed": true }))
}

pub fn list_pending_email_matches(conn: &Connection) -> AppResult<Vec<EmailMatch>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, job_id, gmail_message_id, thread_id, subject, snippet, from_address, received_at, confidence, triage_status, created_at FROM email_matches WHERE triage_status='pending'",
        )
        .map_err(map_sqlite)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(EmailMatch {
                id: row.get(0)?,
                job_id: row.get(1)?,
                gmail_message_id: row.get(2)?,
                thread_id: row.get(3)?,
                subject: row.get(4)?,
                snippet: row.get(5)?,
                from_address: row.get(6)?,
                received_at: row.get(7)?,
                confidence: row.get(8)?,
                triage_status: row.get(9)?,
                created_at: row.get(10)?,
            })
        })
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;
    Ok(rows)
}
