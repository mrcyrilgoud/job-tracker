use std::net::{IpAddr, Ipv6Addr};
use std::time::Duration;

use dns_lookup::lookup_host;
use reqwest::redirect::Policy;
use reqwest::{Client, Method, StatusCode};
use url::Url;

const MAX_BYTES: usize = 1_500_000;
const TIMEOUT_MS: u64 = 10_000;
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SafeFetchResult {
    pub ok: bool,
    pub status: u16,
    pub final_url: String,
    pub body_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            let a = octets[0];
            let b = octets[1];
            a == 10
                || a == 127
                || a == 0
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
        }
        IpAddr::V6(v6) => {
            if v6 == Ipv6Addr::LOCALHOST {
                return true;
            }
            let s = v6.to_string().to_lowercase();
            s.starts_with("fc") || s.starts_with("fd") || s.starts_with("fe80")
        }
    }
}

fn assert_public_hostname(hostname: &str) -> Result<(), String> {
    if hostname == "localhost" || hostname.ends_with(".local") {
        return Err("Private or local hostnames are not allowed".into());
    }

    if let Ok(ip) = hostname.parse::<IpAddr>() {
        if is_private_ip(ip) {
            return Err("Private IP addresses are not allowed".into());
        }
        return Ok(());
    }

    let records = lookup_host(hostname).map_err(|e| e.to_string())?;
    if records.is_empty() {
        return Err("Hostname could not be resolved".into());
    }
    for addr in records {
        if is_private_ip(addr) {
            return Err("Hostname resolves to a private address".into());
        }
    }
    Ok(())
}

pub async fn safe_fetch(
    raw_url: &str,
    method: Option<&str>,
    accept: Option<&str>,
) -> SafeFetchResult {
    let method = match method.unwrap_or("GET").to_uppercase().as_str() {
        "HEAD" => Method::HEAD,
        _ => Method::GET,
    };
    let accept = accept.unwrap_or("text/html,application/json,*/*");
    let mut current = raw_url.to_string();

    let client = match Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_millis(TIMEOUT_MS))
        .user_agent("JobTrackerLocal/1.0")
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return SafeFetchResult {
                ok: false,
                status: 0,
                final_url: current,
                body_text: String::new(),
                error: Some(e.to_string()),
            };
        }
    };

    for _ in 0..=MAX_REDIRECTS {
        let url = match Url::parse(&current) {
            Ok(u) => u,
            Err(e) => {
                return SafeFetchResult {
                    ok: false,
                    status: 0,
                    final_url: current,
                    body_text: String::new(),
                    error: Some(e.to_string()),
                };
            }
        };

        if url.scheme() != "http" && url.scheme() != "https" {
            return SafeFetchResult {
                ok: false,
                status: 0,
                final_url: current,
                body_text: String::new(),
                error: Some("Only HTTP and HTTPS URLs are allowed".into()),
            };
        }

        let host = url.host_str().unwrap_or("").to_string();
        if let Err(e) = assert_public_hostname(&host) {
            return SafeFetchResult {
                ok: false,
                status: 0,
                final_url: current,
                body_text: String::new(),
                error: Some(e),
            };
        }

        let response = match client
            .request(method.clone(), url.clone())
            .header("Accept", accept)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let message = if e.is_timeout() {
                    "Request timed out".to_string()
                } else {
                    e.to_string()
                };
                return SafeFetchResult {
                    ok: false,
                    status: 0,
                    final_url: current,
                    body_text: String::new(),
                    error: Some(message),
                };
            }
        };

        let status = response.status();
        if matches!(
            status,
            StatusCode::MOVED_PERMANENTLY
                | StatusCode::FOUND
                | StatusCode::SEE_OTHER
                | StatusCode::TEMPORARY_REDIRECT
                | StatusCode::PERMANENT_REDIRECT
        ) {
            let location = match response.headers().get("location").and_then(|v| v.to_str().ok()) {
                Some(l) => l.to_string(),
                None => {
                    return SafeFetchResult {
                        ok: false,
                        status: status.as_u16(),
                        final_url: current,
                        body_text: String::new(),
                        error: Some("Redirect missing Location header".into()),
                    };
                }
            };
            current = match Url::parse(&location)
                .or_else(|_| url.join(&location))
                .map(|u| u.to_string())
            {
                Ok(u) => u,
                Err(e) => {
                    return SafeFetchResult {
                        ok: false,
                        status: status.as_u16(),
                        final_url: current,
                        body_text: String::new(),
                        error: Some(e.to_string()),
                    };
                }
            };
            continue;
        }

        if let Some(len) = response.content_length() {
            if len as usize > MAX_BYTES {
                return SafeFetchResult {
                    ok: false,
                    status: status.as_u16(),
                    final_url: current,
                    body_text: String::new(),
                    error: Some("Response exceeds size limit".into()),
                };
            }
        }

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return SafeFetchResult {
                    ok: false,
                    status: status.as_u16(),
                    final_url: current,
                    body_text: String::new(),
                    error: Some(e.to_string()),
                };
            }
        };

        if bytes.len() > MAX_BYTES {
            return SafeFetchResult {
                ok: false,
                status: status.as_u16(),
                final_url: current,
                body_text: String::new(),
                error: Some("Response exceeds size limit".into()),
            };
        }

        let body_text = String::from_utf8_lossy(&bytes).into_owned();
        return SafeFetchResult {
            ok: status.is_success(),
            status: status.as_u16(),
            final_url: current,
            body_text,
            error: None,
        };
    }

    SafeFetchResult {
        ok: false,
        status: 0,
        final_url: current,
        body_text: String::new(),
        error: Some("Too many redirects".into()),
    }
}

pub fn looks_like_closed_posting(html: &str, status: u16) -> bool {
    if status == 404 || status == 410 {
        return true;
    }
    let lower = html.to_lowercase();
    let closed_signals = [
        "no longer accepting applications",
        "job is closed",
        "this job has expired",
        "position has been filled",
        "this posting is no longer available",
        "sorry, this job is no longer available",
    ];
    closed_signals.iter().any(|s| lower.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_on_404() {
        assert!(looks_like_closed_posting("", 404));
        assert!(looks_like_closed_posting("", 410));
    }

    #[test]
    fn closed_copy() {
        assert!(looks_like_closed_posting(
            "<html>This job is closed. Thanks.</html>",
            200
        ));
    }

    #[tokio::test]
    async fn blocks_loopback() {
        let result = safe_fetch("http://127.0.0.1/", Some("GET"), None).await;
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap_or("").contains("Private"));
    }

    #[tokio::test]
    async fn blocks_local_hostname() {
        let result = safe_fetch("https://localhost/job", Some("GET"), None).await;
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap_or("").contains("local"));
    }

    #[tokio::test]
    async fn blocks_file_scheme() {
        let result = safe_fetch("file:///etc/passwd", Some("GET"), None).await;
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap_or("").contains("Only HTTP"));
    }

    #[tokio::test]
    async fn blocks_other_non_http_schemes() {
        let result = safe_fetch("ftp://example.com/job", Some("GET"), None).await;
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap_or("").contains("Only HTTP"));
    }
}
