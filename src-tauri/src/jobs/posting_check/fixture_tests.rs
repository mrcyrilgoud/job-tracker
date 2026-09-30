//! Table-driven classification fixture tests (Req 13.1, 13.2, 13.3).
//!
//! Every row runs a synthetic fixture (provider listing and/or posting page,
//! loaded with `include_str!` from `fixtures/`) through the real
//! [`evaluate`] with a scripted fetcher, then through [`classify`], and
//! asserts the Posting_State and `reason_code`. The fixtures are small and
//! synthetic; none is copied from a real company page.
//!
//! Each fixture body carries a `jt-canary` marker so the rows can also assert
//! that body text never reaches the serialized evidence or the reason
//! (Req 7.4).

use std::collections::HashMap;
use std::sync::Mutex;

use super::classify::{classify, reason_code, state_label, Classification};
use super::evidence::{CheckEvidence, FailureCategory, Provider, ProviderSignalKind};
use super::fetch::{PageFetch, PostingFetcher};
use super::provider::{ListingFetch, ProviderListingCache};
use super::{evaluate, PostingCheckInput};
use crate::jobs::safe_fetch::FetchErrorKind;
use crate::runs::model::{JobIdentity, PostingState};
use crate::runs::progress::MAX_REASON_BYTES;

// --- Fixtures -------------------------------------------------------------

mod listings {
    pub const GREENHOUSE_OPEN: &str = include_str!("fixtures/listings/greenhouse_open.json");
    pub const GREENHOUSE_ABSENT: &str = include_str!("fixtures/listings/greenhouse_absent.json");
    pub const GREENHOUSE_MALFORMED: &str =
        include_str!("fixtures/listings/greenhouse_malformed.json");
    pub const GREENHOUSE_EMPTY: &str = include_str!("fixtures/listings/greenhouse_empty.json");
    pub const LEVER_OPEN: &str = include_str!("fixtures/listings/lever_open.json");
    pub const LEVER_ABSENT: &str = include_str!("fixtures/listings/lever_absent.json");
    pub const LEVER_MALFORMED: &str = include_str!("fixtures/listings/lever_malformed.json");
    pub const LEVER_EMPTY: &str = include_str!("fixtures/listings/lever_empty.json");
    pub const ASHBY_OPEN: &str = include_str!("fixtures/listings/ashby_open.json");
    pub const ASHBY_ABSENT: &str = include_str!("fixtures/listings/ashby_absent.json");
    pub const ASHBY_MALFORMED: &str = include_str!("fixtures/listings/ashby_malformed.json");
    pub const ASHBY_EMPTY: &str = include_str!("fixtures/listings/ashby_empty.json");
}

mod pages {
    pub const GENERIC_POSTING_JSONLD: &str =
        include_str!("fixtures/pages/generic_posting_jsonld.html");
    pub const GENERIC_CLOSED: &str = include_str!("fixtures/pages/generic_closed.html");
    pub const CAREERS_INDEX: &str = include_str!("fixtures/pages/careers_index.html");
    pub const GREENHOUSE_POSTING: &str = include_str!("fixtures/pages/greenhouse_posting.html");
    pub const GREENHOUSE_ERROR_REDIRECT: &str =
        include_str!("fixtures/pages/greenhouse_error_redirect.html");
    pub const LEVER_POSTING: &str = include_str!("fixtures/pages/lever_posting.html");
    pub const LEVER_CLOSED: &str = include_str!("fixtures/pages/lever_closed.html");
    pub const ASHBY_POSTING: &str = include_str!("fixtures/pages/ashby_posting.html");
    pub const ASHBY_CLOSED: &str = include_str!("fixtures/pages/ashby_closed.html");
    pub const WORKDAY_JS_SHELL: &str = include_str!("fixtures/pages/workday_js_shell.html");
    pub const CLOUDFLARE_CHALLENGE: &str = include_str!("fixtures/pages/cloudflare_challenge.html");
    pub const CONSENT_WALL: &str = include_str!("fixtures/pages/consent_wall.html");
    pub const SSO_LOGIN: &str = include_str!("fixtures/pages/sso_login.html");
    pub const ACCESS_DENIED: &str = include_str!("fixtures/pages/access_denied.html");
    /// Neutral body used for the status-only cases.
    pub const STATUS_ERROR: &str = include_str!("fixtures/pages/status_error.html");

    pub const ALL: &[(&str, &str)] = &[
        ("generic_posting_jsonld", GENERIC_POSTING_JSONLD),
        ("generic_closed", GENERIC_CLOSED),
        ("careers_index", CAREERS_INDEX),
        ("greenhouse_posting", GREENHOUSE_POSTING),
        ("greenhouse_error_redirect", GREENHOUSE_ERROR_REDIRECT),
        ("lever_posting", LEVER_POSTING),
        ("lever_closed", LEVER_CLOSED),
        ("ashby_posting", ASHBY_POSTING),
        ("ashby_closed", ASHBY_CLOSED),
        ("workday_js_shell", WORKDAY_JS_SHELL),
        ("cloudflare_challenge", CLOUDFLARE_CHALLENGE),
        ("consent_wall", CONSENT_WALL),
        ("sso_login", SSO_LOGIN),
        ("access_denied", ACCESS_DENIED),
        ("status_error", STATUS_ERROR),
    ];
}

/// Marker present in every page fixture (and most listings).
const CANARY: &str = "jt-canary";
const AT: &str = "2026-01-02T03:04:05Z";

// --- Identities -----------------------------------------------------------

const GENERIC_URL: &str = "https://careers.umbrella.example/jobs/platform-engineer-4821";
const WORKDAY_URL: &str =
    "https://umbrella.wd5.myworkdayjobs.com/en-US/External/job/Remote-USA/Platform-Engineer_R4821";
const GH_URL: &str = "https://boards.greenhouse.io/acme/jobs/127817";
const GH_ID: &str = "127817";
const LEVER_ID: &str = "5f1c2a9e-0b7d-4c3e-9a51-2d8f6e4b7c10";
const LEVER_URL: &str = "https://jobs.lever.co/globex/5f1c2a9e-0b7d-4c3e-9a51-2d8f6e4b7c10";
const ASHBY_ID: &str = "9b2e4f6a-1c3d-4e5f-8a7b-6c5d4e3f2a1b";
const ASHBY_URL: &str = "https://jobs.ashbyhq.com/initech/9b2e4f6a-1c3d-4e5f-8a7b-6c5d4e3f2a1b";

fn identity(title: &str, company: &str, url: &str) -> JobIdentity {
    JobIdentity {
        job_id: "job-fixture".into(),
        title: title.into(),
        company_name: company.into(),
        posting_url: url.into(),
    }
}

fn generic_input(url: &str) -> PostingCheckInput {
    PostingCheckInput::new(identity("Platform Engineer", "Umbrella", url))
}

/// Watch-sourced job: `source` + `source_external_id` set, slug from the URL.
fn provider_input(provider: Provider) -> PostingCheckInput {
    let (title, company, url, id) = match provider {
        Provider::Greenhouse => ("Senior Engineer, Payments", "Acme", GH_URL, GH_ID),
        Provider::Lever => ("Staff Data Scientist", "Globex", LEVER_URL, LEVER_ID),
        Provider::Ashby => ("Product Designer", "Initech", ASHBY_URL, ASHBY_ID),
    };
    PostingCheckInput {
        identity: identity(title, company, url),
        source: Some(provider.as_str().into()),
        source_external_id: Some(id.into()),
        watches: vec![],
    }
}

fn slug(provider: Provider) -> &'static str {
    match provider {
        Provider::Greenhouse => "acme",
        Provider::Lever => "globex",
        Provider::Ashby => "initech",
    }
}

// --- Fake fetcher ---------------------------------------------------------

/// Scripted fetcher. An unscripted request panics, which proves a request
/// was not made. Records every page call.
#[derive(Default)]
struct FakeFetcher {
    pages: HashMap<String, PageFetch>,
    listings: HashMap<(Provider, String), ListingFetch>,
    page_calls: Mutex<Vec<String>>,
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
        self.listings
            .get(&(provider, board_slug.to_string()))
            .cloned()
            .unwrap_or_else(|| panic!("unscripted listing {provider:?}/{board_slug}"))
    }
}

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

fn redirected(url: &str, final_url: &str, redirects: &[u16], status: u16, body: &str) -> PageFetch {
    PageFetch {
        final_url: final_url.into(),
        redirect_statuses: redirects.to_vec(),
        ..page(url, status, body)
    }
}

fn with_cf_challenge_header(mut p: PageFetch) -> PageFetch {
    p.signal_headers = vec![("cf-mitigated".into(), "challenge".into())];
    p
}

fn timed_out(url: &str) -> PageFetch {
    PageFetch {
        http_status: None,
        body: String::new(),
        error_kind: Some(FetchErrorKind::Timeout),
        ..page(url, 0, "")
    }
}

fn fetched(body: &str) -> ListingFetch {
    ListingFetch::Fetched {
        http_status: 200,
        body: body.into(),
    }
}

// --- Table ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Generic,
    Greenhouse,
    Lever,
    Ashby,
    StatusOnly,
}

struct Case {
    name: &'static str,
    group: Group,
    input: PostingCheckInput,
    /// Scripted listing for the input's provider target, if any.
    listing: Option<ListingFetch>,
    /// Scripted page. `None` asserts the page is never fetched.
    page: Option<PageFetch>,
    /// Expected provider signal kind (Req 7.3). `None` means no provider evidence.
    provider_signal: Option<ProviderSignalKind>,
    state: PostingState,
    code: String,
    /// Substrings the reason must contain (decisive categories).
    reason_contains: &'static [&'static str],
}

#[allow(clippy::too_many_arguments)]
fn case(
    name: &'static str,
    group: Group,
    input: PostingCheckInput,
    listing: Option<ListingFetch>,
    page: Option<PageFetch>,
    provider_signal: Option<ProviderSignalKind>,
    state: PostingState,
    code: impl Into<String>,
    reason_contains: &'static [&'static str],
) -> Case {
    Case {
        name,
        group,
        input,
        listing,
        page,
        provider_signal,
        state,
        code: code.into(),
        reason_contains,
    }
}

use PostingState::{Active, Inactive, Unknown};
use ProviderSignalKind::{AbsentFromListing, ListedOpen, ListingUnavailable};

fn cases() -> Vec<Case> {
    use listings as l;
    use pages as p;
    use Group::*;
    let gh = || provider_input(Provider::Greenhouse);
    let lv = || provider_input(Provider::Lever);
    let ab = || provider_input(Provider::Ashby);
    let gen = || generic_input(GENERIC_URL);
    let rc = reason_code::TRANSIENT_PREFIX;

    vec![
        // ---- Generic HTML (no provider target) ----
        case("generic_jsonld_posting_enabled_apply", Generic, gen(), None,
            Some(page(GENERIC_URL, 200, p::GENERIC_POSTING_JSONLD)), None,
            Active, reason_code::PAGE_CONFIRMED_OPEN, &["enabled Apply control", "HTTP 200"]),
        case("generic_closure_copy", Generic, gen(), None,
            Some(page(GENERIC_URL, 200, p::GENERIC_CLOSED)), None,
            Inactive, reason_code::CLOSURE_COPY, &["closure copy"]),
        case("generic_redirect_to_careers_index", Generic, gen(), None,
            Some(redirected(GENERIC_URL, "https://www.umbrella.example/careers", &[301], 200, p::CAREERS_INDEX)),
            None, Unknown, reason_code::GENERIC_CAREERS, &["redirected to a generic careers page"]),
        case("generic_workday_js_shell", Generic, generic_input(WORKDAY_URL), None,
            Some(page(WORKDAY_URL, 200, p::WORKDAY_JS_SHELL)), None,
            Unknown, reason_code::NO_CONCLUSIVE_EVIDENCE, &["no conclusive evidence"]),
        case("generic_cloudflare_challenge_body", Generic, gen(), None,
            Some(page(GENERIC_URL, 403, p::CLOUDFLARE_CHALLENGE)), None,
            Unknown, reason_code::BLOCKED_ANTI_BOT, &["anti-bot", "HTTP 403"]),
        // Header alone on an otherwise positive page: the blocker dominates (Req 6.14).
        case("generic_cf_mitigated_header_on_posting_page", Generic, gen(), None,
            Some(with_cf_challenge_header(page(GENERIC_URL, 200, p::GENERIC_POSTING_JSONLD))), None,
            Unknown, reason_code::BLOCKED_ANTI_BOT, &["anti-bot"]),
        case("generic_consent_wall", Generic, gen(), None,
            Some(redirected(
                GENERIC_URL,
                "https://consent.umbrella.example/?continue=https%3A%2F%2Fcareers.umbrella.example%2Fjobs%2Fplatform-engineer-4821",
                &[302], 200, p::CONSENT_WALL)),
            None, Unknown, reason_code::BLOCKED_CONSENT, &["consent page"]),
        case("generic_sso_login", Generic, gen(), None,
            Some(redirected(GENERIC_URL, "https://login.umbrella.example/sso/saml2?RelayState=r1", &[302, 302], 200, p::SSO_LOGIN)),
            None, Unknown, reason_code::BLOCKED_AUTH, &["sign-in page"]),
        case("generic_access_denied", Generic, gen(), None,
            Some(page(GENERIC_URL, 403, p::ACCESS_DENIED)), None,
            Unknown, reason_code::BLOCKED_ACCESS_DENIED, &["access-denied page", "HTTP 403"]),

        // ---- Greenhouse ----
        case("greenhouse_listed_open", Greenhouse, gh(), Some(fetched(l::GREENHOUSE_OPEN)), None,
            Some(ListedOpen), Active, reason_code::LISTED_OPEN, &["Greenhouse", GH_ID]),
        case("greenhouse_absent_page_404", Greenhouse, gh(), Some(fetched(l::GREENHOUSE_ABSENT)),
            Some(page(GH_URL, 404, p::STATUS_ERROR)), Some(AbsentFromListing),
            Inactive, reason_code::HTTP_GONE, &["HTTP 404", "absent from Greenhouse listing"]),
        case("greenhouse_absent_error_true_redirect", Greenhouse, gh(), Some(fetched(l::GREENHOUSE_ABSENT)),
            Some(redirected(GH_URL, "https://boards.greenhouse.io/acme?error=true", &[302], 200, p::GREENHOUSE_ERROR_REDIRECT)),
            Some(AbsentFromListing), Inactive, reason_code::ABSENT_FROM_LISTING, &["absent from Greenhouse listing"]),
        case("greenhouse_absent_but_live_page_conflict", Greenhouse, gh(), Some(fetched(l::GREENHOUSE_ABSENT)),
            Some(page(GH_URL, 200, p::GREENHOUSE_POSTING)), Some(AbsentFromListing),
            Unknown, reason_code::CONFLICT, &["conflicting evidence", "absent from Greenhouse listing"]),
        case("greenhouse_malformed_listing_error_redirect", Greenhouse, gh(), Some(fetched(l::GREENHOUSE_MALFORMED)),
            Some(redirected(GH_URL, "https://boards.greenhouse.io/acme?error=true", &[302], 200, p::GREENHOUSE_ERROR_REDIRECT)),
            Some(ListingUnavailable { http_status: Some(200) }),
            Unknown, reason_code::GENERIC_CAREERS, &["generic careers page", "Greenhouse listing unavailable"]),
        case("greenhouse_empty_listing_page_404", Greenhouse, gh(), Some(fetched(l::GREENHOUSE_EMPTY)),
            Some(page(GH_URL, 404, p::STATUS_ERROR)), Some(AbsentFromListing),
            Inactive, reason_code::HTTP_GONE, &["HTTP 404"]),
        case("greenhouse_malformed_listing_live_page", Greenhouse, gh(), Some(fetched(l::GREENHOUSE_MALFORMED)),
            Some(page(GH_URL, 200, p::GREENHOUSE_POSTING)), Some(ListingUnavailable { http_status: Some(200) }),
            Active, reason_code::PAGE_CONFIRMED_OPEN, &["enabled Apply control"]),

        // ---- Lever ----
        case("lever_listed_open", Lever, lv(), Some(fetched(l::LEVER_OPEN)), None,
            Some(ListedOpen), Active, reason_code::LISTED_OPEN, &["Lever", LEVER_ID]),
        case("lever_absent_closure_copy", Lever, lv(), Some(fetched(l::LEVER_ABSENT)),
            Some(page(LEVER_URL, 200, p::LEVER_CLOSED)), Some(AbsentFromListing),
            Inactive, reason_code::ABSENT_FROM_LISTING, &["absent from Lever listing", "closure copy"]),
        case("lever_empty_listing_live_page_conflict", Lever, lv(), Some(fetched(l::LEVER_EMPTY)),
            Some(page(LEVER_URL, 200, p::LEVER_POSTING)), Some(AbsentFromListing),
            Unknown, reason_code::CONFLICT, &["conflicting evidence", "absent from Lever listing"]),
        case("lever_malformed_listing_page_429", Lever, lv(), Some(fetched(l::LEVER_MALFORMED)),
            Some(page(LEVER_URL, 429, p::STATUS_ERROR)), Some(ListingUnavailable { http_status: Some(200) }),
            Unknown, reason_code::TRANSIENT_HTTP_429, &["HTTP 429"]),
        case("lever_malformed_listing_live_page", Lever, lv(), Some(fetched(l::LEVER_MALFORMED)),
            Some(page(LEVER_URL, 200, p::LEVER_POSTING)), Some(ListingUnavailable { http_status: Some(200) }),
            Active, reason_code::PAGE_CONFIRMED_OPEN, &["enabled Apply control", "Lever listing unavailable"]),

        // ---- Ashby ----
        case("ashby_listed_open", Ashby, ab(), Some(fetched(l::ASHBY_OPEN)), None,
            Some(ListedOpen), Active, reason_code::LISTED_OPEN, &["Ashby", ASHBY_ID]),
        case("ashby_absent_page_410", Ashby, ab(), Some(fetched(l::ASHBY_ABSENT)),
            Some(page(ASHBY_URL, 410, p::STATUS_ERROR)), Some(AbsentFromListing),
            Inactive, reason_code::HTTP_GONE, &["HTTP 410", "absent from Ashby listing"]),
        case("ashby_empty_listing_closure_copy", Ashby, ab(), Some(fetched(l::ASHBY_EMPTY)),
            Some(page(ASHBY_URL, 200, p::ASHBY_CLOSED)), Some(AbsentFromListing),
            Inactive, reason_code::ABSENT_FROM_LISTING, &["absent from Ashby listing", "closure copy"]),
        case("ashby_malformed_listing_cloudflare", Ashby, ab(), Some(fetched(l::ASHBY_MALFORMED)),
            Some(with_cf_challenge_header(page(ASHBY_URL, 403, p::CLOUDFLARE_CHALLENGE))),
            Some(ListingUnavailable { http_status: Some(200) }),
            Unknown, reason_code::BLOCKED_ANTI_BOT, &["anti-bot", "Ashby listing unavailable"]),
        // Ashby renders the Apply control client-side, so a server-rendered
        // page without it is not positive evidence.
        case("ashby_listing_503_js_page_without_apply", Ashby, ab(),
            Some(ListingFetch::Failed { http_status: Some(503), failure: Some(FailureCategory::ProviderTemporary) }),
            Some(page(ASHBY_URL, 200, p::ASHBY_POSTING)), Some(ListingUnavailable { http_status: Some(503) }),
            Unknown, reason_code::NO_CONCLUSIVE_EVIDENCE, &["no conclusive evidence", "Ashby listing unavailable"]),

        // ---- Status-only (generic URL, neutral body) ----
        case("status_404", StatusOnly, gen(), None, Some(page(GENERIC_URL, 404, p::STATUS_ERROR)), None,
            Inactive, reason_code::HTTP_GONE, &["HTTP 404"]),
        case("status_410", StatusOnly, gen(), None, Some(page(GENERIC_URL, 410, p::STATUS_ERROR)), None,
            Inactive, reason_code::HTTP_GONE, &["HTTP 410"]),
        case("status_401", StatusOnly, gen(), None, Some(page(GENERIC_URL, 401, p::STATUS_ERROR)), None,
            Unknown, reason_code::BLOCKED_HTTP_401, &["HTTP 401"]),
        case("status_403", StatusOnly, gen(), None, Some(page(GENERIC_URL, 403, p::STATUS_ERROR)), None,
            Unknown, reason_code::BLOCKED_HTTP_403, &["HTTP 403"]),
        case("status_429", StatusOnly, gen(), None, Some(page(GENERIC_URL, 429, p::STATUS_ERROR)), None,
            Unknown, reason_code::TRANSIENT_HTTP_429, &["HTTP 429"]),
        case("status_500", StatusOnly, gen(), None, Some(page(GENERIC_URL, 500, p::STATUS_ERROR)), None,
            Unknown, reason_code::TRANSIENT_HTTP_5XX, &["HTTP 500"]),
        case("status_503", StatusOnly, gen(), None, Some(page(GENERIC_URL, 503, p::STATUS_ERROR)), None,
            Unknown, reason_code::TRANSIENT_HTTP_5XX, &["HTTP 503"]),
        case("status_timeout", StatusOnly, gen(), None, Some(timed_out(GENERIC_URL)), None,
            Unknown, format!("{rc}{}", FailureCategory::Timeout.as_str()), &["timed out"]),
    ]
}

// --- Runner ---------------------------------------------------------------

fn fetcher_for(case: &Case) -> FakeFetcher {
    let mut f = FakeFetcher::default();
    if let Some(listing) = &case.listing {
        let target = case
            .input
            .provider_target()
            .expect("listing scripted without a provider target");
        f.listings
            .insert((target.provider, target.board_slug), listing.clone());
    }
    if let Some(p) = &case.page {
        f.pages
            .insert(case.input.identity.posting_url.clone(), p.clone());
    }
    f
}

async fn run_case(case: &Case) -> (CheckEvidence, Classification, Vec<String>) {
    let f = fetcher_for(case);
    let ev = evaluate(&case.input, &f, &ProviderListingCache::new(), AT.into()).await;
    let c = classify(&ev);
    let calls = f.page_calls.lock().unwrap().clone();
    (ev, c, calls)
}

/// Every check for one row. Returns the failures (empty when the row passes).
fn check_case(
    case: &Case,
    ev: &CheckEvidence,
    c: &Classification,
    page_calls: &[String],
) -> Vec<String> {
    let mut errs = Vec::new();
    let mut expect = |ok: bool, msg: String| {
        if !ok {
            errs.push(msg);
        }
    };

    expect(
        c.state == case.state,
        format!("state: expected {:?}, got {:?}", case.state, c.state),
    );
    expect(
        c.reason_code == case.code,
        format!("reason_code: expected {}, got {}", case.code, c.reason_code),
    );
    expect(!c.reason.trim().is_empty(), "reason is empty".into());
    expect(
        c.reason.len() <= MAX_REASON_BYTES,
        format!("reason is {} bytes", c.reason.len()),
    );
    expect(
        c.reason.starts_with(&format!("{}: ", state_label(c.state))),
        format!("reason label mismatch: {}", c.reason),
    );
    for needle in case.reason_contains {
        expect(
            c.reason.contains(needle),
            format!("reason {:?} does not contain {needle:?}", c.reason),
        );
    }

    // Page fetched iff scripted; `ListedOpen` skips it (Req 6.1).
    let expected_calls: Vec<String> = case
        .page
        .iter()
        .map(|_| case.input.identity.posting_url.clone())
        .collect();
    expect(
        page_calls == expected_calls,
        format!("page calls: expected {expected_calls:?}, got {page_calls:?}"),
    );

    // Provider evidence names the provider, the signal kind, and the id (Req 7.3).
    match (&case.provider_signal, &ev.provider) {
        (None, None) => {}
        (Some(kind), Some(sig)) => {
            let target = case.input.provider_target().unwrap();
            expect(
                sig.signal == *kind,
                format!("provider signal: expected {kind:?}, got {:?}", sig.signal),
            );
            expect(
                sig.provider == target.provider,
                format!("provider: got {:?}", sig.provider),
            );
            expect(
                sig.posting_id == target.posting_id,
                format!("posting id: got {}", sig.posting_id),
            );
        }
        (want, got) => errs.push(format!("provider evidence: expected {want:?}, got {got:?}")),
    }

    // The final response decides, and redirect evidence is retained (Req 6.12, 7.2).
    if let Some(p) = &case.page {
        if ev.http_status != p.http_status {
            errs.push(format!(
                "http_status: expected {:?}, got {:?}",
                p.http_status, ev.http_status
            ));
        }
        if ev.redirect_statuses != p.redirect_statuses {
            errs.push(format!(
                "redirects: expected {:?}, got {:?}",
                p.redirect_statuses, ev.redirect_statuses
            ));
        }
        if p.final_url != p.requested_url && ev.final_url.is_none() {
            errs.push("final_url of a redirect was dropped".into());
        }
    }

    // Body text never leaves the fetch layer (Req 7.4).
    let json = serde_json::to_string(ev).expect("evidence serializes");
    if json.contains(CANARY) || c.reason.contains(CANARY) {
        errs.push(format!(
            "fixture body leaked into evidence or reason: {json} / {}",
            c.reason
        ));
    }
    errs
}

/// Req 13.1, 13.2: every fixture row yields its expected state and code.
#[tokio::test]
async fn fixture_corpus_classifies_as_expected() {
    let mut failures = Vec::new();
    for case in cases() {
        let (ev, c, calls) = run_case(&case).await;
        let errs = check_case(&case, &ev, &c, &calls);
        if !errs.is_empty() {
            failures.push(format!(
                "[{}] {}\n    evidence: {ev:?}\n    classification: {c:?}",
                case.name,
                errs.join("; ")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} fixture row(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Req 13.1: the corpus has an Active, a Closed, and an Unknown example for
/// generic HTML and for each provider. Guards against rows being removed.
#[test]
fn corpus_covers_every_state_for_every_source() {
    let all = cases();
    for group in [
        Group::Generic,
        Group::Greenhouse,
        Group::Lever,
        Group::Ashby,
    ] {
        for state in [Active, Inactive, Unknown] {
            assert!(
                all.iter().any(|c| c.group == group && c.state == state),
                "no {state:?} fixture for {group:?}"
            );
        }
    }
    let mut names: Vec<_> = all.iter().map(|c| c.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all.len(), "duplicate fixture row names");
}

/// Every page fixture carries the canary, so the leak check is not vacuous.
#[test]
fn every_page_fixture_carries_the_canary() {
    for (name, body) in pages::ALL {
        assert!(body.contains(CANARY), "{name} has no {CANARY} marker");
    }
}

/// Req 13.3: case and whitespace variants of the identity and of the page's
/// title and closure copy keep the same state and reason_code.
#[tokio::test]
async fn case_and_whitespace_variants_keep_state_and_reason_code() {
    let title_variants: [fn(&str) -> String; 4] = [
        |s| s.to_string(),
        |s| s.to_lowercase(),
        |s| s.to_uppercase(),
        // Spaces only: the title also sits inside a JSON-LD string.
        |s| format!("  {}  ", s.replace(' ', "   ")),
    ];
    let closure_variants: [fn(&str) -> String; 4] = [
        |s| s.to_string(),
        |s| s.to_lowercase(),
        |s| s.to_uppercase(),
        |s| format!("\n  {}\t ", s.replace(' ', " \n\t  ")),
    ];
    let bases = [
        (
            "generic_posting_jsonld",
            pages::GENERIC_POSTING_JSONLD,
            Active,
            reason_code::PAGE_CONFIRMED_OPEN,
        ),
        (
            "generic_closed",
            pages::GENERIC_CLOSED,
            Inactive,
            reason_code::CLOSURE_COPY,
        ),
    ];
    const TITLE: &str = "Platform Engineer";
    const CLOSURE: &str = "This position has been filled.";

    let mut failures = Vec::new();
    for (name, body, state, code) in bases {
        for (ti, page_title) in title_variants.iter().enumerate() {
            for (ci, closure) in closure_variants.iter().enumerate() {
                for (ii, id_text) in title_variants.iter().enumerate() {
                    let html = body
                        .replace(TITLE, &page_title(TITLE))
                        .replace(CLOSURE, &closure(CLOSURE));
                    let input = PostingCheckInput::new(identity(
                        &id_text(TITLE),
                        &id_text("Umbrella"),
                        GENERIC_URL,
                    ));
                    let mut f = FakeFetcher::default();
                    f.pages
                        .insert(GENERIC_URL.into(), page(GENERIC_URL, 200, &html));
                    let ev = evaluate(&input, &f, &ProviderListingCache::new(), AT.into()).await;
                    let c = classify(&ev);
                    if c.state != state || c.reason_code != code {
                        failures.push(format!(
                            "[{name} page_title#{ti} closure#{ci} identity#{ii}] expected {state:?}/{code}, got {:?}/{} ({:?})",
                            c.state, c.reason_code, ev.content
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Sanity check on the fixture set itself: each listing parses (or fails to
/// parse) the way its file name claims, so a broken fixture file cannot turn
/// an "open" row into a silent fallback.
#[test]
fn listing_fixtures_parse_as_named() {
    use super::provider::ProviderListing;
    let rows: [(Provider, &str, Option<bool>); 12] = [
        (Provider::Greenhouse, listings::GREENHOUSE_OPEN, Some(true)),
        (
            Provider::Greenhouse,
            listings::GREENHOUSE_ABSENT,
            Some(false),
        ),
        (
            Provider::Greenhouse,
            listings::GREENHOUSE_EMPTY,
            Some(false),
        ),
        (Provider::Greenhouse, listings::GREENHOUSE_MALFORMED, None),
        (Provider::Lever, listings::LEVER_OPEN, Some(true)),
        (Provider::Lever, listings::LEVER_ABSENT, Some(false)),
        (Provider::Lever, listings::LEVER_EMPTY, Some(false)),
        (Provider::Lever, listings::LEVER_MALFORMED, None),
        (Provider::Ashby, listings::ASHBY_OPEN, Some(true)),
        (Provider::Ashby, listings::ASHBY_ABSENT, Some(false)),
        (Provider::Ashby, listings::ASHBY_EMPTY, Some(false)),
        (Provider::Ashby, listings::ASHBY_MALFORMED, None),
    ];
    for (provider, body, listed) in rows {
        let target = provider_input(provider).provider_target().unwrap();
        assert_eq!(target.board_slug, slug(provider));
        let listing = ProviderListing::from_fetch(provider, fetched(body));
        let signal = target.signal(&listing).signal;
        let expected = match listed {
            Some(true) => ListedOpen,
            Some(false) => AbsentFromListing,
            None => ListingUnavailable {
                http_status: Some(200),
            },
        };
        assert_eq!(signal, expected, "{provider:?} listing {body}");
    }
}
