//! Evidence-based posting availability check.
//!
//! Pipeline: resolve a provider target → fetch the provider listing and/or the
//! posting page → extract structured, sanitized [`evidence::CheckEvidence`] →
//! classify it with the pure `classify` function → persist the result.
//!
//! Only signal *categories* ever leave the fetch layer. Response bodies,
//! excerpts, cookies, and URL credentials are never stored or published
//! (Req 7.4, 7.9).

// Scaffolding: most items are consumed by later tasks (classifier, evaluator,
// coordinator). Remove once the pipeline is wired into the run coordinator.
#![allow(dead_code)]

pub mod classify;
pub mod evidence;
pub mod fetch;
pub mod persist;
pub mod provider;
pub mod signals;

#[cfg(test)]
mod fixture_tests;

use url::Url;

use crate::runs::model::JobIdentity;
use evidence::{CheckEvidence, ContentSignal, FailureCategory, ProviderSignalKind};
use fetch::{PageFetch, PostingFetcher};
use provider::{
    resolve_provider_target, ProviderListingCache, ProviderTarget, ProviderTargetInput, WatchBoard,
};

/// Everything `evaluate` needs for one posting.
///
/// The design passes only a [`JobIdentity`] to `evaluate`. Provider target
/// resolution (design.md, "ProviderTarget resolution") also needs the job's
/// `source`, `source_external_id`, and its company's watch rows, which are not
/// part of the frozen Job_Identity. The caller (the coordinator, which owns the
/// DB connection) loads them into this struct so workers stay DB-free.
/// `evaluate` resolves the target from these fields and `identity.posting_url`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostingCheckInput {
    pub identity: JobIdentity,
    /// `jobs.source`.
    pub source: Option<String>,
    /// `jobs.source_external_id`.
    pub source_external_id: Option<String>,
    /// Every `company_watches` row of the job's company.
    pub watches: Vec<WatchBoard>,
}

impl PostingCheckInput {
    /// Input with no stored provider data. A target can still be resolved from
    /// an ATS posting URL.
    pub fn new(identity: JobIdentity) -> Self {
        Self {
            identity,
            source: None,
            source_external_id: None,
            watches: Vec::new(),
        }
    }

    /// Pure provider target resolution for this posting.
    pub fn provider_target(&self) -> Option<ProviderTarget> {
        resolve_provider_target(&ProviderTargetInput {
            source: self.source.as_deref(),
            source_external_id: self.source_external_id.as_deref(),
            url: &self.identity.posting_url,
            watches: &self.watches,
        })
    }
}

/// Gather Check_Evidence for one posting (design.md, "Evaluation flow").
///
/// 1. Resolve a provider target. With a target, fetch its board listing once
///    per run through `cache` and record the provider signal.
///    - `ListedOpen`: done, the page is not fetched (Req 6.1).
///    - `AbsentFromListing` / `ListedClosed`: fetch the page for confirmation,
///      so a live but unlisted page yields conflicting evidence (Req 6.4, 6.8).
///    - `ListingUnavailable`: fall back to the page. The listing failure is
///      recorded only as that provider signal, never as `evidence.failure`, so
///      it cannot block positive page evidence.
/// 2. Without a target, fetch the page only.
///
/// Page evidence comes from the final response: final URL, final HTTP status,
/// and every redirect status in order (Req 6.12, 7.2). `evidence.failure` is
/// set only when the page fetch failed. The body is dropped right after signal
/// extraction. The result is normalized (URLs sanitized, `final_url` dropped
/// when equal to the requested URL).
///
/// No DB access and no clock: `attempted_at` is supplied by the caller.
pub async fn evaluate<F: PostingFetcher>(
    input: &PostingCheckInput,
    fetcher: &F,
    cache: &ProviderListingCache,
    attempted_at: String,
) -> CheckEvidence {
    let identity = &input.identity;
    let mut evidence = CheckEvidence::new(&identity.posting_url, attempted_at);

    if let Some(target) = input.provider_target() {
        let listing = cache
            .get_or_fetch(target.provider, &target.board_slug, || {
                fetcher.fetch_listing(target.provider, &target.board_slug)
            })
            .await;
        let signal = target.signal(&listing);
        let listed_open = signal.signal == ProviderSignalKind::ListedOpen;
        evidence.provider = Some(signal);
        if listed_open {
            return evidence.normalized();
        }
    }

    let page = fetcher.fetch_page(&identity.posting_url).await;
    apply_page(&mut evidence, page, identity);
    evidence.normalized()
}

/// Fold a page fetch into `evidence`, consuming (and so dropping) the body.
fn apply_page(evidence: &mut CheckEvidence, page: PageFetch, identity: &JobIdentity) {
    let PageFetch {
        requested_url,
        final_url,
        http_status,
        redirect_statuses,
        signal_headers,
        body,
        error_kind,
    } = page;

    evidence.http_status = http_status;
    evidence.redirect_statuses = redirect_statuses;
    if !final_url.trim().is_empty() {
        evidence.final_url = Some(final_url.clone());
    }

    if let Some(kind) = error_kind {
        evidence.failure = Some(FailureCategory::from(kind));
        // A failed fetch may still carry the challenge header of its last response.
        if signals::anti_bot_header(&signal_headers) {
            evidence.content.insert(ContentSignal::AntiBot);
        }
        return;
    }

    let final_parsed = Url::parse(&final_url).ok();
    let requested_parsed = Url::parse(&requested_url)
        .ok()
        .or_else(|| final_parsed.clone());
    match (requested_parsed, final_parsed) {
        (Some(requested), Some(final_url)) => {
            evidence.content =
                signals::extract_signals(&body, identity, &requested, &final_url, &signal_headers);
        }
        _ => {
            if signals::anti_bot_header(&signal_headers) {
                evidence.content.insert(ContentSignal::AntiBot);
            }
        }
    }
    drop(body);
}

#[cfg(test)]
mod tests {
    use super::classify::{classify, reason_code};
    use super::evidence::{FailureCategory, Provider, ProviderSignal};
    use super::fetch::PageFetch;
    use super::provider::ListingFetch;
    use super::*;
    use crate::jobs::safe_fetch::FetchErrorKind;
    use crate::runs::model::PostingState;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const AT: &str = "2026-01-02T03:04:05Z";
    const GH_URL: &str = "https://boards.greenhouse.io/acme/jobs/127817";

    /// Scripted fetcher. Unscripted requests panic, which proves a request was
    /// not made. Records every call.
    #[derive(Default)]
    struct FakeFetcher {
        pages: HashMap<String, PageFetch>,
        listings: HashMap<(Provider, String), ListingFetch>,
        page_calls: Mutex<Vec<String>>,
        listing_calls: Mutex<Vec<(Provider, String)>>,
    }

    impl FakeFetcher {
        fn page(mut self, url: &str, page: PageFetch) -> Self {
            self.pages.insert(url.to_string(), page);
            self
        }
        fn listing(mut self, provider: Provider, slug: &str, fetch: ListingFetch) -> Self {
            self.listings.insert((provider, slug.to_string()), fetch);
            self
        }
        fn page_calls(&self) -> Vec<String> {
            self.page_calls.lock().unwrap().clone()
        }
        fn listing_calls(&self) -> Vec<(Provider, String)> {
            self.listing_calls.lock().unwrap().clone()
        }
    }

    impl PostingFetcher for FakeFetcher {
        async fn fetch_page(&self, url: &str) -> PageFetch {
            self.page_calls.lock().unwrap().push(url.to_string());
            self.pages
                .get(url)
                .cloned()
                .unwrap_or_else(|| panic!("unscripted page {url}"))
        }
        async fn fetch_listing(&self, provider: Provider, board_slug: &str) -> ListingFetch {
            self.listing_calls
                .lock()
                .unwrap()
                .push((provider, board_slug.to_string()));
            self.listings
                .get(&(provider, board_slug.to_string()))
                .cloned()
                .unwrap_or_else(|| panic!("unscripted listing {provider:?}/{board_slug}"))
        }
    }

    fn identity(url: &str) -> JobIdentity {
        JobIdentity {
            job_id: "job-1".into(),
            title: "Senior Engineer, Payments".into(),
            company_name: "Acme".into(),
            posting_url: url.into(),
        }
    }

    fn gh_input(url: &str, posting_id: &str) -> PostingCheckInput {
        PostingCheckInput {
            identity: identity(url),
            source: Some("greenhouse".into()),
            source_external_id: Some(posting_id.into()),
            watches: vec![],
        }
    }

    const OPEN_PAGE: &str = r#"<!doctype html><html><head>
        <title>Senior Engineer, Payments | Acme Careers</title>
        <meta property="og:title" content="Senior Engineer, Payments">
        <script type="application/ld+json">
          {"@context":"https://schema.org","@type":"JobPosting",
           "title":"Senior Engineer, Payments",
           "hiringOrganization":{"@type":"Organization","name":"Acme"}}
        </script></head><body>
        <h1>Senior Engineer, Payments</h1>
        <a class="btn" href="/acme/jobs/127817/apply">Apply for this job</a>
        </body></html>"#;

    fn page(url: &str, status: u16, body: &str) -> PageFetch {
        PageFetch {
            requested_url: url.into(),
            final_url: url.into(),
            http_status: Some(status),
            redirect_statuses: vec![],
            signal_headers: vec![],
            body: body.into(),
            error_kind: None,
        }
    }

    fn failed_page(url: &str, status: Option<u16>, kind: FetchErrorKind) -> PageFetch {
        PageFetch {
            requested_url: url.into(),
            final_url: url.into(),
            http_status: status,
            redirect_statuses: vec![],
            signal_headers: vec![],
            body: String::new(),
            error_kind: Some(kind),
        }
    }

    fn listing(body: &str) -> ListingFetch {
        ListingFetch::Fetched {
            http_status: 200,
            body: body.into(),
        }
    }

    const LISTED: &str = r#"{"jobs":[{"id":127817,"title":"Senior Engineer, Payments"}]}"#;
    const NOT_LISTED: &str = r#"{"jobs":[{"id":1,"title":"Other"}]}"#;

    async fn run<F: PostingFetcher>(input: &PostingCheckInput, f: &F) -> CheckEvidence {
        evaluate(input, f, &ProviderListingCache::new(), AT.into()).await
    }

    #[tokio::test]
    async fn listed_open_skips_the_page_fetch() {
        let f = FakeFetcher::default().listing(Provider::Greenhouse, "acme", listing(LISTED));
        let ev = run(&gh_input(GH_URL, "127817"), &f).await;
        assert!(f.page_calls().is_empty());
        assert_eq!(
            f.listing_calls(),
            vec![(Provider::Greenhouse, "acme".to_string())]
        );
        assert_eq!(
            ev.provider,
            Some(ProviderSignal {
                provider: Provider::Greenhouse,
                signal: ProviderSignalKind::ListedOpen,
                posting_id: "127817".into(),
            })
        );
        assert_eq!(ev.http_status, None);
        assert!(ev.content.is_empty() && ev.failure.is_none());
        assert_eq!(ev.attempted_at, AT);
        let c = classify(&ev);
        assert_eq!(c.state, PostingState::Active);
        assert_eq!(c.reason_code, reason_code::LISTED_OPEN);
    }

    #[tokio::test]
    async fn absent_with_live_page_is_a_conflict() {
        let f = FakeFetcher::default()
            .listing(Provider::Greenhouse, "acme", listing(NOT_LISTED))
            .page(GH_URL, page(GH_URL, 200, OPEN_PAGE));
        let ev = run(&gh_input(GH_URL, "127817"), &f).await;
        assert_eq!(f.page_calls(), vec![GH_URL.to_string()]);
        assert_eq!(
            ev.provider.as_ref().map(|p| p.signal),
            Some(ProviderSignalKind::AbsentFromListing)
        );
        assert_eq!(
            ev.content,
            [
                ContentSignal::TitleMatch,
                ContentSignal::CompanyMatch,
                ContentSignal::ApplyEnabled
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(ev.http_status, Some(200));
        let c = classify(&ev);
        assert_eq!(c.state, PostingState::Unknown);
        assert_eq!(c.reason_code, reason_code::CONFLICT);
    }

    #[tokio::test]
    async fn absent_with_404_page_is_closed() {
        let f = FakeFetcher::default()
            .listing(Provider::Greenhouse, "acme", listing(NOT_LISTED))
            .page(
                GH_URL,
                page(GH_URL, 404, "<html><title>Not found</title></html>"),
            );
        let ev = run(&gh_input(GH_URL, "127817"), &f).await;
        assert_eq!(ev.http_status, Some(404));
        assert_eq!(
            ev.provider.as_ref().map(|p| p.signal),
            Some(ProviderSignalKind::AbsentFromListing)
        );
        assert_eq!(classify(&ev).state, PostingState::Inactive);
    }

    #[tokio::test]
    async fn listing_failure_falls_back_to_the_page() {
        let f = FakeFetcher::default()
            .listing(
                Provider::Greenhouse,
                "acme",
                ListingFetch::Failed {
                    http_status: Some(503),
                    failure: Some(FailureCategory::ProviderTemporary),
                },
            )
            .page(GH_URL, page(GH_URL, 200, OPEN_PAGE));
        let ev = run(&gh_input(GH_URL, "127817"), &f).await;
        assert_eq!(f.page_calls().len(), 1);
        assert_eq!(
            ev.provider.as_ref().map(|p| p.signal),
            Some(ProviderSignalKind::ListingUnavailable {
                http_status: Some(503)
            })
        );
        // The listing failure is not an evidence-level failure (not a blocker).
        assert_eq!(ev.failure, None);
        let c = classify(&ev);
        assert_eq!(c.state, PostingState::Active);
        assert_eq!(c.reason_code, reason_code::PAGE_CONFIRMED_OPEN);
    }

    #[tokio::test]
    async fn page_redirect_evidence_is_retained_and_sanitized() {
        let url = "https://acme.com/careers/senior-engineer-payments";
        let mut p = page(url, 200, "<html><head><title>Careers at Acme</title></head><body><h1>Open roles</h1></body></html>");
        p.final_url = "https://acme.com/careers?token=abc123".into();
        p.redirect_statuses = vec![301, 302];
        let f = FakeFetcher::default().page(url, p);
        let ev = run(&PostingCheckInput::new(identity(url)), &f).await;
        assert_eq!(ev.requested_url, url);
        assert_eq!(
            ev.final_url.as_deref(),
            Some("https://acme.com/careers?token=redacted")
        );
        assert_eq!(ev.redirect_statuses, vec![301, 302]);
        assert_eq!(ev.http_status, Some(200));
        assert!(
            ev.content.contains(&ContentSignal::GenericCareers),
            "{:?}",
            ev.content
        );
        assert_ne!(classify(&ev).state, PostingState::Active);
    }

    #[tokio::test]
    async fn page_fetch_failure_maps_to_its_failure_category() {
        let url = "https://acme.com/careers/senior-engineer-payments";
        let cases = [
            (None, FetchErrorKind::Timeout, FailureCategory::Timeout),
            (
                None,
                FetchErrorKind::DnsTimeout,
                FailureCategory::DnsTimeout,
            ),
            (
                None,
                FetchErrorKind::BlockedDestination,
                FailureCategory::BlockedDestination,
            ),
            (
                Some(200),
                FetchErrorKind::TooLarge,
                FailureCategory::TooLarge,
            ),
            (
                Some(301),
                FetchErrorKind::RedirectMissingLocation,
                FailureCategory::RedirectFailure,
            ),
        ];
        for (status, kind, expected) in cases {
            let f = FakeFetcher::default().page(url, failed_page(url, status, kind));
            let ev = run(&PostingCheckInput::new(identity(url)), &f).await;
            assert_eq!(ev.failure, Some(expected), "{kind:?}");
            assert_eq!(ev.http_status, status, "{kind:?}");
            assert!(ev.content.is_empty(), "{kind:?}");
            assert_eq!(classify(&ev).state, PostingState::Unknown, "{kind:?}");
        }
    }

    #[tokio::test]
    async fn no_provider_target_fetches_the_page_only() {
        let url = "https://careers.acme.com/jobs/senior-engineer-123";
        let f = FakeFetcher::default().page(url, page(url, 200, OPEN_PAGE));
        let ev = run(&PostingCheckInput::new(identity(url)), &f).await;
        assert!(f.listing_calls().is_empty());
        assert_eq!(f.page_calls(), vec![url.to_string()]);
        assert_eq!(ev.provider, None);
        assert_eq!(ev.final_url, None); // equal to requested, dropped by normalization
        assert_eq!(classify(&ev).state, PostingState::Active);
    }

    #[tokio::test]
    async fn cache_shares_one_listing_fetch_across_postings() {
        let other_url = "https://boards.greenhouse.io/acme/jobs/555";
        let f = FakeFetcher::default()
            .listing(Provider::Greenhouse, "acme", listing(LISTED))
            .page(other_url, page(other_url, 404, ""));
        let cache = ProviderListingCache::new();
        let a = evaluate(&gh_input(GH_URL, "127817"), &f, &cache, AT.into()).await;
        let b = evaluate(&gh_input(other_url, "555"), &f, &cache, AT.into()).await;
        assert_eq!(f.listing_calls().len(), 1);
        assert_eq!(
            a.provider.map(|p| p.signal),
            Some(ProviderSignalKind::ListedOpen)
        );
        assert_eq!(
            b.provider.map(|p| p.signal),
            Some(ProviderSignalKind::AbsentFromListing)
        );
        assert_eq!(f.page_calls(), vec![other_url.to_string()]);
    }

    #[tokio::test]
    async fn anti_bot_header_on_a_failed_fetch_is_kept() {
        let url = "https://acme.com/careers/senior-engineer-payments";
        let mut p = failed_page(url, Some(403), FetchErrorKind::TooLarge);
        p.signal_headers = vec![("cf-mitigated".into(), "challenge".into())];
        let f = FakeFetcher::default().page(url, p);
        let ev = run(&PostingCheckInput::new(identity(url)), &f).await;
        assert!(ev.content.contains(&ContentSignal::AntiBot));
        assert_eq!(classify(&ev).state, PostingState::Unknown);
    }
}
