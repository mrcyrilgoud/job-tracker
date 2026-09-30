//! Structured, sanitized Check_Evidence (`evidence_version = 1`).
//!
//! `CheckEvidence` is the only record of *why* a posting was classified. It holds
//! the requested and final URL (sanitized), HTTP status, redirect statuses, the
//! provider signal, content signal categories, and a failure category. It never
//! holds response bodies, excerpts, headers, cookies, or URL credentials
//! (Req 7.4, 7.9).
//!
//! Wire format (persisted in `posting_check_evidence.evidence_json`):
//! - camelCase field names, snake_case enum values.
//! - Optional and empty fields are absent, never `null` or `[]`.
//! - `provider` is a flat object `{ provider, signal, postingId, httpStatus? }`,
//!   the same shape as [`EvidenceProviderView`]. `httpStatus` is only valid for
//!   `signal: "listing_unavailable"`.
//! - `content` is an array in [`ContentSignal`] declaration order (a `BTreeSet`).
//!
//! [`CheckEvidence::normalized`] is idempotent, and serialize → deserialize of a
//! normalized value yields an equal value (Req 6.15, 7.1).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::jobs::safe_fetch::FetchErrorKind;
use crate::runs::model::JobIdentity;
use crate::runs::progress::{
    bounded, ContractBounds, EvidenceProviderView, EvidenceView, MAX_ID_BYTES, MAX_URL_BYTES,
};

/// Version of the `CheckEvidence` JSON layout. Stored beside the JSON in
/// `posting_check_evidence.evidence_version` and carried in [`EvidenceView`].
pub const EVIDENCE_VERSION: u32 = 1;

/// Replacement value for secret-like query parameters and path parameters.
pub const REDACTED: &str = "redacted";

/// Stored in place of a URL that cannot be parsed as an absolute HTTP(S) URL.
/// Such a URL cannot be sanitized reliably, so none of it is kept.
pub const INVALID_URL: &str = "about:invalid";

/// Bound for `attempted_at` (RFC 3339 timestamps are ~32 bytes).
const MAX_TIMESTAMP_BYTES: usize = 64;

/// Upper bound on retained redirect statuses. `safe_fetch` follows at most 5.
const MAX_REDIRECT_STATUSES: usize = 10;

/// Case-insensitive substrings that mark a query or path-parameter key as
/// secret-like (design.md, "URL sanitization"). Matching is deliberately broad:
/// a false positive only redacts a value, a false negative would leak one.
const SECRET_KEY_MARKERS: &[&str] = &[
    "token", "session", "sid", "auth", "key", "sig", "secret", "password", "pwd", "code", "ticket",
    "jwt",
];

/// ATS provider with a public board listing API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Greenhouse,
    Lever,
    Ashby,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Self::Greenhouse, Self::Lever, Self::Ashby];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Greenhouse => "greenhouse",
            Self::Lever => "lever",
            Self::Ashby => "ashby",
        }
    }
}

/// What the provider listing said about the posting's stable id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderSignalKind {
    /// The listing was retrieved, parsed, and contains the posting id.
    ListedOpen,
    /// The listing was retrieved, parsed, and marks the posting closed.
    ListedClosed,
    /// The listing was retrieved and parsed, and the posting id is absent.
    AbsentFromListing,
    /// The listing could not be retrieved or parsed. Never conclusive.
    ListingUnavailable { http_status: Option<u16> },
}

impl ProviderSignalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ListedOpen => "listed_open",
            Self::ListedClosed => "listed_closed",
            Self::AbsentFromListing => "absent_from_listing",
            Self::ListingUnavailable { .. } => "listing_unavailable",
        }
    }

    /// HTTP status of a failed listing request, when one was received.
    pub fn http_status(self) -> Option<u16> {
        match self {
            Self::ListingUnavailable { http_status } => http_status,
            _ => None,
        }
    }
}

/// Provider evidence for one posting. Holds no request headers or credentials;
/// the public board APIs need none (Req 7.3).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "ProviderSignalWire", try_from = "ProviderSignalWire")]
pub struct ProviderSignal {
    pub provider: Provider,
    pub signal: ProviderSignalKind,
    pub posting_id: String,
}

/// Flat serde form of [`ProviderSignal`], matching [`EvidenceProviderView`].
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderSignalWire {
    provider: Provider,
    signal: ProviderSignalTag,
    posting_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    http_status: Option<u16>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProviderSignalTag {
    ListedOpen,
    ListedClosed,
    AbsentFromListing,
    ListingUnavailable,
}

impl From<ProviderSignal> for ProviderSignalWire {
    fn from(p: ProviderSignal) -> Self {
        let (signal, http_status) = match p.signal {
            ProviderSignalKind::ListedOpen => (ProviderSignalTag::ListedOpen, None),
            ProviderSignalKind::ListedClosed => (ProviderSignalTag::ListedClosed, None),
            ProviderSignalKind::AbsentFromListing => (ProviderSignalTag::AbsentFromListing, None),
            ProviderSignalKind::ListingUnavailable { http_status } => {
                (ProviderSignalTag::ListingUnavailable, http_status)
            }
        };
        Self {
            provider: p.provider,
            signal,
            posting_id: p.posting_id,
            http_status,
        }
    }
}

impl TryFrom<ProviderSignalWire> for ProviderSignal {
    type Error = String;

    fn try_from(w: ProviderSignalWire) -> Result<Self, Self::Error> {
        if w.http_status.is_some() && w.signal != ProviderSignalTag::ListingUnavailable {
            return Err("httpStatus is only valid for signal listing_unavailable".into());
        }
        let signal = match w.signal {
            ProviderSignalTag::ListedOpen => ProviderSignalKind::ListedOpen,
            ProviderSignalTag::ListedClosed => ProviderSignalKind::ListedClosed,
            ProviderSignalTag::AbsentFromListing => ProviderSignalKind::AbsentFromListing,
            ProviderSignalTag::ListingUnavailable => ProviderSignalKind::ListingUnavailable {
                http_status: w.http_status,
            },
        };
        Ok(Self {
            provider: w.provider,
            signal,
            posting_id: w.posting_id,
        })
    }
}

/// Content signal category extracted from a posting page. Only the category is
/// kept, never the matched text (Req 7.4). `Ord` follows declaration order, which
/// is the canonical order in the `BTreeSet` and on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentSignal {
    TitleMatch,
    CompanyMatch,
    ApplyEnabled,
    ApplyDisabled,
    ClosureCopyMatched,
    ClosureCopyUnmatched,
    ConsentPage,
    AuthPage,
    AntiBot,
    AccessDenied,
    GenericCareers,
}

impl ContentSignal {
    pub const ALL: [ContentSignal; 11] = [
        Self::TitleMatch,
        Self::CompanyMatch,
        Self::ApplyEnabled,
        Self::ApplyDisabled,
        Self::ClosureCopyMatched,
        Self::ClosureCopyUnmatched,
        Self::ConsentPage,
        Self::AuthPage,
        Self::AntiBot,
        Self::AccessDenied,
        Self::GenericCareers,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TitleMatch => "title_match",
            Self::CompanyMatch => "company_match",
            Self::ApplyEnabled => "apply_enabled",
            Self::ApplyDisabled => "apply_disabled",
            Self::ClosureCopyMatched => "closure_copy_matched",
            Self::ClosureCopyUnmatched => "closure_copy_unmatched",
            Self::ConsentPage => "consent_page",
            Self::AuthPage => "auth_page",
            Self::AntiBot => "anti_bot",
            Self::AccessDenied => "access_denied",
            Self::GenericCareers => "generic_careers",
        }
    }
}

/// Why a check could not produce a usable response or result (Req 7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    Timeout,
    DnsTimeout,
    Dns,
    Connect,
    BlockedDestination,
    InvalidUrl,
    RedirectFailure,
    TooManyRedirects,
    TooLarge,
    ProviderTemporary,
    ProviderFailure,
    Internal,
    Persistence,
}

impl FailureCategory {
    pub const ALL: [FailureCategory; 13] = [
        Self::Timeout,
        Self::DnsTimeout,
        Self::Dns,
        Self::Connect,
        Self::BlockedDestination,
        Self::InvalidUrl,
        Self::RedirectFailure,
        Self::TooManyRedirects,
        Self::TooLarge,
        Self::ProviderTemporary,
        Self::ProviderFailure,
        Self::Internal,
        Self::Persistence,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::DnsTimeout => "dns_timeout",
            Self::Dns => "dns",
            Self::Connect => "connect",
            Self::BlockedDestination => "blocked_destination",
            Self::InvalidUrl => "invalid_url",
            Self::RedirectFailure => "redirect_failure",
            Self::TooManyRedirects => "too_many_redirects",
            Self::TooLarge => "too_large",
            Self::ProviderTemporary => "provider_temporary",
            Self::ProviderFailure => "provider_failure",
            Self::Internal => "internal",
            Self::Persistence => "persistence",
        }
    }

    /// Transient failures force Unknown and name their category in the reason
    /// (Req 6.10). Exactly `timeout`, `dns_timeout`, `connect`, `provider_temporary`.
    pub fn is_transient(self) -> bool {
        matches!(
            self,
            Self::Timeout | Self::DnsTimeout | Self::Connect | Self::ProviderTemporary
        )
    }
}

/// Exhaustive on purpose: adding a `FetchErrorKind` variant must fail to compile
/// here until it is mapped.
impl From<FetchErrorKind> for FailureCategory {
    fn from(kind: FetchErrorKind) -> Self {
        match kind {
            FetchErrorKind::Timeout => Self::Timeout,
            FetchErrorKind::Dns => Self::Dns,
            FetchErrorKind::DnsTimeout => Self::DnsTimeout,
            FetchErrorKind::Connect => Self::Connect,
            FetchErrorKind::BlockedDestination => Self::BlockedDestination,
            FetchErrorKind::InvalidUrl => Self::InvalidUrl,
            FetchErrorKind::RedirectMissingLocation => Self::RedirectFailure,
            FetchErrorKind::RedirectInvalid => Self::RedirectFailure,
            FetchErrorKind::TooManyRedirects => Self::TooManyRedirects,
            FetchErrorKind::TooLarge => Self::TooLarge,
            // A body read that fails without timing out is a dropped connection.
            FetchErrorKind::Body => Self::Connect,
            // Client build failures and unclassified request errors are ours.
            FetchErrorKind::Client => Self::Internal,
        }
    }
}

/// HTTP 429 and 5xx are transient (Req 6.10, 6.11).
pub fn is_transient_http_status(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status)
}

/// Structured evidence for one posting check attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckEvidence {
    pub attempted_at: String,
    /// Sanitized.
    pub requested_url: String,
    /// Sanitized. Absent when equal to `requested_url` after normalization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    /// Status of the final response, when a response was received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// Status of every redirect followed, in order (Req 6.12, 7.2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirect_statuses: Vec<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderSignal>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub content: BTreeSet<ContentSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<FailureCategory>,
}

impl CheckEvidence {
    /// Empty evidence for an attempt at `requested_url` (sanitized here).
    pub fn new(requested_url: &str, attempted_at: impl Into<String>) -> Self {
        Self {
            attempted_at: bounded(attempted_at, MAX_TIMESTAMP_BYTES),
            requested_url: sanitize_url(requested_url),
            final_url: None,
            http_status: None,
            redirect_statuses: Vec::new(),
            provider: None,
            content: BTreeSet::new(),
            failure: None,
        }
    }

    /// Authoritative evidence for an evaluation that exceeded the 30 s bound
    /// (Req 8.2). Classifies as Unknown with category `timeout`.
    pub fn timeout(identity: &JobIdentity, attempted_at: impl Into<String>) -> Self {
        let mut ev = Self::new(&identity.posting_url, attempted_at);
        ev.failure = Some(FailureCategory::Timeout);
        ev
    }

    /// Canonical form: URLs sanitized and length-bounded, `final_url` dropped
    /// when it equals `requested_url`, out-of-range HTTP statuses dropped,
    /// redirect statuses limited to 3xx, strings bounded. Idempotent.
    pub fn normalized(&self) -> Self {
        let requested_url = sanitize_url(&self.requested_url);
        let final_url = self
            .final_url
            .as_deref()
            .map(sanitize_url)
            .filter(|f| *f != requested_url);
        let provider = self.provider.as_ref().map(|p| ProviderSignal {
            provider: p.provider,
            signal: match p.signal {
                ProviderSignalKind::ListingUnavailable { http_status } => {
                    ProviderSignalKind::ListingUnavailable {
                        http_status: http_status.filter(|s| is_valid_http_status(*s)),
                    }
                }
                other => other,
            },
            posting_id: bounded(p.posting_id.clone(), MAX_ID_BYTES),
        });
        Self {
            attempted_at: bounded(self.attempted_at.clone(), MAX_TIMESTAMP_BYTES),
            requested_url,
            final_url,
            http_status: self.http_status.filter(|s| is_valid_http_status(*s)),
            redirect_statuses: self
                .redirect_statuses
                .iter()
                .copied()
                .filter(|s| (300..=399).contains(s))
                .take(MAX_REDIRECT_STATUSES)
                .collect(),
            provider,
            content: self.content.clone(),
            failure: self.failure,
        }
    }

    /// True when the attempt ended with a transient failure category or a
    /// transient final HTTP status (429/5xx). A failed provider listing alone
    /// is not transient here: the page is still evaluated in that case.
    pub fn is_transient(&self) -> bool {
        self.failure.is_some_and(FailureCategory::is_transient)
            || self.http_status.is_some_and(is_transient_http_status)
    }

    /// Final URL of the attempt: `final_url` when a redirect changed it, else
    /// `requested_url`.
    pub fn effective_final_url(&self) -> &str {
        self.final_url.as_deref().unwrap_or(&self.requested_url)
    }

    /// Normalized JSON for `posting_check_evidence.evidence_json`.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(&self.normalized())
    }

    /// Parse stored evidence JSON, returning the normalized value.
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str::<Self>(json).map(|e| e.normalized())
    }
}

impl From<&CheckEvidence> for EvidenceView {
    /// Always converts from the normalized form, so the view is sanitized even
    /// when the source evidence was not.
    fn from(e: &CheckEvidence) -> Self {
        let e = e.normalized();
        let mut view = EvidenceView {
            evidence_version: EVIDENCE_VERSION,
            attempted_at: e.attempted_at,
            requested_url: e.requested_url,
            final_url: e.final_url,
            http_status: e.http_status,
            redirect_statuses: e.redirect_statuses,
            provider: e.provider.map(|p| EvidenceProviderView {
                provider: p.provider.as_str().to_string(),
                signal: p.signal.as_str().to_string(),
                posting_id: p.posting_id,
                http_status: p.signal.http_status(),
            }),
            content: e.content.iter().map(|c| c.as_str().to_string()).collect(),
            failure_category: e.failure.map(|f| f.as_str().to_string()),
        };
        view.enforce_bounds();
        view
    }
}

impl From<CheckEvidence> for EvidenceView {
    fn from(e: CheckEvidence) -> Self {
        EvidenceView::from(&e)
    }
}

fn is_valid_http_status(s: u16) -> bool {
    (100..=599).contains(&s)
}

fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    SECRET_KEY_MARKERS.iter().any(|m| k.contains(m))
}

/// Sanitize a URL for evidence, events, and persistence (Req 7.9):
/// - Only absolute `http`/`https` URLs with a host are kept; anything else
///   becomes [`INVALID_URL`], because it cannot be sanitized reliably.
/// - Userinfo and fragment are removed. The host is lowercased by `url`.
/// - Path parameters (`;k=v`) with a secret-like key are removed, which covers
///   `;jsessionid=…` in any case.
/// - Query values whose key is secret-like are replaced with [`REDACTED`].
///   Other parameters, such as `gh_jid`, are kept because they identify the
///   posting. The query is only re-encoded when something was redacted.
/// - Results longer than `MAX_URL_BYTES` lose the query, then the path.
///
/// The function is idempotent: `sanitize_url(&sanitize_url(u)) == sanitize_url(u)`.
pub fn sanitize_url(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw.trim()) else {
        return INVALID_URL.to_string();
    };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return INVALID_URL.to_string();
    }
    // Both setters only fail for URLs without a host, excluded above.
    let _ = url.set_password(None);
    let _ = url.set_username("");
    url.set_fragment(None);

    let path = url.path().to_string();
    let cleaned = strip_secret_path_params(&path);
    if cleaned != path {
        url.set_path(&cleaned);
    }

    if url.query_pairs().any(|(k, _)| is_secret_key(&k)) {
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| {
                let v = if is_secret_key(&k) {
                    REDACTED.to_string()
                } else {
                    v.into_owned()
                };
                (k.into_owned(), v)
            })
            .collect();
        url.query_pairs_mut().clear().extend_pairs(pairs);
    }

    clamp_url_len(url)
}

/// Remove `;key=value` path parameters whose (percent-decoded) key is secret-like.
fn strip_secret_path_params(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            let mut parts = segment.split(';');
            let head = parts.next().unwrap_or_default();
            let kept: Vec<&str> = parts
                .filter(|param| {
                    let key = param.split('=').next().unwrap_or_default();
                    let key = urlencoding::decode(key)
                        .map(|k| k.into_owned())
                        .unwrap_or_else(|_| key.to_string());
                    !is_secret_key(&key)
                })
                .collect();
            if kept.is_empty() {
                head.to_string()
            } else {
                format!("{head};{}", kept.join(";"))
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Bound a sanitized URL to `MAX_URL_BYTES` while keeping it a valid URL, so
/// the result is stable under re-sanitization (plain truncation is not).
fn clamp_url_len(mut url: Url) -> String {
    if url.as_str().len() <= MAX_URL_BYTES {
        return url.into();
    }
    url.set_query(None);
    if url.as_str().len() <= MAX_URL_BYTES {
        return url.into();
    }
    url.set_path("/");
    if url.as_str().len() <= MAX_URL_BYTES {
        return url.into();
    }
    INVALID_URL.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn identity(url: &str) -> JobIdentity {
        JobIdentity {
            job_id: "job-1".into(),
            title: "Engineer".into(),
            company_name: "Acme".into(),
            posting_url: url.into(),
        }
    }

    fn full_evidence() -> CheckEvidence {
        CheckEvidence {
            attempted_at: "2026-01-02T03:04:05Z".into(),
            requested_url: "https://boards.greenhouse.io/acme/jobs/127817?gh_jid=127817".into(),
            final_url: Some("https://boards.greenhouse.io/acme?error=true".into()),
            http_status: Some(200),
            redirect_statuses: vec![301, 302],
            provider: Some(ProviderSignal {
                provider: Provider::Greenhouse,
                signal: ProviderSignalKind::AbsentFromListing,
                posting_id: "127817".into(),
            }),
            content: [ContentSignal::GenericCareers, ContentSignal::CompanyMatch]
                .into_iter()
                .collect(),
            failure: None,
        }
    }

    // --- sanitize_url ---

    #[test]
    fn sanitize_strips_userinfo() {
        assert_eq!(
            sanitize_url("https://user:hunter2@Example.COM/jobs/1"),
            "https://example.com/jobs/1"
        );
        assert_eq!(
            sanitize_url("https://user@example.com/jobs/1"),
            "https://example.com/jobs/1"
        );
    }

    #[test]
    fn sanitize_strips_fragment() {
        assert_eq!(
            sanitize_url("https://example.com/jobs/1?x=1#access_token=abc"),
            "https://example.com/jobs/1?x=1"
        );
    }

    #[test]
    fn sanitize_strips_jsessionid_path_params() {
        assert_eq!(
            sanitize_url("https://example.com/job;jsessionid=ABC123?x=1"),
            "https://example.com/job?x=1"
        );
        assert_eq!(
            sanitize_url("https://example.com/a;JSESSIONID=ABC/b;view=full"),
            "https://example.com/a/b;view=full"
        );
    }

    #[test]
    fn sanitize_redacts_secret_query_values_and_keeps_others() {
        let out = sanitize_url(
            "https://example.com/job?gh_jid=123&token=abc&SessionId=zz9&api_key=k1&utm_source=li",
        );
        assert_eq!(
            out,
            "https://example.com/job?gh_jid=123&token=redacted&SessionId=redacted&api_key=redacted&utm_source=li"
        );
        for secret in ["abc", "zz9", "k1"] {
            assert!(!out.contains(&format!("={secret}")), "{out} leaks {secret}");
        }
    }

    #[test]
    fn sanitize_keeps_gh_jid_url_unchanged() {
        let url = "https://boards.greenhouse.io/acme/jobs/127817?gh_jid=127817";
        assert_eq!(sanitize_url(url), url);
        let url = "https://acme.com/careers?gh_jid=42";
        assert_eq!(sanitize_url(url), url);
    }

    #[test]
    fn sanitize_rejects_unparseable_and_non_http() {
        assert_eq!(sanitize_url("not a url"), INVALID_URL);
        assert_eq!(sanitize_url("/relative/path"), INVALID_URL);
        assert_eq!(sanitize_url("javascript:alert(1)"), INVALID_URL);
        assert_eq!(sanitize_url("ftp://user:pw@example.com/f"), INVALID_URL);
        assert_eq!(sanitize_url(INVALID_URL), INVALID_URL);
    }

    #[test]
    fn sanitize_bounds_length_and_stays_valid() {
        let long = format!("https://example.com/job?q={}", "a".repeat(3000));
        let out = sanitize_url(&long);
        assert_eq!(out, "https://example.com/job");
        let long_path = format!("https://example.com/{}", "p".repeat(3000));
        assert_eq!(sanitize_url(&long_path), "https://example.com/");
    }

    #[test]
    fn sanitize_is_idempotent() {
        let samples = [
            "https://user:pw@EXAMPLE.com:443/a;jsessionid=1/b?token=x&gh_jid=2&q=a b#frag",
            "https://example.com/job?a=%20x&auth=%2F",
            "http://example.com",
            "https://example.com/?",
            "https://example.com/caf%C3%A9;sid=9",
            "garbage",
        ];
        for s in samples {
            let once = sanitize_url(s);
            assert_eq!(sanitize_url(&once), once, "not idempotent for {s}");
        }
    }

    // --- failure mapping and transience ---

    #[test]
    fn fetch_error_kind_mapping_is_exhaustive() {
        use FailureCategory as F;
        use FetchErrorKind as K;
        let table = [
            (K::Timeout, F::Timeout),
            (K::Dns, F::Dns),
            (K::DnsTimeout, F::DnsTimeout),
            (K::Connect, F::Connect),
            (K::BlockedDestination, F::BlockedDestination),
            (K::InvalidUrl, F::InvalidUrl),
            (K::RedirectMissingLocation, F::RedirectFailure),
            (K::RedirectInvalid, F::RedirectFailure),
            (K::TooManyRedirects, F::TooManyRedirects),
            (K::TooLarge, F::TooLarge),
            (K::Body, F::Connect),
            (K::Client, F::Internal),
        ];
        // Compile-time guard that `table` lists every FetchErrorKind variant.
        for (kind, _) in &table {
            match kind {
                K::Timeout
                | K::Dns
                | K::DnsTimeout
                | K::Connect
                | K::BlockedDestination
                | K::InvalidUrl
                | K::RedirectMissingLocation
                | K::RedirectInvalid
                | K::TooManyRedirects
                | K::TooLarge
                | K::Body
                | K::Client => {}
            }
        }
        assert_eq!(table.len(), 12);
        for (kind, expected) in table {
            assert_eq!(FailureCategory::from(kind), expected, "{kind:?}");
        }
    }

    #[test]
    fn transient_categories_are_exact() {
        let transient: Vec<_> = FailureCategory::ALL
            .into_iter()
            .filter(|c| c.is_transient())
            .collect();
        assert_eq!(
            transient,
            vec![
                FailureCategory::Timeout,
                FailureCategory::DnsTimeout,
                FailureCategory::Connect,
                FailureCategory::ProviderTemporary,
            ]
        );
    }

    #[test]
    fn transient_http_statuses() {
        for s in [429, 500, 502, 503, 599] {
            assert!(is_transient_http_status(s), "{s}");
        }
        for s in [200, 301, 401, 403, 404, 410, 428, 600] {
            assert!(!is_transient_http_status(s), "{s}");
        }
    }

    #[test]
    fn evidence_is_transient_from_failure_or_status() {
        let mut ev = CheckEvidence::new("https://example.com/j", "t");
        assert!(!ev.is_transient());
        ev.http_status = Some(503);
        assert!(ev.is_transient());
        ev.http_status = Some(404);
        assert!(!ev.is_transient());
        ev.failure = Some(FailureCategory::Dns);
        assert!(!ev.is_transient());
        ev.failure = Some(FailureCategory::DnsTimeout);
        assert!(ev.is_transient());
        // A failed provider listing alone is not transient.
        let mut ev = CheckEvidence::new("https://example.com/j", "t");
        ev.provider = Some(ProviderSignal {
            provider: Provider::Lever,
            signal: ProviderSignalKind::ListingUnavailable {
                http_status: Some(503),
            },
            posting_id: "x".into(),
        });
        assert!(!ev.is_transient());
    }

    // --- constructors and normalization ---

    #[test]
    fn timeout_evidence_is_sanitized_and_marked() {
        let ev = CheckEvidence::timeout(&identity("https://u:p@example.com/j#f"), "t1");
        assert_eq!(ev.requested_url, "https://example.com/j");
        assert_eq!(ev.failure, Some(FailureCategory::Timeout));
        assert!(ev.http_status.is_none() && ev.final_url.is_none() && ev.content.is_empty());
        assert!(ev.is_transient());
        assert_eq!(ev.normalized(), ev);
    }

    #[test]
    fn normalized_is_idempotent_and_canonical() {
        let messy = CheckEvidence {
            attempted_at: "x".repeat(200),
            requested_url: "https://u:p@Example.com/j;jsessionid=1?token=t#f".into(),
            final_url: Some("https://example.com/j?token=other".into()),
            http_status: Some(0),
            redirect_statuses: vec![301, 0, 200, 302, 999],
            provider: Some(ProviderSignal {
                provider: Provider::Ashby,
                signal: ProviderSignalKind::ListingUnavailable {
                    http_status: Some(0),
                },
                posting_id: "p".repeat(100),
            }),
            content: [ContentSignal::AntiBot, ContentSignal::TitleMatch]
                .into_iter()
                .collect(),
            failure: Some(FailureCategory::Connect),
        };
        let once = messy.normalized();
        assert_eq!(once.normalized(), once);
        assert_eq!(once.requested_url, "https://example.com/j?token=redacted");
        // Final URL equals requested after sanitization, so it is dropped.
        assert_eq!(once.final_url, None);
        assert_eq!(once.http_status, None);
        assert_eq!(once.redirect_statuses, vec![301, 302]);
        assert!(once.attempted_at.len() <= MAX_TIMESTAMP_BYTES);
        let p = once.provider.as_ref().unwrap();
        assert!(p.posting_id.len() <= MAX_ID_BYTES);
        assert_eq!(
            p.signal,
            ProviderSignalKind::ListingUnavailable { http_status: None }
        );
        assert_eq!(
            once.content.iter().copied().collect::<Vec<_>>(),
            vec![ContentSignal::TitleMatch, ContentSignal::AntiBot]
        );

        let full = full_evidence().normalized();
        assert_eq!(full.normalized(), full);
        assert_eq!(
            full,
            full_evidence(),
            "already-canonical evidence is unchanged"
        );
    }

    // --- serde ---

    #[test]
    fn serde_shape_is_camel_case_flat_and_omits_empties() {
        let v = serde_json::to_value(full_evidence()).unwrap();
        assert_eq!(
            v,
            json!({
                "attemptedAt": "2026-01-02T03:04:05Z",
                "requestedUrl": "https://boards.greenhouse.io/acme/jobs/127817?gh_jid=127817",
                "finalUrl": "https://boards.greenhouse.io/acme?error=true",
                "httpStatus": 200,
                "redirectStatuses": [301, 302],
                "provider": {"provider": "greenhouse", "signal": "absent_from_listing", "postingId": "127817"},
                "content": ["company_match", "generic_careers"]
            })
        );

        let minimal = CheckEvidence::timeout(&identity("https://example.com/j"), "t");
        let v = serde_json::to_value(&minimal).unwrap();
        assert_eq!(
            v,
            json!({"attemptedAt": "t", "requestedUrl": "https://example.com/j", "failure": "timeout"})
        );
        assert!(!serde_json::to_string(&minimal).unwrap().contains("null"));
    }

    #[test]
    fn serde_round_trips() {
        let mut unavailable = full_evidence();
        unavailable.provider = Some(ProviderSignal {
            provider: Provider::Lever,
            signal: ProviderSignalKind::ListingUnavailable {
                http_status: Some(503),
            },
            posting_id: "abc".into(),
        });
        unavailable.failure = Some(FailureCategory::ProviderTemporary);
        for ev in [
            full_evidence(),
            unavailable,
            CheckEvidence::timeout(&identity("https://example.com/j"), "t"),
        ] {
            let ev = ev.normalized();
            let json = ev.to_json().unwrap();
            assert_eq!(serde_json::from_str::<CheckEvidence>(&json).unwrap(), ev);
            assert_eq!(CheckEvidence::from_json(&json).unwrap(), ev);
        }
        let v = serde_json::to_value(CheckEvidence {
            provider: Some(ProviderSignal {
                provider: Provider::Lever,
                signal: ProviderSignalKind::ListingUnavailable {
                    http_status: Some(503),
                },
                posting_id: "abc".into(),
            }),
            ..CheckEvidence::new("https://example.com/j", "t")
        })
        .unwrap();
        assert_eq!(
            v["provider"],
            json!({"provider": "lever", "signal": "listing_unavailable", "postingId": "abc", "httpStatus": 503})
        );
    }

    #[test]
    fn serde_rejects_http_status_on_conclusive_provider_signal() {
        let bad = json!({
            "attemptedAt": "t",
            "requestedUrl": "https://example.com/j",
            "provider": {"provider": "ashby", "signal": "listed_open", "postingId": "1", "httpStatus": 200}
        });
        assert!(serde_json::from_value::<CheckEvidence>(bad).is_err());
    }

    #[test]
    fn as_str_matches_serde_names() {
        for p in Provider::ALL {
            assert_eq!(serde_json::to_value(p).unwrap(), json!(p.as_str()));
        }
        for c in ContentSignal::ALL {
            assert_eq!(serde_json::to_value(c).unwrap(), json!(c.as_str()));
        }
        for f in FailureCategory::ALL {
            assert_eq!(serde_json::to_value(f).unwrap(), json!(f.as_str()));
        }
        for kind in [
            ProviderSignalKind::ListedOpen,
            ProviderSignalKind::ListedClosed,
            ProviderSignalKind::AbsentFromListing,
            ProviderSignalKind::ListingUnavailable { http_status: None },
        ] {
            let sig = ProviderSignal {
                provider: Provider::Lever,
                signal: kind,
                posting_id: "1".into(),
            };
            assert_eq!(
                serde_json::to_value(sig).unwrap()["signal"],
                json!(kind.as_str())
            );
        }
    }

    // --- EvidenceView ---

    #[test]
    fn evidence_view_conversion_carries_categories() {
        let view = EvidenceView::from(&full_evidence());
        assert_eq!(view.evidence_version, EVIDENCE_VERSION);
        assert_eq!(view.attempted_at, "2026-01-02T03:04:05Z");
        assert_eq!(
            view.requested_url,
            "https://boards.greenhouse.io/acme/jobs/127817?gh_jid=127817"
        );
        assert_eq!(
            view.final_url.as_deref(),
            Some("https://boards.greenhouse.io/acme?error=true")
        );
        assert_eq!(view.http_status, Some(200));
        assert_eq!(view.redirect_statuses, vec![301, 302]);
        assert_eq!(
            view.provider,
            Some(EvidenceProviderView {
                provider: "greenhouse".into(),
                signal: "absent_from_listing".into(),
                posting_id: "127817".into(),
                http_status: None,
            })
        );
        assert_eq!(view.content, vec!["company_match", "generic_careers"]);
        assert_eq!(view.failure_category, None);
        assert!(view.is_within_bounds());

        let timeout = EvidenceView::from(CheckEvidence::timeout(
            &identity("https://example.com/j"),
            "t",
        ));
        assert_eq!(timeout.failure_category.as_deref(), Some("timeout"));
        assert!(timeout.provider.is_none() && timeout.content.is_empty());
    }

    #[test]
    fn evidence_view_is_sanitized_even_from_raw_evidence() {
        let raw = CheckEvidence {
            requested_url: "https://u:secretpw@example.com/j;jsessionid=S3CR3T?token=T0K#frag"
                .into(),
            final_url: Some("https://example.com/careers?session=SESS".into()),
            ..CheckEvidence::new("https://example.com/j", "t")
        };
        let view = EvidenceView::from(&raw);
        let json = serde_json::to_string(&view).unwrap();
        for canary in ["secretpw", "S3CR3T", "T0K", "SESS", "frag"] {
            assert!(!json.contains(canary), "{json} leaks {canary}");
        }
        assert_eq!(view.requested_url, "https://example.com/j?token=redacted");
        assert_eq!(
            view.final_url.as_deref(),
            Some("https://example.com/careers?session=redacted")
        );
    }
}
