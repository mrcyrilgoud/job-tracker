use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use keyring::Entry;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use url::Url;

use crate::error::{map_sqlite, AppError, AppResult};
use crate::util::now_iso;

const SERVICE: &str = "job-tracker-local";
const ACCOUNT: &str = "gmail-refresh-token";
const CLIENT_ID_KEY: &str = "gmail_client_id";
const CLIENT_SECRET_KEY: &str = "gmail_client_secret";
const REDIRECT_URI_KEY: &str = "gmail_redirect_uri";
const OAUTH_STATE_KEY: &str = "gmail_oauth_state";
const OAUTH_VERIFIER_KEY: &str = "gmail_oauth_verifier";
pub const CHECKPOINT_KEY: &str = "gmail_history_checkpoint";

static PENDING_REDIRECT: Mutex<Option<String>> = Mutex::new(None);

fn get_setting(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
    .map_err(map_sqlite)
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
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

fn delete_setting(conn: &Connection, key: &str) -> AppResult<()> {
    conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])
        .map_err(map_sqlite)?;
    Ok(())
}

pub fn get_gmail_config(conn: &Connection) -> AppResult<serde_json::Value> {
    let client_id = std::env::var("GMAIL_CLIENT_ID")
        .ok()
        .or(get_setting(conn, CLIENT_ID_KEY)?);
    let client_secret = std::env::var("GMAIL_CLIENT_SECRET")
        .ok()
        .or(get_setting(conn, CLIENT_SECRET_KEY)?);
    let redirect_uri = std::env::var("GMAIL_REDIRECT_URI").ok().or(get_setting(
        conn,
        REDIRECT_URI_KEY,
    )?);
    Ok(serde_json::json!({
        "clientId": client_id,
        "clientSecret": client_secret,
        "redirectUri": redirect_uri,
    }))
}

pub fn save_gmail_config(
    conn: &Connection,
    client_id: &str,
    client_secret: &str,
    redirect_uri: Option<&str>,
) -> AppResult<()> {
    set_setting(conn, CLIENT_ID_KEY, client_id)?;
    set_setting(conn, CLIENT_SECRET_KEY, client_secret)?;
    if let Some(uri) = redirect_uri {
        set_setting(conn, REDIRECT_URI_KEY, uri)?;
    }
    Ok(())
}

fn keyring_entry() -> AppResult<Entry> {
    Entry::new(SERVICE, ACCOUNT).map_err(|e| AppError::from(e.to_string()))
}

pub fn is_gmail_connected() -> AppResult<bool> {
    match keyring_entry()?.get_password() {
        Ok(token) => Ok(!token.is_empty()),
        Err(_) => Ok(false),
    }
}

pub fn disconnect_gmail() -> AppResult<()> {
    let _ = keyring_entry()?.delete_credential();
    Ok(())
}

fn base64_url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Start loopback OAuth: bind 127.0.0.1 ephemeral port, return auth URL + redirect URI.
pub fn begin_gmail_oauth(conn: &Connection) -> AppResult<serde_json::Value> {
    let config = get_gmail_config(conn)?;
    let client_id = config
        .get("clientId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::from("Gmail OAuth client is not configured"))?;
    let _secret = config
        .get("clientSecret")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::from("Gmail OAuth client is not configured"))?;

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| AppError::from(e.to_string()))?;
    listener
        .set_nonblocking(false)
        .map_err(|e| AppError::from(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::from(e.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    set_setting(conn, REDIRECT_URI_KEY, &redirect_uri)?;

    let state = base64_url(&rand_bytes(24));
    let verifier = base64_url(&rand_bytes(32));
    let challenge = base64_url(&Sha256::digest(verifier.as_bytes()));
    set_setting(conn, OAUTH_STATE_KEY, &state)?;
    set_setting(conn, OAUTH_VERIFIER_KEY, &verifier)?;

    let mut auth = Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
        .map_err(|e| AppError::from(e.to_string()))?;
    {
        let mut qp = auth.query_pairs_mut();
        qp.append_pair("client_id", client_id);
        qp.append_pair("redirect_uri", &redirect_uri);
        qp.append_pair("response_type", "code");
        qp.append_pair("scope", "https://www.googleapis.com/auth/gmail.readonly");
        qp.append_pair("access_type", "offline");
        qp.append_pair("prompt", "consent");
        qp.append_pair("state", &state);
        qp.append_pair("code_challenge", &challenge);
        qp.append_pair("code_challenge_method", "S256");
    }

    // Spawn blocking accept in background thread; store result for complete step.
    let expected_state = state.clone();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(180)));
            let mut buf = [0u8; 4096];
            if let Ok(n) = stream.read(&mut buf) {
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                if let Some(path) = first_line.split_whitespace().nth(1) {
                    let full = format!("http://127.0.0.1{path}");
                    if let Ok(url) = Url::parse(&full) {
                        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
                        if pairs.get("state").map(String::as_str) == Some(expected_state.as_str())
                        {
                            if let Some(code) = pairs.get("code") {
                                *PENDING_REDIRECT.lock().unwrap() = Some(code.clone());
                            }
                        }
                    }
                }
                let body = "<html><body><h1>Job Tracker</h1><p>You can close this window and return to the app.</p></body></html>";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        }
    });

    Ok(serde_json::json!({
        "url": auth.to_string(),
        "state": state,
        "redirectUri": redirect_uri
    }))
}

fn rand_bytes(n: usize) -> Vec<u8> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut out = vec![0u8; n];
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(1);
    let mut x = seed as u64;
    for b in &mut out {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = (x & 0xff) as u8;
    }
    // Mix with uuid randomness
    let id = uuid::Uuid::new_v4();
    for (i, byte) in id.as_bytes().iter().enumerate() {
        if i < out.len() {
            out[i] ^= byte;
        }
    }
    out
}

pub async fn complete_gmail_oauth_from_pending(
    db_path: &std::path::Path,
) -> AppResult<serde_json::Value> {
    let mut code = None;
    for _ in 0..120 {
        if let Some(c) = PENDING_REDIRECT.lock().unwrap().take() {
            code = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let code = code.ok_or_else(|| AppError::from("OAuth callback was not received"))?;
    let (expected_state, verifier, config) = {
        let conn = Connection::open(db_path).map_err(map_sqlite)?;
        (
            get_setting(&conn, OAUTH_STATE_KEY)?,
            get_setting(&conn, OAUTH_VERIFIER_KEY)?,
            get_gmail_config(&conn)?,
        )
    };
    let state = expected_state.ok_or_else(|| AppError::from("Invalid OAuth state"))?;
    complete_gmail_oauth(db_path, &code, &state, verifier, config).await
}

pub async fn complete_gmail_oauth(
    db_path: &std::path::Path,
    code: &str,
    state: &str,
    verifier: Option<String>,
    config: serde_json::Value,
) -> AppResult<serde_json::Value> {
    {
        let conn = Connection::open(db_path).map_err(map_sqlite)?;
        let expected_state = get_setting(&conn, OAUTH_STATE_KEY)?;
        if expected_state.as_deref() != Some(state) {
            return Err(AppError::from("Invalid OAuth state"));
        }
    }
    let verifier = verifier.ok_or_else(|| AppError::from("Missing PKCE verifier"))?;
    let client_id = config
        .get("clientId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::from("Gmail OAuth client is not configured"))?
        .to_string();
    let client_secret = config
        .get("clientSecret")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::from("Gmail OAuth client is not configured"))?
        .to_string();
    let redirect_uri = config
        .get("redirectUri")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::from("Missing redirect URI"))?
        .to_string();

    let client = reqwest::Client::new();
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", code),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|e| AppError::from(e.to_string()))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::from(e.to_string()))?;
    let refresh = body
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AppError::from("No refresh token returned. Re-authorize with prompt=consent.")
        })?;

    keyring_entry()?
        .set_password(refresh)
        .map_err(|e| AppError::from(e.to_string()))?;
    {
        let conn = Connection::open(db_path).map_err(map_sqlite)?;
        delete_setting(&conn, OAUTH_STATE_KEY)?;
        delete_setting(&conn, OAUTH_VERIFIER_KEY)?;
    }
    Ok(serde_json::json!({ "connected": true }))
}

pub async fn get_access_token_with_config(config: &serde_json::Value) -> AppResult<String> {
    let refresh = keyring_entry()?
        .get_password()
        .map_err(|_| AppError::from("Gmail is not connected"))?;
    let client_id = config
        .get("clientId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::from("Gmail OAuth client is not configured"))?
        .to_string();
    let client_secret = config
        .get("clientSecret")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::from("Gmail OAuth client is not configured"))?
        .to_string();

    let client = reqwest::Client::new();
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| AppError::from(e.to_string()))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::from(e.to_string()))?;
    body.get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::from("Failed to refresh Gmail access token"))
}

pub fn get_checkpoint(conn: &Connection) -> AppResult<Option<String>> {
    get_setting(conn, CHECKPOINT_KEY)
}

pub fn set_checkpoint(conn: &Connection, value: &str) -> AppResult<()> {
    set_setting(conn, CHECKPOINT_KEY, value)
}
