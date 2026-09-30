//! Provider target resolution, per-run [`ProviderListingCache`], and listing
//! lookup (Req 6.1, 6.4, 7.3).
//!
//! - [`resolve_provider_target`] is pure. The caller passes in the job's
//!   `source`, `source_external_id`, URL, and its company's watch rows.
//! - [`ProviderListingCache`] fetches each `(provider, board slug)` listing at
//!   most once per run, even under concurrent workers. The fetch itself is
//!   injected, so the `PostingFetcher` (task 5.9) owns all network I/O.
//! - A fetched listing is parsed once with `ats::parse_ats_jobs_from_json` into a
//!   [`ProviderListing`] (a set of normalized posting ids). The response body is
//!   dropped right after parsing and never leaves this module (Req 7.4).
//! - [`lookup_listing`] maps a listing and posting id to a
//!   [`ProviderSignalKind`]. Only a retrieved *and* parsed listing is conclusive;
//!   any fetch or parse failure is `ListingUnavailable`.
//!
//! The public board listing APIs are unauthenticated, so no header, token, or
//! key is ever part of provider evidence (Req 7.3).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::OnceCell;

use super::evidence::{FailureCategory, Provider, ProviderSignal, ProviderSignalKind};
use crate::ats::parse_ats_jobs_from_json;
use crate::jobs::board_discovery::discover_from_url;

/// Parse a stored provider name (`jobs.source`, `company_watches.provider`,
/// `DetectedBoard.provider`). Those columns hold the lowercase names
/// `greenhouse`, `lever`, and `ashby`; matching is trimmed and case-insensitive.
pub fn parse_provider(raw: &str) -> Option<Provider> {
    let raw = raw.trim();
    Provider::ALL
        .into_iter()
        .find(|p| p.as_str().eq_ignore_ascii_case(raw))
}

/// Canonical posting id for comparison: trimmed and ASCII-lowercased.
/// Greenhouse ids are numeric (case has no effect); Lever and Ashby ids are
/// UUIDs, which compare case-insensitively.
pub fn normalize_posting_id(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// Canonical board slug: trimmed and ASCII-lowercased, matching
/// `board_discovery`, which lowercases every slug it detects.
fn normalize_board_slug(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// One `company_watches` row of the job's company, as the resolver needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchBoard {
    /// `company_watches.provider`.
    pub provider: String,
    /// `company_watches.board_slug`.
    pub board_slug: String,
}

/// Everything [`resolve_provider_target`] reads, loaded by the caller.
#[derive(Debug, Clone, Copy)]
pub struct ProviderTargetInput<'a> {
    /// `jobs.source`.
    pub source: Option<&'a str>,
    /// `jobs.source_external_id`.
    pub source_external_id: Option<&'a str>,
    /// `jobs.url`.
    pub url: &'a str,
    /// Every watch row of the job's company.
    pub watches: &'a [WatchBoard],
}

/// A posting that maps to a provider board with a stable posting id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderTarget {
    pub provider: Provider,
    /// Normalized (trimmed, lowercase).
    pub board_slug: String,
    /// As stored or discovered, trimmed. Compared via [`normalize_posting_id`].
    pub posting_id: String,
}

impl ProviderTarget {
    /// Provider evidence for this target given the run's listing for its board.
    pub fn signal(&self, listing: &ProviderListing) -> ProviderSignal {
        ProviderSignal {
            provider: self.provider,
            signal: lookup_listing(listing, &self.posting_id),
            posting_id: self.posting_id.clone(),
        }
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// Resolve which provider board, if any, can give authoritative evidence for a
/// posting (design.md, "ProviderTarget resolution"). Priority:
///
/// 1. `source` is a supported provider and `source_external_id` is set: that
///    provider and posting id. The board slug comes from the job URL when URL
///    discovery detects the same provider, else from the company's watch rows
///    when exactly one row exists for that provider.
/// 2. Otherwise, URL discovery detects a supported board with a posting id:
///    that board and id (manual jobs pasted from ATS URLs).
/// 3. Otherwise there is no target.
///
/// Pure: no I/O. URL discovery never makes a network request.
pub fn resolve_provider_target(input: &ProviderTargetInput<'_>) -> Option<ProviderTarget> {
    let discovered = discover_from_url(input.url).ok().and_then(|d| d.board);
    let discovered_provider = discovered
        .as_ref()
        .and_then(|b| parse_provider(&b.provider));

    // 1. Watch-sourced job with a stable provider id.
    if let (Some(provider), Some(posting_id)) = (
        non_empty(input.source).and_then(parse_provider),
        non_empty(input.source_external_id),
    ) {
        let slug_from_url = discovered
            .as_ref()
            .filter(|_| discovered_provider == Some(provider))
            .map(|b| normalize_board_slug(&b.board_slug));
        let slug = slug_from_url.or_else(|| {
            let mut matching = input
                .watches
                .iter()
                .filter(|w| parse_provider(&w.provider) == Some(provider))
                .map(|w| normalize_board_slug(&w.board_slug))
                .filter(|s| !s.is_empty());
            match (matching.next(), matching.next()) {
                (Some(only), None) => Some(only),
                _ => None, // none, or ambiguous
            }
        });
        if let Some(board_slug) = slug.filter(|s| !s.is_empty()) {
            return Some(ProviderTarget {
                provider,
                board_slug,
                posting_id: posting_id.to_string(),
            });
        }
    }

    // 2. ATS URL with a posting id.
    let board = discovered?;
    let provider = discovered_provider?;
    let posting_id = non_empty(board.posting_id.as_deref())?.to_string();
    let board_slug = normalize_board_slug(&board.board_slug);
    if board_slug.is_empty() {
        return None;
    }
    Some(ProviderTarget {
        provider,
        board_slug,
        posting_id,
    })
}

/// Raw outcome of one listing request, produced by the injected fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingFetch {
    /// A 2xx response with its body. The body is parsed and dropped by
    /// [`ProviderListing::from_fetch`].
    Fetched { http_status: u16, body: String },
    /// The request failed or returned a non-2xx status.
    Failed {
        http_status: Option<u16>,
        failure: Option<FailureCategory>,
    },
}

/// A board listing as seen by one run, parsed once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderListing {
    /// Retrieved and parsed. Holds the normalized ids of every open posting.
    Parsed { posting_ids: HashSet<String> },
    /// Not retrieved, or retrieved with an unexpected shape. Never conclusive.
    Unavailable {
        http_status: Option<u16>,
        failure: Option<FailureCategory>,
    },
}

impl ProviderListing {
    /// Parse a fetch outcome with `ats::parse_ats_jobs_from_json`.
    ///
    /// Entries without an id are ignored. A non-empty listing in which *no*
    /// entry has an id is treated as an unexpected shape (`Unavailable`), so
    /// a provider schema change can never mark every posting absent. An empty
    /// listing is a successful parse: every posting is absent from it (Req 6.4).
    pub fn from_fetch(provider: Provider, fetch: ListingFetch) -> Self {
        match fetch {
            ListingFetch::Failed {
                http_status,
                failure,
            } => Self::Unavailable {
                http_status,
                failure,
            },
            ListingFetch::Fetched { http_status, body } => {
                let unexpected_shape = Self::Unavailable {
                    http_status: Some(http_status),
                    failure: Some(FailureCategory::ProviderFailure),
                };
                if !(200..=299).contains(&http_status) {
                    return unexpected_shape;
                }
                let Ok(jobs) = parse_ats_jobs_from_json(provider.as_str(), &body) else {
                    return unexpected_shape;
                };
                let posting_ids: HashSet<String> = jobs
                    .iter()
                    .map(|job| normalize_posting_id(&job.external_id))
                    .filter(|id| !id.is_empty())
                    .collect();
                if !jobs.is_empty() && posting_ids.is_empty() {
                    return unexpected_shape;
                }
                Self::Parsed { posting_ids }
            }
        }
    }
}

/// What the listing says about `posting_id` (Req 6.1, 6.4).
///
/// - Parsed and containing the id → `ListedOpen`.
/// - Parsed and not containing it → `AbsentFromListing`.
/// - Unavailable → `ListingUnavailable` with the HTTP status, if any.
///
/// The public listing APIs only publish open postings, so `ListedClosed` is
/// never produced here. An empty posting id cannot be looked up and is
/// `ListingUnavailable` rather than a false `AbsentFromListing`.
pub fn lookup_listing(listing: &ProviderListing, posting_id: &str) -> ProviderSignalKind {
    match listing {
        ProviderListing::Unavailable { http_status, .. } => {
            ProviderSignalKind::ListingUnavailable {
                http_status: *http_status,
            }
        }
        ProviderListing::Parsed { posting_ids } => {
            let id = normalize_posting_id(posting_id);
            if id.is_empty() {
                ProviderSignalKind::ListingUnavailable { http_status: None }
            } else if posting_ids.contains(&id) {
                ProviderSignalKind::ListedOpen
            } else {
                ProviderSignalKind::AbsentFromListing
            }
        }
    }
}

type CacheKey = (Provider, String);
type ListingCell = Arc<OnceCell<Arc<ProviderListing>>>;

/// Per-run memo of board listings: one fetch per `(provider, slug)` per run.
/// Cheap to clone; clones share the same entries.
#[derive(Debug, Clone, Default)]
pub struct ProviderListingCache {
    entries: Arc<Mutex<HashMap<CacheKey, ListingCell>>>,
}

impl ProviderListingCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the run's listing for `(provider, slug)`, calling `fetch` only if
    /// no caller has fetched it yet. Concurrent callers for the same board wait
    /// on the single in-flight fetch. If that fetch is canceled (its caller was
    /// aborted), the next caller fetches instead.
    pub async fn get_or_fetch<F, Fut>(
        &self,
        provider: Provider,
        board_slug: &str,
        fetch: F,
    ) -> Arc<ProviderListing>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ListingFetch>,
    {
        let cell = {
            // The lock is never held across an await, so poisoning can only come
            // from a panic inside HashMap ops; the map is still consistent then.
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            entries
                .entry((provider, normalize_board_slug(board_slug)))
                .or_default()
                .clone()
        };
        cell.get_or_init(|| async move {
            Arc::new(ProviderListing::from_fetch(provider, fetch().await))
        })
        .await
        .clone()
    }

    /// Number of boards with a cache entry (fetched or in flight).
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn watch(provider: &str, slug: &str) -> WatchBoard {
        WatchBoard {
            provider: provider.into(),
            board_slug: slug.into(),
        }
    }

    fn input<'a>(
        source: Option<&'a str>,
        ext: Option<&'a str>,
        url: &'a str,
        watches: &'a [WatchBoard],
    ) -> ProviderTargetInput<'a> {
        ProviderTargetInput {
            source,
            source_external_id: ext,
            url,
            watches,
        }
    }

    fn target(provider: Provider, slug: &str, id: &str) -> ProviderTarget {
        ProviderTarget {
            provider,
            board_slug: slug.into(),
            posting_id: id.into(),
        }
    }

    fn fetched(body: &str) -> ListingFetch {
        ListingFetch::Fetched {
            http_status: 200,
            body: body.into(),
        }
    }

    // --- resolve_provider_target ---

    #[test]
    fn source_with_external_id_takes_priority_for_every_provider() {
        let cases = [
            (
                "greenhouse",
                "127817",
                "https://job-boards.greenhouse.io/Acme/jobs/127817",
                target(Provider::Greenhouse, "acme", "127817"),
            ),
            (
                "lever",
                "abc-123",
                "https://jobs.lever.co/acme/abc-123",
                target(Provider::Lever, "acme", "abc-123"),
            ),
            (
                "ashby",
                "ash-1",
                "https://jobs.ashbyhq.com/acme/ash-1",
                target(Provider::Ashby, "acme", "ash-1"),
            ),
        ];
        for (source, ext, url, expected) in cases {
            assert_eq!(
                resolve_provider_target(&input(Some(source), Some(ext), url, &[])),
                Some(expected),
                "{source}"
            );
        }
    }

    #[test]
    fn source_external_id_wins_over_the_url_posting_id() {
        // The URL names a different posting id; the stored stable id is used.
        let got = resolve_provider_target(&input(
            Some("greenhouse"),
            Some("999"),
            "https://boards.greenhouse.io/acme/jobs/111",
            &[],
        ));
        assert_eq!(got, Some(target(Provider::Greenhouse, "acme", "999")));
    }

    #[test]
    fn source_slug_falls_back_to_a_single_matching_watch() {
        for (source, provider) in [
            ("greenhouse", Provider::Greenhouse),
            ("lever", Provider::Lever),
            ("ashby", Provider::Ashby),
        ] {
            // Non-ATS URL; one watch for this provider plus one for another.
            let other = if source == "lever" { "ashby" } else { "lever" };
            let watches = [watch(source, " AcmeBoard "), watch(other, "elsewhere")];
            let got = resolve_provider_target(&input(
                Some(source),
                Some("id-1"),
                "https://acme.com/careers/role",
                &watches,
            ));
            assert_eq!(got, Some(target(provider, "acmeboard", "id-1")), "{source}");
        }
    }

    #[test]
    fn url_slug_is_preferred_over_the_watch_slug() {
        let watches = [watch("lever", "old-slug")];
        let got = resolve_provider_target(&input(
            Some("lever"),
            Some("abc"),
            "https://jobs.lever.co/new-slug/abc",
            &watches,
        ));
        assert_eq!(got, Some(target(Provider::Lever, "new-slug", "abc")));
    }

    #[test]
    fn no_target_with_multiple_or_no_matching_watches() {
        let two = [watch("greenhouse", "a"), watch("greenhouse", "b")];
        let none: [WatchBoard; 0] = [];
        let other_provider = [watch("lever", "a")];
        for watches in [&two[..], &none[..], &other_provider[..]] {
            let got = resolve_provider_target(&input(
                Some("greenhouse"),
                Some("1"),
                "https://acme.com/careers/role",
                watches,
            ));
            assert_eq!(got, None, "{watches:?}");
        }
    }

    #[test]
    fn source_url_of_another_provider_does_not_supply_the_slug() {
        // source=greenhouse, but the URL is a Lever posting and there are no
        // watches. Step 1 has no slug, so step 2 uses the Lever URL itself.
        let got = resolve_provider_target(&input(
            Some("greenhouse"),
            Some("1"),
            "https://jobs.lever.co/acme/xyz",
            &[],
        ));
        assert_eq!(got, Some(target(Provider::Lever, "acme", "xyz")));
    }

    #[test]
    fn url_discovery_resolves_manual_jobs_for_each_provider() {
        let cases = [
            (
                "https://boards.greenhouse.io/acme/jobs/5013911008",
                target(Provider::Greenhouse, "acme", "5013911008"),
            ),
            (
                "https://jobs.lever.co/Acme/abc-123?source=linkedin",
                target(Provider::Lever, "acme", "abc-123"),
            ),
            (
                "https://jobs.ashbyhq.com/chaidiscovery/49557cff-8121-4a6d-bfa3-83f2fabe080f",
                target(
                    Provider::Ashby,
                    "chaidiscovery",
                    "49557cff-8121-4a6d-bfa3-83f2fabe080f",
                ),
            ),
        ];
        for (url, expected) in cases {
            for source in [None, Some("manual"), Some("")] {
                assert_eq!(
                    resolve_provider_target(&input(source, None, url, &[])),
                    Some(expected.clone()),
                    "{url} {source:?}"
                );
            }
        }
    }

    #[test]
    fn blank_external_id_is_treated_as_missing() {
        let watches = [watch("ashby", "acme")];
        let got = resolve_provider_target(&input(
            Some("ashby"),
            Some("  "),
            "https://acme.com/careers/role",
            &watches,
        ));
        assert_eq!(got, None);
    }

    #[test]
    fn non_ats_and_malformed_urls_have_no_target() {
        for url in [
            "https://acme.com/careers/role",
            "https://jobs.lever.co/acme", // no posting id
            "not a url",
            "",
        ] {
            assert_eq!(
                resolve_provider_target(&input(None, None, url, &[])),
                None,
                "{url}"
            );
        }
    }

    #[test]
    fn parse_provider_accepts_only_supported_names() {
        assert_eq!(parse_provider("greenhouse"), Some(Provider::Greenhouse));
        assert_eq!(parse_provider(" Lever "), Some(Provider::Lever));
        assert_eq!(parse_provider("ASHBY"), Some(Provider::Ashby));
        for raw in ["", "manual", "workday", "greenhouse.io"] {
            assert_eq!(parse_provider(raw), None, "{raw}");
        }
    }

    // --- ProviderListing / lookup_listing ---

    #[test]
    fn lookup_open_and_absent_for_each_provider() {
        let cases = [
            (
                Provider::Greenhouse,
                r#"{"jobs":[{"id":127817,"title":"Eng","absolute_url":"u"}]}"#,
                "127817",
            ),
            (
                Provider::Lever,
                r#"[{"id":"ABC-123","text":"Eng","hostedUrl":"u"}]"#,
                "abc-123",
            ),
            (
                Provider::Ashby,
                r#"{"jobs":[{"id":"ash-1","title":"PM","jobUrl":"u"}]}"#,
                " ash-1 ",
            ),
        ];
        for (provider, body, present) in cases {
            let listing = ProviderListing::from_fetch(provider, fetched(body));
            assert_eq!(
                lookup_listing(&listing, present),
                ProviderSignalKind::ListedOpen,
                "{provider:?}"
            );
            assert_eq!(
                lookup_listing(&listing, "not-there"),
                ProviderSignalKind::AbsentFromListing,
                "{provider:?}"
            );
        }
    }

    #[test]
    fn empty_listing_is_parsed_and_every_posting_is_absent() {
        for (provider, body) in [
            (Provider::Greenhouse, r#"{"jobs":[]}"#),
            (Provider::Lever, "[]"),
            (Provider::Ashby, r#"{"jobs":[]}"#),
        ] {
            let listing = ProviderListing::from_fetch(provider, fetched(body));
            assert_eq!(
                listing,
                ProviderListing::Parsed {
                    posting_ids: HashSet::new()
                }
            );
            assert_eq!(
                lookup_listing(&listing, "1"),
                ProviderSignalKind::AbsentFromListing
            );
        }
    }

    #[test]
    fn malformed_listing_is_unavailable_with_its_status() {
        for (provider, body) in [
            (Provider::Greenhouse, "{}"),
            (Provider::Greenhouse, "<html>oops</html>"),
            (Provider::Lever, r#"{"postings":[]}"#),
            (Provider::Ashby, r#"{"jobs":"nope"}"#),
            // Entries exist but none has an id: an unexpected shape.
            (Provider::Lever, r#"[{"text":"Eng"},{"text":"PM"}]"#),
        ] {
            let listing = ProviderListing::from_fetch(provider, fetched(body));
            assert_eq!(
                listing,
                ProviderListing::Unavailable {
                    http_status: Some(200),
                    failure: Some(FailureCategory::ProviderFailure),
                },
                "{provider:?} {body}"
            );
            assert_eq!(
                lookup_listing(&listing, "1"),
                ProviderSignalKind::ListingUnavailable {
                    http_status: Some(200)
                }
            );
        }
    }

    #[test]
    fn failed_fetch_is_unavailable_and_keeps_the_status() {
        let listing = ProviderListing::from_fetch(
            Provider::Greenhouse,
            ListingFetch::Failed {
                http_status: Some(503),
                failure: Some(FailureCategory::ProviderTemporary),
            },
        );
        assert_eq!(
            lookup_listing(&listing, "1"),
            ProviderSignalKind::ListingUnavailable {
                http_status: Some(503)
            }
        );

        let listing = ProviderListing::from_fetch(
            Provider::Lever,
            ListingFetch::Failed {
                http_status: None,
                failure: Some(FailureCategory::Timeout),
            },
        );
        assert_eq!(
            lookup_listing(&listing, "1"),
            ProviderSignalKind::ListingUnavailable { http_status: None }
        );

        // A non-2xx "Fetched" outcome is never parsed as a listing.
        let listing = ProviderListing::from_fetch(
            Provider::Ashby,
            ListingFetch::Fetched {
                http_status: 404,
                body: r#"{"jobs":[]}"#.into(),
            },
        );
        assert_eq!(
            lookup_listing(&listing, "1"),
            ProviderSignalKind::ListingUnavailable {
                http_status: Some(404)
            }
        );
    }

    #[test]
    fn empty_posting_id_is_never_absent() {
        let listing = ProviderListing::from_fetch(Provider::Lever, fetched("[]"));
        assert_eq!(
            lookup_listing(&listing, "  "),
            ProviderSignalKind::ListingUnavailable { http_status: None }
        );
    }

    #[test]
    fn target_signal_names_provider_kind_and_id() {
        let listing =
            ProviderListing::from_fetch(Provider::Greenhouse, fetched(r#"{"jobs":[{"id":7}]}"#));
        let sig = target(Provider::Greenhouse, "acme", "7").signal(&listing);
        assert_eq!(
            sig,
            ProviderSignal {
                provider: Provider::Greenhouse,
                signal: ProviderSignalKind::ListedOpen,
                posting_id: "7".into(),
            }
        );
    }

    // --- ProviderListingCache ---

    #[tokio::test(start_paused = true)]
    async fn cache_fetches_once_under_concurrent_callers() {
        let cache = ProviderListingCache::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for i in 0..16 {
            let cache = cache.clone();
            let calls = calls.clone();
            // Mixed slug casing still maps to one board.
            let slug = if i % 2 == 0 { "acme" } else { " ACME " };
            handles.push(tokio::spawn(async move {
                cache
                    .get_or_fetch(Provider::Greenhouse, slug, || async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        fetched(r#"{"jobs":[{"id":1}]}"#)
                    })
                    .await
            }));
        }
        let mut results = Vec::new();
        for h in handles {
            results.push(h.await.unwrap());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(results.iter().all(|r| Arc::ptr_eq(r, &results[0])));
        assert_eq!(
            lookup_listing(&results[0], "1"),
            ProviderSignalKind::ListedOpen
        );

        // A later call is served from the cache.
        let again = cache
            .get_or_fetch(Provider::Greenhouse, "acme", || async {
                panic!("must not refetch");
            })
            .await;
        assert!(Arc::ptr_eq(&again, &results[0]));
        assert_eq!(cache.len(), 1);
    }

    #[tokio::test]
    async fn cache_keys_by_provider_and_slug() {
        let cache = ProviderListingCache::new();
        let calls = Arc::new(AtomicUsize::new(0));
        for (provider, slug) in [
            (Provider::Greenhouse, "acme"),
            (Provider::Lever, "acme"),
            (Provider::Greenhouse, "other"),
            (Provider::Greenhouse, "acme"),
        ] {
            let calls = calls.clone();
            cache
                .get_or_fetch(provider, slug, || async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    ListingFetch::Failed {
                        http_status: Some(500),
                        failure: None,
                    }
                })
                .await;
        }
        // A failed fetch is also memoized: one attempt per board per run.
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(cache.len(), 3);
    }
}
