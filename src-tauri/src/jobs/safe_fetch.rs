use std::net::{IpAddr, Ipv6Addr};
use std::sync::OnceLock;
use std::time::Duration;

use dns_lookup::lookup_host;
use reqwest::header::HeaderMap;
use reqwest::redirect::Policy;
use reqwest::{Client, Method, StatusCode};
use url::Url;

const MAX_BYTES: usize = 1_500_000;
const TIMEOUT_MS: u64 = 10_000;
const MAX_REDIRECTS: usize = 5;
/// Upper bound for a single hostname resolution. DNS runs on the blocking pool so it
/// never stalls a Tokio worker, and this bound keeps the 30 s evaluation budget enforceable.
const DNS_TIMEOUT: Duration = Duration::from_secs(5);

static SHARED_CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();

pub(crate) fn shared_client() -> Result<Client, String> {
    SHARED_CLIENT
        .get_or_init(|| {
            Client::builder()
                .redirect(Policy::none())
                .timeout(Duration::from_millis(TIMEOUT_MS))
                .user_agent("JobTrackerLocal/1.0")
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map(Clone::clone)
        .map_err(Clone::clone)
}

/// Response headers that carry classification signals. Everything else is discarded so
/// cookies, auth headers, and other response metadata never leave the fetch layer.
pub const SIGNAL_HEADER_ALLOWLIST: &[&str] = &["cf-mitigated"];
/// Maximum retained byte length of an allowlisted header value.
const MAX_SIGNAL_HEADER_VALUE_BYTES: usize = 128;

/// Machine-readable reason a fetch did not produce a usable response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchErrorKind {
    /// The HTTP request or body read exceeded the request timeout.
    Timeout,
    /// Hostname resolution failed or returned no addresses.
    Dns,
    /// Hostname resolution did not finish within `DNS_TIMEOUT`.
    DnsTimeout,
    /// The connection (TCP/TLS) could not be established or was reset.
    Connect,
    /// The destination is private, loopback, link-local, or a local hostname.
    BlockedDestination,
    /// The URL could not be parsed or uses a scheme other than HTTP/HTTPS.
    InvalidUrl,
    /// A redirect response had no `Location` header.
    RedirectMissingLocation,
    /// A redirect `Location` could not be resolved to a URL.
    RedirectInvalid,
    /// More than `MAX_REDIRECTS` redirects were followed.
    TooManyRedirects,
    /// The response exceeded `MAX_BYTES`.
    TooLarge,
    /// Reading the response body failed.
    Body,
    /// The HTTP client could not be built or the request failed for another reason.
    Client,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SafeFetchResult {
    pub ok: bool,
    /// HTTP status of the last response received (0 when no response was received).
    pub status: u16,
    /// URL of the last request attempted (the final URL after redirects on success).
    pub final_url: String,
    pub body_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// URL exactly as passed by the caller.
    pub requested_url: String,
    /// Status codes of every redirect response followed, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub redirect_statuses: Vec<u16>,
    /// Set exactly when `error` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<FetchErrorKind>,
    /// Allowlisted signal headers (see `SIGNAL_HEADER_ALLOWLIST`) from the last response,
    /// as lowercase `(name, value)` pairs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub signal_headers: Vec<(String, String)>,
}

/// A host check failure with its machine-readable kind and the legacy message text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HostCheckError {
    kind: FetchErrorKind,
    message: String,
}

impl HostCheckError {
    fn new(kind: FetchErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
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

/// Runs a blocking resolver on the blocking pool, bounded by `timeout`, and rejects
/// empty or private results. Split out from `assert_public_hostname` so the timeout and
/// private-address paths are testable without network access.
async fn resolve_public_with<F>(resolve: F, timeout: Duration) -> Result<(), HostCheckError>
where
    F: FnOnce() -> std::io::Result<Vec<IpAddr>> + Send + 'static,
{
    let records = match tokio::time::timeout(timeout, tokio::task::spawn_blocking(resolve)).await {
        Err(_elapsed) => {
            return Err(HostCheckError::new(
                FetchErrorKind::DnsTimeout,
                "DNS lookup timed out",
            ))
        }
        Ok(Err(join_err)) => {
            return Err(HostCheckError::new(
                FetchErrorKind::Dns,
                format!("DNS lookup failed: {join_err}"),
            ))
        }
        Ok(Ok(Err(e))) => return Err(HostCheckError::new(FetchErrorKind::Dns, e.to_string())),
        Ok(Ok(Ok(records))) => records,
    };
    if records.is_empty() {
        return Err(HostCheckError::new(
            FetchErrorKind::Dns,
            "Hostname could not be resolved",
        ));
    }
    if records.into_iter().any(is_private_ip) {
        return Err(HostCheckError::new(
            FetchErrorKind::BlockedDestination,
            "Hostname resolves to a private address",
        ));
    }
    Ok(())
}

async fn assert_public_hostname(hostname: &str) -> Result<(), HostCheckError> {
    if hostname == "localhost" || hostname.ends_with(".local") {
        return Err(HostCheckError::new(
            FetchErrorKind::BlockedDestination,
            "Private or local hostnames are not allowed",
        ));
    }

    if let Ok(ip) = hostname.parse::<IpAddr>() {
        if is_private_ip(ip) {
            return Err(HostCheckError::new(
                FetchErrorKind::BlockedDestination,
                "Private IP addresses are not allowed",
            ));
        }
        return Ok(());
    }

    let owned = hostname.to_string();
    resolve_public_with(move || lookup_host(&owned), DNS_TIMEOUT).await
}

/// Keeps only allowlisted signal headers with UTF-8 values, lowercased names, and values
/// truncated on a character boundary to `MAX_SIGNAL_HEADER_VALUE_BYTES`.
pub fn filter_signal_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for name in SIGNAL_HEADER_ALLOWLIST {
        for value in headers.get_all(*name) {
            let Ok(text) = value.to_str() else { continue };
            let text = text.trim();
            let mut end = text.len().min(MAX_SIGNAL_HEADER_VALUE_BYTES);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            out.push(((*name).to_string(), text[..end].to_string()));
        }
    }
    out
}

/// Maps the properties of a failed `send()` to a kind. Timeout wins over connect because
/// reqwest can flag a connect timeout as both.
fn request_error_kind(is_timeout: bool, is_connect: bool, is_builder: bool) -> FetchErrorKind {
    if is_timeout {
        FetchErrorKind::Timeout
    } else if is_connect {
        FetchErrorKind::Connect
    } else if is_builder {
        FetchErrorKind::InvalidUrl
    } else {
        FetchErrorKind::Client
    }
}

/// Maps a failed body read to a kind.
fn body_error_kind(is_timeout: bool) -> FetchErrorKind {
    if is_timeout {
        FetchErrorKind::Timeout
    } else {
        FetchErrorKind::Body
    }
}

fn is_followed_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

/// Validates one hop before any request is sent: the URL must parse, use HTTP/HTTPS, and
/// point at a public host. Called for the requested URL and for every redirect target.
async fn check_hop_url(current: &str) -> Result<Url, HostCheckError> {
    let url = Url::parse(current)
        .map_err(|e| HostCheckError::new(FetchErrorKind::InvalidUrl, e.to_string()))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(HostCheckError::new(
            FetchErrorKind::InvalidUrl,
            "Only HTTP and HTTPS URLs are allowed",
        ));
    }
    let host = url.host_str().unwrap_or("");
    assert_public_hostname(host).await?;
    Ok(url)
}

/// Outcome of handling one followed redirect response.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RedirectStep {
    /// Follow the redirect to this absolute URL. The status was recorded.
    Follow(String),
    /// The `Location` header was missing or unusable. Nothing was recorded.
    Invalid(HostCheckError),
    /// The status was recorded but the redirect limit is now exceeded. Carries the URL the
    /// redirect pointed at, which becomes the reported `final_url`.
    TooMany(String),
}

/// Pure redirect step: resolves `location` against `current` (absolute or relative),
/// records `status` in the trace, and enforces `MAX_REDIRECTS`.
fn follow_redirect(
    trace: &mut FetchTrace,
    current: &Url,
    status: u16,
    location: Option<&str>,
) -> RedirectStep {
    let Some(location) = location else {
        return RedirectStep::Invalid(HostCheckError::new(
            FetchErrorKind::RedirectMissingLocation,
            "Redirect missing Location header",
        ));
    };
    let next = match Url::parse(location).or_else(|_| current.join(location)) {
        Ok(u) => u.to_string(),
        Err(e) => {
            return RedirectStep::Invalid(HostCheckError::new(
                FetchErrorKind::RedirectInvalid,
                e.to_string(),
            ))
        }
    };
    trace.redirect_statuses.push(status);
    if trace.redirect_statuses.len() > MAX_REDIRECTS {
        RedirectStep::TooMany(next)
    } else {
        RedirectStep::Follow(next)
    }
}

/// Per-call accumulator so every return path carries the same redirect evidence.
struct FetchTrace {
    requested_url: String,
    redirect_statuses: Vec<u16>,
}

impl FetchTrace {
    fn fail(
        &self,
        status: u16,
        final_url: String,
        kind: FetchErrorKind,
        message: impl Into<String>,
        signal_headers: Vec<(String, String)>,
    ) -> SafeFetchResult {
        SafeFetchResult {
            ok: false,
            status,
            final_url,
            body_text: String::new(),
            error: Some(message.into()),
            requested_url: self.requested_url.clone(),
            redirect_statuses: self.redirect_statuses.clone(),
            error_kind: Some(kind),
            signal_headers,
        }
    }
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
    let mut trace = FetchTrace {
        requested_url: raw_url.to_string(),
        redirect_statuses: Vec::new(),
    };

    let client = match shared_client() {
        Ok(client) => client,
        Err(error) => {
            return trace.fail(0, current, FetchErrorKind::Client, error, Vec::new());
        }
    };

    // `follow_redirect` enforces MAX_REDIRECTS, so this loop always terminates.
    loop {
        // Every hop (including redirect targets) is validated before any request is sent.
        let url = match check_hop_url(&current).await {
            Ok(u) => u,
            Err(e) => return trace.fail(0, current, e.kind, e.message, Vec::new()),
        };

        let response = match client
            .request(method.clone(), url.clone())
            .header("Accept", accept)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let kind = request_error_kind(e.is_timeout(), e.is_connect(), e.is_builder());
                let message = if e.is_timeout() {
                    "Request timed out".to_string()
                } else {
                    e.to_string()
                };
                return trace.fail(0, current, kind, message, Vec::new());
            }
        };

        let status = response.status();
        let signal_headers = filter_signal_headers(response.headers());
        if is_followed_redirect(status) {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok());
            match follow_redirect(&mut trace, &url, status.as_u16(), location) {
                RedirectStep::Follow(next) => {
                    current = next;
                    continue;
                }
                RedirectStep::Invalid(e) => {
                    return trace.fail(status.as_u16(), current, e.kind, e.message, signal_headers);
                }
                RedirectStep::TooMany(next) => {
                    return trace.fail(
                        0,
                        next,
                        FetchErrorKind::TooManyRedirects,
                        "Too many redirects",
                        Vec::new(),
                    );
                }
            }
        }

        if let Some(len) = response.content_length() {
            if len as usize > MAX_BYTES {
                return trace.fail(
                    status.as_u16(),
                    current,
                    FetchErrorKind::TooLarge,
                    "Response exceeds size limit",
                    signal_headers,
                );
            }
        }

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return trace.fail(
                    status.as_u16(),
                    current,
                    body_error_kind(e.is_timeout()),
                    e.to_string(),
                    signal_headers,
                );
            }
        };

        if bytes.len() > MAX_BYTES {
            return trace.fail(
                status.as_u16(),
                current,
                FetchErrorKind::TooLarge,
                "Response exceeds size limit",
                signal_headers,
            );
        }

        let body_text = String::from_utf8_lossy(&bytes).into_owned();
        return SafeFetchResult {
            ok: status.is_success(),
            status: status.as_u16(),
            final_url: current,
            body_text,
            error: None,
            requested_url: trace.requested_url,
            redirect_statuses: trace.redirect_statuses,
            error_kind: None,
            signal_headers,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;
    use std::net::Ipv4Addr;

    #[test]
    fn shared_client_configuration_is_initialized_once() {
        let first = shared_client().expect("shared HTTP client should build");
        let second = shared_client().expect("shared HTTP client should be reused");
        assert!(first.get("https://example.com").build().is_ok());
        assert!(second.get("https://example.com").build().is_ok());
        assert!(std::ptr::eq(
            SHARED_CLIENT.get().expect("client should be initialized"),
            SHARED_CLIENT
                .get()
                .expect("same client config should be reused"),
        ));
    }

    #[tokio::test]
    async fn blocks_loopback() {
        let result = safe_fetch("http://127.0.0.1/", Some("GET"), None).await;
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap_or("").contains("Private"));
        assert_eq!(result.error_kind, Some(FetchErrorKind::BlockedDestination));
        assert_eq!(result.requested_url, "http://127.0.0.1/");
        assert!(result.redirect_statuses.is_empty());
    }

    #[tokio::test]
    async fn blocks_local_hostname() {
        let result = safe_fetch("https://localhost/job", Some("GET"), None).await;
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap_or("").contains("local"));
        assert_eq!(result.error_kind, Some(FetchErrorKind::BlockedDestination));
    }

    #[tokio::test]
    async fn blocks_file_scheme() {
        let result = safe_fetch("file:///etc/passwd", Some("GET"), None).await;
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap_or("").contains("Only HTTP"));
        assert_eq!(result.error_kind, Some(FetchErrorKind::InvalidUrl));
    }

    #[tokio::test]
    async fn blocks_other_non_http_schemes() {
        let result = safe_fetch("ftp://example.com/job", Some("GET"), None).await;
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap_or("").contains("Only HTTP"));
    }

    #[tokio::test]
    async fn unparseable_url_is_invalid_url() {
        let result = safe_fetch("not a url", Some("GET"), None).await;
        assert!(!result.ok);
        assert_eq!(result.error_kind, Some(FetchErrorKind::InvalidUrl));
        assert_eq!(result.requested_url, "not a url");
    }

    #[tokio::test]
    async fn resolver_returning_private_address_is_blocked() {
        let err = resolve_public_with(
            || {
                Ok(vec![
                    IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                ])
            },
            DNS_TIMEOUT,
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind, FetchErrorKind::BlockedDestination);
        assert_eq!(err.message, "Hostname resolves to a private address");
    }

    #[tokio::test]
    async fn resolver_returning_public_address_is_allowed() {
        let result = resolve_public_with(
            || Ok(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]),
            DNS_TIMEOUT,
        )
        .await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn resolver_empty_and_error_map_to_dns() {
        let empty = resolve_public_with(|| Ok(vec![]), DNS_TIMEOUT)
            .await
            .unwrap_err();
        assert_eq!(empty.kind, FetchErrorKind::Dns);
        assert_eq!(empty.message, "Hostname could not be resolved");

        let failed = resolve_public_with(|| Err(std::io::Error::other("nxdomain")), DNS_TIMEOUT)
            .await
            .unwrap_err();
        assert_eq!(failed.kind, FetchErrorKind::Dns);
    }

    #[tokio::test]
    async fn slow_resolver_maps_to_dns_timeout() {
        let err = resolve_public_with(
            || {
                std::thread::sleep(Duration::from_millis(300));
                Ok(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))])
            },
            Duration::from_millis(20),
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind, FetchErrorKind::DnsTimeout);
    }

    #[tokio::test]
    async fn trailing_dot_localhost_is_not_allowed() {
        // "localhost." skips the literal name check, so this exercises the async DNS path.
        // Depending on the resolver it either resolves to loopback (blocked) or fails (dns).
        let err = assert_public_hostname("localhost.").await.unwrap_err();
        assert!(matches!(
            err.kind,
            FetchErrorKind::BlockedDestination | FetchErrorKind::Dns | FetchErrorKind::DnsTimeout
        ));
    }

    #[test]
    fn signal_header_filter_keeps_only_allowlisted_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("CF-Mitigated", HeaderValue::from_static("challenge"));
        headers.insert("set-cookie", HeaderValue::from_static("session=secret"));
        headers.insert("authorization", HeaderValue::from_static("Bearer secret"));
        headers.insert("server", HeaderValue::from_static("cloudflare"));
        assert_eq!(
            filter_signal_headers(&headers),
            vec![("cf-mitigated".to_string(), "challenge".to_string())]
        );
    }

    #[test]
    fn signal_header_filter_truncates_long_values() {
        let mut headers = HeaderMap::new();
        let long = "a".repeat(500);
        headers.insert("cf-mitigated", HeaderValue::from_str(&long).unwrap());
        let out = filter_signal_headers(&headers);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1.len(), MAX_SIGNAL_HEADER_VALUE_BYTES);
        assert!(filter_signal_headers(&HeaderMap::new()).is_empty());
    }

    #[test]
    fn request_error_kind_mapping() {
        assert_eq!(
            request_error_kind(true, true, false),
            FetchErrorKind::Timeout
        );
        assert_eq!(
            request_error_kind(true, false, false),
            FetchErrorKind::Timeout
        );
        assert_eq!(
            request_error_kind(false, true, false),
            FetchErrorKind::Connect
        );
        assert_eq!(
            request_error_kind(false, false, true),
            FetchErrorKind::InvalidUrl
        );
        assert_eq!(
            request_error_kind(false, false, false),
            FetchErrorKind::Client
        );
        assert_eq!(body_error_kind(true), FetchErrorKind::Timeout);
        assert_eq!(body_error_kind(false), FetchErrorKind::Body);
    }

    #[test]
    fn only_standard_redirect_statuses_are_followed() {
        for code in [301u16, 302, 303, 307, 308] {
            assert!(is_followed_redirect(StatusCode::from_u16(code).unwrap()));
        }
        for code in [200u16, 300, 304, 404] {
            assert!(!is_followed_redirect(StatusCode::from_u16(code).unwrap()));
        }
    }

    fn trace_for(url: &str) -> FetchTrace {
        FetchTrace {
            requested_url: url.to_string(),
            redirect_statuses: Vec::new(),
        }
    }

    /// Outcome of walking a scripted redirect chain the same way `safe_fetch` does.
    #[derive(Debug, PartialEq, Eq)]
    enum ChainEnd {
        /// Every hop was followed; this is the URL the final request would go to.
        Reached(String),
        Failed(FetchErrorKind),
    }

    /// Mirrors the `safe_fetch` loop without network I/O: validate the hop, then apply the
    /// next scripted `(status, Location)` redirect response. Uses IP literals so no DNS runs.
    async fn walk_chain(start: &str, hops: &[(u16, Option<&str>)]) -> (ChainEnd, Vec<u16>) {
        let mut trace = trace_for(start);
        let mut current = start.to_string();
        for (status, location) in hops {
            let url = match check_hop_url(&current).await {
                Ok(u) => u,
                Err(e) => return (ChainEnd::Failed(e.kind), trace.redirect_statuses),
            };
            match follow_redirect(&mut trace, &url, *status, *location) {
                RedirectStep::Follow(next) => current = next,
                RedirectStep::Invalid(e) => {
                    return (ChainEnd::Failed(e.kind), trace.redirect_statuses)
                }
                RedirectStep::TooMany(_) => {
                    return (
                        ChainEnd::Failed(FetchErrorKind::TooManyRedirects),
                        trace.redirect_statuses,
                    )
                }
            }
        }
        match check_hop_url(&current).await {
            Ok(_) => (ChainEnd::Reached(current), trace.redirect_statuses),
            Err(e) => (ChainEnd::Failed(e.kind), trace.redirect_statuses),
        }
    }

    #[tokio::test]
    async fn redirect_statuses_are_recorded_in_order_across_a_chain() {
        let (end, statuses) = walk_chain(
            "http://93.184.216.34/a",
            &[
                (301, Some("http://93.184.216.35/b")),
                (302, Some("/c")),
                (307, Some("https://93.184.216.36/d")),
                (308, Some("e")),
            ],
        )
        .await;
        assert_eq!(
            end,
            ChainEnd::Reached("https://93.184.216.36/e".to_string())
        );
        assert_eq!(statuses, vec![301, 302, 307, 308]);
    }

    #[test]
    fn relative_location_resolves_against_current_url() {
        let base = Url::parse("https://example.com/a/b?x=1").unwrap();
        let cases = [
            ("/jobs/1?gh_jid=9", "https://example.com/jobs/1?gh_jid=9"),
            ("c", "https://example.com/a/c"),
            ("../d", "https://example.com/d"),
            ("?error=true", "https://example.com/a/b?error=true"),
            ("//boards.example.org/z", "https://boards.example.org/z"),
            ("http://other.example/abs", "http://other.example/abs"),
        ];
        for (location, expected) in cases {
            let mut trace = trace_for(base.as_str());
            assert_eq!(
                follow_redirect(&mut trace, &base, 302, Some(location)),
                RedirectStep::Follow(expected.to_string()),
                "location {location:?}"
            );
            assert_eq!(trace.redirect_statuses, vec![302]);
        }
    }

    #[test]
    fn missing_location_is_redirect_missing_location_and_not_recorded() {
        let base = Url::parse("https://example.com/a").unwrap();
        let mut trace = trace_for(base.as_str());
        let RedirectStep::Invalid(err) = follow_redirect(&mut trace, &base, 301, None) else {
            panic!("expected Invalid");
        };
        assert_eq!(err.kind, FetchErrorKind::RedirectMissingLocation);
        assert_eq!(err.message, "Redirect missing Location header");
        assert!(trace.redirect_statuses.is_empty());
    }

    #[test]
    fn unresolvable_location_is_redirect_invalid_and_not_recorded() {
        let base = Url::parse("https://example.com/a").unwrap();
        let mut trace = trace_for(base.as_str());
        let RedirectStep::Invalid(err) =
            follow_redirect(&mut trace, &base, 302, Some("http://[::1"))
        else {
            panic!("expected Invalid");
        };
        assert_eq!(err.kind, FetchErrorKind::RedirectInvalid);
        assert!(trace.redirect_statuses.is_empty());
    }

    #[test]
    fn redirect_limit_allows_max_and_rejects_one_more() {
        let base = Url::parse("https://example.com/0").unwrap();
        let mut trace = trace_for(base.as_str());
        for i in 1..=MAX_REDIRECTS {
            let loc = format!("/{i}");
            assert_eq!(
                follow_redirect(&mut trace, &base, 302, Some(&loc)),
                RedirectStep::Follow(format!("https://example.com/{i}"))
            );
        }
        assert_eq!(
            follow_redirect(&mut trace, &base, 301, Some("/over")),
            RedirectStep::TooMany("https://example.com/over".to_string())
        );
        assert_eq!(trace.redirect_statuses.len(), MAX_REDIRECTS + 1);
        assert_eq!(trace.redirect_statuses.last(), Some(&301));
    }

    #[tokio::test]
    async fn chain_longer_than_limit_fails_with_too_many_redirects() {
        let hops: Vec<(u16, Option<&str>)> = vec![(302, Some("/next")); MAX_REDIRECTS + 1];
        let (end, statuses) = walk_chain("http://93.184.216.34/start", &hops).await;
        assert_eq!(end, ChainEnd::Failed(FetchErrorKind::TooManyRedirects));
        assert_eq!(statuses.len(), MAX_REDIRECTS + 1);
    }

    #[tokio::test]
    async fn redirect_to_blocked_destination_is_rejected_at_the_next_hop() {
        for target in [
            "http://10.0.0.5/admin",
            "http://127.0.0.1:8080/",
            "http://169.254.169.254/latest/meta-data/",
            "http://192.168.1.1/",
            "https://localhost/",
            "https://printer.local/",
        ] {
            let (end, statuses) =
                walk_chain("http://93.184.216.34/job", &[(302, Some(target))]).await;
            assert_eq!(
                end,
                ChainEnd::Failed(FetchErrorKind::BlockedDestination),
                "target {target}"
            );
            // The redirect that led there is still part of the evidence.
            assert_eq!(statuses, vec![302]);
        }
        let (end, _) = walk_chain(
            "http://93.184.216.34/job",
            &[(301, Some("file:///etc/passwd"))],
        )
        .await;
        assert_eq!(end, ChainEnd::Failed(FetchErrorKind::InvalidUrl));
    }

    #[tokio::test]
    async fn public_ip_literal_hop_passes_validation() {
        let url = check_hop_url("https://93.184.216.34/jobs/1").await.unwrap();
        assert_eq!(url.as_str(), "https://93.184.216.34/jobs/1");
    }

    #[test]
    fn private_ipv4_ranges_are_blocked() {
        let private = [
            "10.0.0.0",
            "10.255.255.255",
            "127.0.0.1",
            "127.255.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "169.254.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.0.1",
            "192.168.255.255",
        ];
        for ip in private {
            assert!(is_private_ip(ip.parse().unwrap()), "{ip} should be private");
        }
        let public = [
            "9.255.255.255",
            "11.0.0.1",
            "126.255.255.255",
            "169.253.0.1",
            "169.255.0.1",
            "172.15.255.255",
            "172.32.0.1",
            "192.167.0.1",
            "192.169.0.1",
            "93.184.216.34",
            "8.8.8.8",
        ];
        for ip in public {
            assert!(!is_private_ip(ip.parse().unwrap()), "{ip} should be public");
        }
    }

    #[test]
    fn private_and_loopback_ipv6_are_blocked() {
        for ip in [
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fdff:ffff::1",
            "fe80::1",
            "fe80::abcd:1",
        ] {
            assert!(is_private_ip(ip.parse().unwrap()), "{ip} should be private");
        }
        for ip in ["2606:4700:4700::1111", "2001:4860:4860::8888"] {
            assert!(!is_private_ip(ip.parse().unwrap()), "{ip} should be public");
        }
    }

    #[tokio::test]
    async fn ipv6_literal_hostnames_are_blocked() {
        for ip in ["::1", "fd00::1", "fe80::1"] {
            let err = assert_public_hostname(ip).await.unwrap_err();
            assert_eq!(err.kind, FetchErrorKind::BlockedDestination, "{ip}");
        }
    }

    #[tokio::test]
    async fn bracketed_ipv6_loopback_url_is_not_fetched() {
        let result = safe_fetch("http://[::1]:8080/", Some("GET"), None).await;
        assert!(!result.ok);
        assert!(result.body_text.is_empty());
        assert!(matches!(
            result.error_kind,
            Some(
                FetchErrorKind::BlockedDestination
                    | FetchErrorKind::Dns
                    | FetchErrorKind::DnsTimeout
            )
        ));
    }

    #[test]
    fn non_allowlisted_header_values_never_reach_serialized_result() {
        let mut headers = HeaderMap::new();
        headers.insert("cf-mitigated", HeaderValue::from_static("challenge"));
        headers.insert(
            "set-cookie",
            HeaderValue::from_static("sid=CANARY_COOKIE_7f3a"),
        );
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer CANARY_TOKEN_91bc"),
        );
        headers.insert(
            "x-amz-security-token",
            HeaderValue::from_static("CANARY_AMZ_55de"),
        );
        headers.insert(
            "location",
            HeaderValue::from_static("https://x.test/?CANARY_LOC_0a1b"),
        );

        let mut trace = trace_for("https://example.com/job");
        trace.redirect_statuses.push(302);
        let result = trace.fail(
            503,
            "https://example.com/job".to_string(),
            FetchErrorKind::Body,
            "body read failed",
            filter_signal_headers(&headers),
        );
        let json = serde_json::to_string(&result).unwrap();
        for canary in [
            "CANARY_COOKIE_7f3a",
            "CANARY_TOKEN_91bc",
            "CANARY_AMZ_55de",
            "CANARY_LOC_0a1b",
        ] {
            assert!(!json.contains(canary), "{canary} leaked into {json}");
        }
        for name in [
            "set-cookie",
            "authorization",
            "x-amz-security-token",
            "\"location\"",
        ] {
            assert!(!json.contains(name), "{name} leaked into {json}");
        }
        assert!(json.contains("\"signal_headers\":[[\"cf-mitigated\",\"challenge\"]]"));
        assert!(json.contains("\"redirect_statuses\":[302]"));
    }

    #[test]
    fn fetch_error_kind_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&FetchErrorKind::DnsTimeout).unwrap(),
            "\"dns_timeout\""
        );
        assert_eq!(
            serde_json::to_string(&FetchErrorKind::RedirectMissingLocation).unwrap(),
            "\"redirect_missing_location\""
        );
    }
}
