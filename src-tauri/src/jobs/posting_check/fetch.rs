//! Network boundary of the posting check (design.md, Component 7).
//!
//! [`PostingFetcher`] is the only way the evaluator touches the network, so the
//! coordinator and tests can inject a scripted fetcher. It is generic (return
//! position `impl Future`), not object-safe; callers take `F: PostingFetcher`.
//!
//! [`HttpPostingFetcher`] is the production implementation. It uses
//! `safe_fetch` for posting pages and for the public ATS listing endpoints
//! (`ats::listing_url`), so the private-address checks, redirect limit, size
//! limit, and honest `JobTrackerLocal/1.0` user agent all apply unchanged.
//!
//! The page body travels inside [`PageFetch`] only as far as
//! `signals::extract_signals`; `evaluate` drops it right after extraction and
//! it never reaches `CheckEvidence` (Req 7.4).

use std::fmt;
use std::future::Future;

use super::evidence::{is_transient_http_status, FailureCategory, Provider};
use super::provider::ListingFetch;
use crate::ats::{listing_url, LISTING_ACCEPT};
use crate::jobs::safe_fetch::{safe_fetch, FetchErrorKind, SafeFetchResult};

/// Raw outcome of one posting page request, after redirects.
#[derive(Clone, PartialEq, Eq)]
pub struct PageFetch {
    /// URL as requested (unsanitized; `evaluate` sanitizes via `CheckEvidence`).
    pub requested_url: String,
    /// URL of the last request attempted (the final URL after redirects).
    pub final_url: String,
    /// Status of the final response. `None` when no response was received.
    pub http_status: Option<u16>,
    /// Status of every redirect followed, in order (Req 6.12, 7.2).
    pub redirect_statuses: Vec<u16>,
    /// Allowlisted signal headers (`cf-mitigated`) of the final response.
    pub signal_headers: Vec<(String, String)>,
    /// Response body. Empty on failure. Consumed and dropped by `evaluate`.
    pub body: String,
    /// Set when the fetch did not produce a usable response.
    pub error_kind: Option<FetchErrorKind>,
}

/// Manual `Debug` so logs and test failures never print the page body.
impl fmt::Debug for PageFetch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageFetch")
            .field("requested_url", &self.requested_url)
            .field("final_url", &self.final_url)
            .field("http_status", &self.http_status)
            .field("redirect_statuses", &self.redirect_statuses)
            .field("signal_headers", &self.signal_headers)
            .field("body_len", &self.body.len())
            .field("error_kind", &self.error_kind)
            .finish()
    }
}

impl From<SafeFetchResult> for PageFetch {
    fn from(r: SafeFetchResult) -> Self {
        Self {
            requested_url: r.requested_url,
            final_url: r.final_url,
            // `safe_fetch` reports 0 when no response was received.
            http_status: (r.status != 0).then_some(r.status),
            redirect_statuses: r.redirect_statuses,
            signal_headers: r.signal_headers,
            body: r.body_text,
            error_kind: r.error_kind,
        }
    }
}

/// Map a listing `safe_fetch` result onto a [`ListingFetch`].
///
/// - 2xx without a fetch error → `Fetched` (the body is parsed and dropped by
///   `ProviderListing::from_fetch`).
/// - A fetch error → `Failed` with its mapped category.
/// - Any other status → `Failed`, `ProviderTemporary` for 429/5xx and
///   `ProviderFailure` otherwise.
///
/// The failure category stays inside the listing; `evaluate` records only a
/// `ListingUnavailable` provider signal, never an evidence-level failure, so a
/// failed listing cannot block positive page evidence.
pub fn listing_fetch_from_result(r: SafeFetchResult) -> ListingFetch {
    let http_status = (r.status != 0).then_some(r.status);
    if let Some(kind) = r.error_kind {
        return ListingFetch::Failed {
            http_status,
            failure: Some(FailureCategory::from(kind)),
        };
    }
    if r.ok {
        return ListingFetch::Fetched {
            http_status: r.status,
            body: r.body_text,
        };
    }
    let failure = if http_status.is_some_and(is_transient_http_status) {
        FailureCategory::ProviderTemporary
    } else {
        FailureCategory::ProviderFailure
    };
    ListingFetch::Failed {
        http_status,
        failure: Some(failure),
    }
}

/// Network I/O for posting checks. Implementations must be cheap to share
/// across workers (`Send + Sync`) and must never touch the database.
pub trait PostingFetcher: Send + Sync + 'static {
    /// Fetch a posting page, following redirects.
    fn fetch_page(&self, url: &str) -> impl Future<Output = PageFetch> + Send;

    /// Fetch the public board listing for `provider` and `board_slug`.
    fn fetch_listing(
        &self,
        provider: Provider,
        board_slug: &str,
    ) -> impl Future<Output = ListingFetch> + Send;
}

/// Production fetcher backed by `safe_fetch`.
#[derive(Debug, Clone, Copy, Default)]
pub struct HttpPostingFetcher;

impl PostingFetcher for HttpPostingFetcher {
    async fn fetch_page(&self, url: &str) -> PageFetch {
        // Same method and default Accept header as the legacy check.
        PageFetch::from(safe_fetch(url, Some("GET"), None).await)
    }

    async fn fetch_listing(&self, provider: Provider, board_slug: &str) -> ListingFetch {
        let Some(url) = listing_url(provider.as_str(), board_slug) else {
            // Unreachable for the three supported providers.
            return ListingFetch::Failed {
                http_status: None,
                failure: Some(FailureCategory::Internal),
            };
        };
        listing_fetch_from_result(safe_fetch(&url, Some("GET"), Some(LISTING_ACCEPT)).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(ok: bool, status: u16, body: &str, kind: Option<FetchErrorKind>) -> SafeFetchResult {
        SafeFetchResult {
            ok,
            status,
            final_url: "https://example.com/final".into(),
            body_text: body.into(),
            error: kind.map(|k| format!("{k:?}")),
            requested_url: "https://example.com/start".into(),
            redirect_statuses: vec![301],
            error_kind: kind,
            signal_headers: vec![("cf-mitigated".into(), "challenge".into())],
        }
    }

    #[test]
    fn page_fetch_maps_status_zero_to_none_and_keeps_redirects() {
        let p = PageFetch::from(result(false, 0, "", Some(FetchErrorKind::Timeout)));
        assert_eq!(p.http_status, None);
        assert_eq!(p.error_kind, Some(FetchErrorKind::Timeout));
        assert_eq!(p.redirect_statuses, vec![301]);
        assert_eq!(p.requested_url, "https://example.com/start");
        assert_eq!(p.final_url, "https://example.com/final");

        let p = PageFetch::from(result(true, 200, "<html></html>", None));
        assert_eq!(p.http_status, Some(200));
        assert_eq!(p.body, "<html></html>");
        assert_eq!(p.signal_headers.len(), 1);
    }

    #[test]
    fn page_fetch_debug_omits_the_body() {
        let p = PageFetch::from(result(true, 200, "canary-body-secret", None));
        let dbg = format!("{p:?}");
        assert!(!dbg.contains("canary-body-secret"), "{dbg}");
        assert!(dbg.contains("body_len: 18"), "{dbg}");
    }

    #[test]
    fn listing_fetch_mapping() {
        assert_eq!(
            listing_fetch_from_result(result(true, 200, "[]", None)),
            ListingFetch::Fetched {
                http_status: 200,
                body: "[]".into()
            }
        );
        assert_eq!(
            listing_fetch_from_result(result(false, 503, "", None)),
            ListingFetch::Failed {
                http_status: Some(503),
                failure: Some(FailureCategory::ProviderTemporary),
            }
        );
        assert_eq!(
            listing_fetch_from_result(result(false, 429, "", None)),
            ListingFetch::Failed {
                http_status: Some(429),
                failure: Some(FailureCategory::ProviderTemporary),
            }
        );
        assert_eq!(
            listing_fetch_from_result(result(false, 404, "", None)),
            ListingFetch::Failed {
                http_status: Some(404),
                failure: Some(FailureCategory::ProviderFailure),
            }
        );
        assert_eq!(
            listing_fetch_from_result(result(false, 0, "", Some(FetchErrorKind::DnsTimeout))),
            ListingFetch::Failed {
                http_status: None,
                failure: Some(FailureCategory::DnsTimeout)
            }
        );
        // A size-limit failure after a 200 keeps the status but is not parsed.
        assert_eq!(
            listing_fetch_from_result(result(false, 200, "", Some(FetchErrorKind::TooLarge))),
            ListingFetch::Failed {
                http_status: Some(200),
                failure: Some(FailureCategory::TooLarge)
            }
        );
    }
}
