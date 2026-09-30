//! HTML → content signal extraction (design.md, "Page signal extraction").
//!
//! [`extract_signals`] is pure: no network, no clock. It parses the page once,
//! derives [`ContentSignal`] categories, and drops the parsed document and every
//! extracted string before returning. Only the categories leave this module, so
//! body text, excerpts, and non-allowlisted headers are never retained
//! (Req 7.4).
//!
//! All text comparisons go through [`norm`] and token-bounded containment, so
//! letter case, entity encoding, punctuation, and whitespace runs never change
//! the result (Req 13.3).
//!
//! Where the design leaves a detail open, the rules lean towards *not* emitting
//! positive signals (`TitleMatch`, `CompanyMatch`, `ApplyEnabled`) and towards
//! emitting blocker signals. A false Unknown is cheaper than a false Active.

use std::collections::{BTreeSet, HashSet};

use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use url::Url;

use super::evidence::ContentSignal;
use crate::runs::model::JobIdentity;

/// Signal categories extracted from one page, in canonical order.
pub type ContentSignals = BTreeSet<ContentSignal>;

/// The only response header consulted, matching `safe_fetch`'s allowlist.
pub const ANTI_BOT_HEADER: &str = "cf-mitigated";

/// Closure phrases. The first six preserve the older heuristic's signals;
/// the rest come from the design.
/// Compared after [`norm`], so punctuation and case here do not matter.
pub const CLOSURE_PHRASES: &[&str] = &[
    "no longer accepting applications",
    "job is closed",
    "this job has expired",
    "position has been filled",
    "this posting is no longer available",
    "sorry, this job is no longer available",
    "no longer open",
    "no longer available",
    "job not found",
    "applications are closed",
    "this role has been filled",
    "posting has expired",
];

/// `<title>` / first `<h1>` phrases that mark a consent wall.
const CONSENT_PHRASES: &[&str] = &[
    "before you continue",
    "cookie consent",
    "cookie preferences",
    "cookie settings",
    "manage cookies",
    "manage consent",
    "consent required",
    "privacy choices",
    "privacy gateway",
    "we value your privacy",
];

/// Lowercased substrings of the final URL path that mark a consent wall.
const CONSENT_PATH_MARKERS: &[&str] = &["/consent", "/privacy-gateway", "/cookie"];

/// `<title>` prefixes that mark a sign-in page.
const SIGN_IN_TITLE_PREFIXES: &[&str] = &[
    "sign in",
    "signin",
    "sign on",
    "single sign on",
    "log in",
    "login",
    "authentication required",
];

/// Path segment prefixes that mark a sign-in page. A segment matches when it
/// starts with one of these and the next character (if any) is not a letter,
/// so `/login.aspx` and `/oauth2` match but `/authors` does not.
const AUTH_SEGMENT_PREFIXES: &[&str] = &["login", "signin", "sign-in", "sso", "oauth", "auth"];

/// Host prefixes that mark an identity provider.
const AUTH_HOST_PREFIXES: &[&str] = &["accounts.", "login.", "signin.", "sso.", "auth."];

/// Lowercased raw-HTML markers of a bot challenge.
const ANTI_BOT_MARKERS: &[&str] = &["_incapsula_resource", "px-captcha"];

/// Cloudflare's challenge script path. Cloudflare also injects
/// `…/challenge-platform/scripts/jsd/…` ("JavaScript detections") into ordinary
/// pages, so that one occurrence alone does not count as a challenge.
const CF_CHALLENGE_MARKER: &str = "challenge-platform";
const CF_JSD_MARKER: &str = "challenge-platform/scripts/jsd";

/// `<title>` / first `<h1>` phrases of an access-denied page.
const ACCESS_DENIED_PHRASES: &[&str] = &[
    "access denied",
    "forbidden",
    "you don't have permission",
    "you do not have permission",
];

/// Last path segments that look like a job listing, not a single posting.
const LISTING_LAST_SEGMENTS: &[&str] = &[
    "careers",
    "career",
    "jobs",
    "openings",
    "open-positions",
    "positions",
    "vacancies",
    "opportunities",
    "join-us",
];

/// Path segments that are followed by a posting id in a job-detail link.
const JOB_DETAIL_PARENT_SEGMENTS: &[&str] = &[
    "jobs",
    "job",
    "positions",
    "position",
    "careers",
    "openings",
    "opening",
    "vacancies",
    "vacancy",
    "postings",
    "posting",
    "roles",
    "role",
];

/// Segments after a job-detail parent that are listing controls, not ids.
const NON_POSTING_CHILD_SEGMENTS: &[&str] = &[
    "search",
    "filter",
    "filters",
    "page",
    "all",
    "department",
    "departments",
    "location",
    "locations",
    "team",
    "teams",
];

/// Distinct job-detail links needed to call a page a generic careers index.
const GENERIC_CAREERS_MIN_LINKS: usize = 3;

/// ATS board hosts whose first path segment is the board slug.
const ATS_BOARD_HOSTS: &[&str] = &[
    "boards.greenhouse.io",
    "job-boards.greenhouse.io",
    "jobs.lever.co",
    "jobs.eu.lever.co",
    "jobs.ashbyhq.com",
];

/// Registrable labels of ATS / job-board hosts. They name the vendor, never
/// the hiring company, so they never produce `CompanyMatch`.
const ATS_REGISTRABLE_LABELS: &[&str] = &[
    "ashbyhq",
    "bamboohr",
    "greenhouse",
    "icims",
    "jobvite",
    "lever",
    "linkedin",
    "myworkdayjobs",
    "smartrecruiters",
    "workable",
    "workday",
    "indeed",
    "glassdoor",
];

/// `og:site_name` substrings of ATS vendors (same list as `metadata.rs`).
const ATS_SITE_NAMES: &[&str] = &[
    "ashby",
    "bamboohr",
    "glassdoor",
    "greenhouse",
    "icims",
    "indeed",
    "jobvite",
    "lever",
    "linkedin",
    "smartrecruiters",
    "workable",
    "workday",
];

/// Second-level labels of two-part public suffixes such as `co.uk`.
const SECOND_LEVEL_SUFFIX_LABELS: &[&str] =
    &["co", "com", "org", "net", "ac", "gov", "edu", "ltd", "plc"];

/// Normalize text for matching: decode HTML entities, lowercase, collapse every
/// run of non-alphanumeric characters to one space, and trim.
///
/// `norm("  Senior&nbsp;Engineer — PAYMENTS ") == "senior engineer payments"`.
pub fn norm(s: &str) -> String {
    let decoded = html_escape::decode_html_entities(s);
    let mut out = String::with_capacity(decoded.len());
    let mut pending_space = false;
    for c in decoded.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        } else {
            pending_space = true;
        }
    }
    out
}

/// Token-bounded containment of an already-normalized `needle` in an
/// already-normalized `haystack`. An empty needle never matches.
fn contains_tokens(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    format!(" {haystack} ").contains(&format!(" {needle} "))
}

/// Token-bounded prefix test on normalized strings.
fn starts_with_tokens(haystack: &str, prefix: &str) -> bool {
    !prefix.is_empty()
        && (haystack == prefix
            || haystack
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with(' ')))
}

/// Normalized text with all spaces removed, for slug/host label comparison.
fn compact(s: &str) -> String {
    norm(s).replace(' ', "")
}

/// True when the allowlisted signal headers carry `cf-mitigated: challenge`.
/// Exposed so non-HTML responses can still be flagged as anti-bot.
pub fn anti_bot_header(signal_headers: &[(String, String)]) -> bool {
    signal_headers.iter().any(|(name, value)| {
        name.trim().eq_ignore_ascii_case(ANTI_BOT_HEADER)
            && value.trim().eq_ignore_ascii_case("challenge")
    })
}

/// Extract content signal categories from a fetched page.
///
/// - `requested` is the posting URL that was requested, `final_url` the URL of
///   the final response after redirects.
/// - `signal_headers` are the allowlisted response headers from `safe_fetch`
///   (only `cf-mitigated` is consulted).
///
/// Identity fields are `<title>`, `og:title`, the first `<h1>`, and JSON-LD
/// `JobPosting` fields. JSON-LD is used only when the page holds exactly one
/// `JobPosting`, so a listing page with many postings cannot match by accident.
pub fn extract_signals(
    html: &str,
    identity: &JobIdentity,
    requested: &Url,
    final_url: &Url,
    signal_headers: &[(String, String)],
) -> ContentSignals {
    let mut signals = ContentSignals::new();
    // Every string derived from the body lives in `page` and is dropped with it
    // at the end of this function.
    let page = PageFacts::parse(html, requested, final_url);

    let title_match = title_matches(&page, &identity.title);
    if title_match {
        signals.insert(ContentSignal::TitleMatch);
    }
    if company_matches(&page, &identity.company_name, final_url) {
        signals.insert(ContentSignal::CompanyMatch);
    }
    if page.apply_enabled {
        signals.insert(ContentSignal::ApplyEnabled);
    }
    if page.apply_disabled {
        signals.insert(ContentSignal::ApplyDisabled);
    }

    let same_path = path_key(requested) == path_key(final_url);
    if has_closure_copy(&page.visible_text) {
        signals.insert(if title_match || same_path {
            ContentSignal::ClosureCopyMatched
        } else {
            ContentSignal::ClosureCopyUnmatched
        });
    }

    if is_consent_page(&page, final_url) {
        signals.insert(ContentSignal::ConsentPage);
    }
    if is_auth_page(&page, final_url, title_match) {
        signals.insert(ContentSignal::AuthPage);
    }
    if anti_bot_header(signal_headers) || is_challenge_page(&page) {
        signals.insert(ContentSignal::AntiBot);
    }
    if is_access_denied(&page) {
        signals.insert(ContentSignal::AccessDenied);
    }
    if !title_match && is_generic_careers(&page, final_url, same_path) {
        signals.insert(ContentSignal::GenericCareers);
    }

    drop(page);
    signals
}

/// Normalized facts derived from one parse of the page.
struct PageFacts {
    title: String,
    og_title: String,
    og_site_name: String,
    h1: String,
    /// Title of the page's single JSON-LD `JobPosting`, when exactly one exists.
    ld_title: String,
    /// `hiringOrganization` names of that single `JobPosting`.
    ld_org_names: Vec<String>,
    visible_text: String,
    apply_enabled: bool,
    apply_disabled: bool,
    has_password_input: bool,
    /// Lowercased raw-HTML challenge markers present.
    has_challenge_marker: bool,
    job_detail_links: usize,
}

impl PageFacts {
    fn parse(html: &str, requested: &Url, final_url: &Url) -> Self {
        let doc = Html::parse_document(html);

        let title = first_text(&doc, "title");
        let h1 = first_text(&doc, "h1");
        let og_title = meta_content(&doc, "og:title");
        let og_site_name = meta_content(&doc, "og:site_name");

        let postings = json_ld_job_postings(&doc);
        let (ld_title, ld_org_names) = match postings.as_slice() {
            [only] => (
                only.get("title")
                    .and_then(Value::as_str)
                    .map(norm)
                    .unwrap_or_default(),
                hiring_org_names(only),
            ),
            _ => (String::new(), Vec::new()),
        };

        let (apply_enabled, apply_disabled) = apply_controls(&doc);
        let has_password_input = doc.select(&sel("input")).any(|el| {
            el.value()
                .attr("type")
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("password"))
        });

        let raw_lower = html.to_lowercase();
        let has_challenge_marker = ANTI_BOT_MARKERS.iter().any(|m| raw_lower.contains(m))
            || raw_lower.matches(CF_CHALLENGE_MARKER).count()
                > raw_lower.matches(CF_JSD_MARKER).count();

        Self {
            title,
            og_title,
            og_site_name,
            h1,
            ld_title,
            ld_org_names,
            visible_text: norm(&visible_text(&doc)),
            apply_enabled,
            apply_disabled,
            has_password_input,
            has_challenge_marker,
            job_detail_links: job_detail_link_count(&doc, requested, final_url),
        }
    }
}

fn sel(css: &str) -> Selector {
    Selector::parse(css).expect("static selector must parse")
}

fn first_text(doc: &Html, css: &str) -> String {
    doc.select(&sel(css))
        .next()
        .map(|el| norm(&el.text().collect::<String>()))
        .unwrap_or_default()
}

/// Content of the first `<meta property=key>` or `<meta name=key>`, normalized.
fn meta_content(doc: &Html, key: &str) -> String {
    doc.select(&sel("meta"))
        .find_map(|el| {
            let v = el.value();
            let matches = v
                .attr("property")
                .is_some_and(|p| p.trim().eq_ignore_ascii_case(key))
                || v.attr("name")
                    .is_some_and(|n| n.trim().eq_ignore_ascii_case(key));
            matches.then(|| v.attr("content")).flatten().map(norm)
        })
        .unwrap_or_default()
}

/// Every JSON-LD `JobPosting` object on the page.
fn json_ld_job_postings(doc: &Html) -> Vec<serde_json::Map<String, Value>> {
    let mut out = Vec::new();
    for el in doc.select(&sel("script[type]")) {
        let is_ld = el.value().attr("type").is_some_and(|t| {
            t.trim()
                .to_ascii_lowercase()
                .starts_with("application/ld+json")
        });
        if !is_ld {
            continue;
        }
        let source = el.text().collect::<String>();
        if let Ok(value) = serde_json::from_str::<Value>(&source) {
            collect_job_postings(&value, &mut out);
        }
    }
    out
}

fn collect_job_postings(value: &Value, out: &mut Vec<serde_json::Map<String, Value>>) {
    match value {
        Value::Object(obj) => {
            if obj.get("@type").is_some_and(is_job_posting_type) {
                out.push(obj.clone());
            } else {
                obj.values().for_each(|v| collect_job_postings(v, out));
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_job_postings(v, out)),
        _ => {}
    }
}

fn is_job_posting_type(value: &Value) -> bool {
    match value {
        Value::String(s) => {
            let s = s.trim().to_ascii_lowercase();
            s == "jobposting" || s.ends_with("/jobposting")
        }
        Value::Array(items) => items.iter().any(is_job_posting_type),
        _ => false,
    }
}

fn hiring_org_names(posting: &serde_json::Map<String, Value>) -> Vec<String> {
    fn name_of(v: &Value) -> Option<String> {
        match v {
            Value::String(s) => Some(norm(s)),
            Value::Object(o) => o.get("name").and_then(Value::as_str).map(norm),
            _ => None,
        }
    }
    let names = match posting.get("hiringOrganization") {
        Some(Value::Array(items)) => items.iter().filter_map(name_of).collect(),
        Some(v) => name_of(v).into_iter().collect(),
        None => Vec::new(),
    };
    names
        .into_iter()
        .filter(|n: &String| !n.is_empty())
        .collect()
}

/// Text of every text node outside `script`, `style`, `noscript`, and `template`.
fn visible_text(doc: &Html) -> String {
    let mut out = String::new();
    for node in doc.root_element().descendants() {
        let Some(text) = node.value().as_text() else {
            continue;
        };
        let hidden = node.ancestors().any(|a| {
            a.value()
                .as_element()
                .is_some_and(|e| matches!(e.name(), "script" | "style" | "noscript" | "template"))
        });
        if !hidden {
            out.push_str(text);
            out.push(' ');
        }
    }
    out
}

/// Returns `(any enabled apply control, any disabled apply control)`.
fn apply_controls(doc: &Html) -> (bool, bool) {
    let mut enabled = false;
    let mut disabled = false;
    let mut record = |el: ElementRef| {
        if is_hidden(el) {
            return;
        }
        if is_disabled(el) {
            disabled = true;
        } else {
            enabled = true;
        }
    };

    for el in doc.select(&sel("a, button, input")) {
        let v = el.value();
        let is_candidate = match v.name() {
            "input" => v
                .attr("type")
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("submit")),
            _ => true,
        };
        if !is_candidate {
            continue;
        }
        let text_hit = [
            Some(el.text().collect::<String>()),
            v.attr("aria-label").map(str::to_string),
            v.attr("value").map(str::to_string),
        ]
        .into_iter()
        .flatten()
        .any(|t| is_apply_label(&norm(&t)));
        let href_hit = v
            .attr("href")
            .is_some_and(|h| h.to_ascii_lowercase().contains("/apply"));
        if text_hit || href_hit {
            record(el);
        }
    }

    for el in doc.select(&sel(
        "#application_form, #application-form, form[action*=\"apply\"]",
    )) {
        record(el);
    }

    (enabled, disabled)
}

fn is_apply_label(normalized: &str) -> bool {
    contains_tokens(normalized, "apply")
        || contains_tokens(normalized, "submit application")
        || contains_tokens(normalized, "start application")
}

/// A control is disabled by `disabled`, `aria-disabled="true"`, a `disabled`
/// class token (also `*-disabled` / `*_disabled`), or a disabled ancestor
/// (`fieldset[disabled]` or `aria-disabled="true"`).
fn is_disabled(el: ElementRef) -> bool {
    let v = el.value();
    let own = v.attr("disabled").is_some()
        || aria_disabled(v.attr("aria-disabled"))
        || v.classes().any(|c| {
            let c = c.to_ascii_lowercase();
            c == "disabled" || c.ends_with("-disabled") || c.ends_with("_disabled")
        });
    own || el.ancestors().filter_map(ElementRef::wrap).any(|a| {
        let av = a.value();
        (av.name() == "fieldset" && av.attr("disabled").is_some())
            || aria_disabled(av.attr("aria-disabled"))
    })
}

fn aria_disabled(attr: Option<&str>) -> bool {
    attr.is_some_and(|a| a.trim().eq_ignore_ascii_case("true"))
}

/// Controls with `hidden` (or inside a `hidden` ancestor) or `type=hidden`
/// never count as apply controls.
fn is_hidden(el: ElementRef) -> bool {
    let v = el.value();
    v.attr("hidden").is_some()
        || v.attr("type")
            .is_some_and(|t| t.trim().eq_ignore_ascii_case("hidden"))
        || el
            .ancestors()
            .filter_map(ElementRef::wrap)
            .any(|a| a.value().attr("hidden").is_some())
}

fn title_matches(page: &PageFacts, title: &str) -> bool {
    let needle = norm(title);
    [&page.title, &page.og_title, &page.h1, &page.ld_title]
        .into_iter()
        .any(|field| contains_tokens(field, &needle))
}

fn company_matches(page: &PageFacts, company: &str, final_url: &Url) -> bool {
    let needle = norm(company);
    if needle.is_empty() {
        return false;
    }
    let site_name = (!is_ats_site_name(&page.og_site_name)).then_some(&page.og_site_name);
    let in_fields = page
        .ld_org_names
        .iter()
        .chain(site_name)
        .chain([&page.title, &page.h1])
        .any(|field| contains_tokens(field, &needle));
    if in_fields {
        return true;
    }
    let compact_company = compact(company);
    let host_label =
        registrable_label(final_url).filter(|l| !ATS_REGISTRABLE_LABELS.contains(&l.as_str()));
    let slug = ats_board_slug(final_url);
    [host_label, slug]
        .into_iter()
        .flatten()
        .any(|label| compact(&label) == compact_company)
}

fn is_ats_site_name(normalized: &str) -> bool {
    ATS_SITE_NAMES.iter().any(|ats| normalized.contains(ats))
}

/// Lowercased host labels without a trailing dot.
fn host_labels(url: &Url) -> Vec<String> {
    url.host_str()
        .map(|h| h.trim_end_matches('.').to_ascii_lowercase())
        .unwrap_or_default()
        .split('.')
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Approximate registrable label (`careers.acme.co.uk` → `acme`) without a
/// public-suffix list: two-part suffixes are recognized for 2-letter TLDs.
fn registrable_label(url: &Url) -> Option<String> {
    let labels = host_labels(url);
    let n = labels.len();
    if n < 2
        || url
            .host_str()
            .is_some_and(|h| h.parse::<std::net::IpAddr>().is_ok())
    {
        return None;
    }
    let two_part = n >= 3
        && labels[n - 1].len() == 2
        && SECOND_LEVEL_SUFFIX_LABELS.contains(&labels[n - 2].as_str());
    Some(labels[if two_part { n - 3 } else { n - 2 }].clone())
}

/// First path segment on a known ATS board host (`embed` is not a slug).
fn ats_board_slug(url: &Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    if !ATS_BOARD_HOSTS.contains(&host.as_str()) {
        return None;
    }
    segments(url).into_iter().next().filter(|s| s != "embed")
}

/// Lowercased, non-empty, percent-decoded path segments.
fn segments(url: &Url) -> Vec<String> {
    url.path_segments()
        .map(|segs| {
            segs.filter(|s| !s.is_empty())
                .map(|s| {
                    urlencoding::decode(s)
                        .map(|d| d.into_owned())
                        .unwrap_or_else(|_| s.to_string())
                        .to_lowercase()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Path used for "same path" comparison: trailing slashes are ignored.
fn path_key(url: &Url) -> &str {
    url.path().trim_end_matches('/')
}

fn has_closure_copy(visible_text: &str) -> bool {
    CLOSURE_PHRASES
        .iter()
        .any(|p| contains_tokens(visible_text, &norm(p)))
}

fn heading_matches_any(page: &PageFacts, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| {
        let p = norm(p);
        contains_tokens(&page.title, &p) || contains_tokens(&page.h1, &p)
    })
}

fn is_consent_page(page: &PageFacts, final_url: &Url) -> bool {
    let host_hit = host_labels(final_url).iter().any(|l| l == "consent");
    let path = final_url.path().to_ascii_lowercase();
    let path_hit = CONSENT_PATH_MARKERS.iter().any(|m| path.contains(m));
    host_hit || path_hit || heading_matches_any(page, CONSENT_PHRASES)
}

fn is_auth_page(page: &PageFacts, final_url: &Url, title_match: bool) -> bool {
    let host = final_url
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let host_hit = AUTH_HOST_PREFIXES.iter().any(|p| host.starts_with(p));
    let path_hit = segments(final_url).iter().any(|seg| {
        AUTH_SEGMENT_PREFIXES.iter().any(|p| {
            seg.strip_prefix(p)
                .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_alphabetic()))
        })
    });
    let title_hit = SIGN_IN_TITLE_PREFIXES
        .iter()
        .any(|p| starts_with_tokens(&page.title, &norm(p)));
    let password_hit = page.has_password_input && !title_match;
    host_hit || path_hit || title_hit || password_hit
}

fn is_challenge_page(page: &PageFacts) -> bool {
    page.has_challenge_marker
        || starts_with_tokens(&page.title, "just a moment")
        || contains_tokens(&page.title, "attention required")
        || page.title.contains("captcha")
}

fn is_access_denied(page: &PageFacts) -> bool {
    heading_matches_any(page, ACCESS_DENIED_PHRASES)
}

/// Generic careers page: a redirect to a listing-like URL (site root, a
/// `/careers`-style path, an ATS board root, or `?error=true`), or a page with
/// at least three distinct job-detail links. The caller only asks when the
/// title did not match.
fn is_generic_careers(page: &PageFacts, final_url: &Url, same_path: bool) -> bool {
    let error_redirect = final_url
        .query_pairs()
        .any(|(k, v)| k.eq_ignore_ascii_case("error") && v.eq_ignore_ascii_case("true"));
    let redirected_to_listing = !same_path && is_listing_like(final_url);
    error_redirect || redirected_to_listing || page.job_detail_links >= GENERIC_CAREERS_MIN_LINKS
}

fn is_listing_like(url: &Url) -> bool {
    let segs = segments(url);
    let Some(last) = segs.last() else { return true };
    let board_root = segs.len() == 1 && ats_board_slug(url).is_some();
    board_root || LISTING_LAST_SEGMENTS.contains(&last.as_str())
}

/// Stable key of a job-detail URL, or `None` when the URL is not one.
fn job_detail_key(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    if let Some((_, jid)) = url
        .query_pairs()
        .find(|(k, v)| k == "gh_jid" && !v.is_empty())
    {
        return Some(format!("gh_jid:{jid}"));
    }
    let segs = segments(url);
    match host.as_str() {
        "jobs.lever.co" | "jobs.eu.lever.co" | "jobs.ashbyhq.com" if segs.len() >= 2 => {
            return Some(format!("{host}/{}/{}", segs[0], segs[1]));
        }
        _ => {}
    }
    segs.windows(2)
        .position(|w| {
            JOB_DETAIL_PARENT_SEGMENTS.contains(&w[0].as_str())
                && !NON_POSTING_CHILD_SEGMENTS.contains(&w[1].as_str())
                && !JOB_DETAIL_PARENT_SEGMENTS.contains(&w[1].as_str())
        })
        .map(|i| format!("{host}/{}", segs[..=i + 1].join("/")))
}

/// Distinct job-detail links, excluding the posting itself.
fn job_detail_link_count(doc: &Html, requested: &Url, final_url: &Url) -> usize {
    let own: HashSet<String> = [requested, final_url]
        .into_iter()
        .filter_map(job_detail_key)
        .collect();
    doc.select(&sel("a[href]"))
        .filter_map(|a| a.value().attr("href"))
        .filter_map(|href| final_url.join(href.trim()).ok())
        .filter_map(|u| job_detail_key(&u))
        .filter(|k| !own.contains(k))
        .collect::<HashSet<_>>()
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ContentSignal as S;

    fn identity(title: &str, company: &str, url: &str) -> JobIdentity {
        JobIdentity {
            job_id: "job-1".into(),
            title: title.into(),
            company_name: company.into(),
            posting_url: url.into(),
        }
    }

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn extract(html: &str, id: &JobIdentity, requested: &str, final_url: &str) -> ContentSignals {
        extract_signals(html, id, &url(requested), &url(final_url), &[])
    }

    fn set(items: &[ContentSignal]) -> ContentSignals {
        items.iter().copied().collect()
    }

    const POSTING_URL: &str = "https://careers.acme.com/jobs/senior-engineer-123";

    fn posting_page() -> String {
        r#"<!doctype html><html><head>
            <title>Senior Engineer, Payments | Acme Careers</title>
            <meta property="og:title" content="Senior Engineer, Payments">
            <script type="application/ld+json">
              {"@context":"https://schema.org","@type":"JobPosting",
               "title":"Senior Engineer, Payments",
               "hiringOrganization":{"@type":"Organization","name":"Acme"}}
            </script>
          </head><body>
            <h1>Senior Engineer, Payments</h1>
            <p>Build payment systems.</p>
            <a class="btn" href="/jobs/senior-engineer-123/apply">Apply for this job</a>
          </body></html>"#
            .to_string()
    }

    #[test]
    fn norm_collapses_case_entities_punctuation_and_whitespace() {
        assert_eq!(
            norm("  Senior&nbsp;Engineer — PAYMENTS\n\t"),
            "senior engineer payments"
        );
        assert_eq!(norm("Acme &amp; Co."), "acme co");
        assert_eq!(
            norm("You don't have permission"),
            "you don t have permission"
        );
        assert_eq!(norm("...!!"), "");
    }

    #[test]
    fn contains_tokens_is_token_bounded() {
        assert!(contains_tokens("senior engineer payments", "engineer"));
        assert!(!contains_tokens("engineering jobs", "engineer"));
        assert!(!contains_tokens("anything", ""));
    }

    #[test]
    fn posting_page_with_json_ld_and_enabled_apply() {
        let id = identity("Senior Engineer, Payments", "Acme", POSTING_URL);
        let got = extract(&posting_page(), &id, POSTING_URL, POSTING_URL);
        assert_eq!(got, set(&[S::TitleMatch, S::CompanyMatch, S::ApplyEnabled]));
    }

    #[test]
    fn json_ld_alone_is_an_identity_field() {
        let html = r#"<html><head><title>Careers</title>
            <script type="application/ld+json">{"@graph":[{"@type":"JobPosting",
              "title":"Data Scientist","hiringOrganization":"Globex"}]}</script></head>
            <body><button type="button">Apply now</button></body></html>"#;
        let id = identity("data scientist", "GLOBEX", "https://example.org/p/9");
        let got = extract(
            html,
            &id,
            "https://example.org/p/9",
            "https://example.org/p/9",
        );
        assert_eq!(got, set(&[S::TitleMatch, S::CompanyMatch, S::ApplyEnabled]));
    }

    #[test]
    fn multiple_json_ld_postings_are_ignored() {
        let html = r#"<html><head><title>Open roles</title>
            <script type="application/ld+json">[
              {"@type":"JobPosting","title":"Data Scientist"},
              {"@type":"JobPosting","title":"Designer"}]</script></head><body></body></html>"#;
        let id = identity("Data Scientist", "Globex", "https://example.org/p/9");
        let got = extract(
            html,
            &id,
            "https://example.org/p/9",
            "https://example.org/p/9",
        );
        assert!(!got.contains(&S::TitleMatch));
    }

    #[test]
    fn body_text_does_not_produce_title_match() {
        let html = r#"<html><head><title>Jobs</title></head><body>
            <p>Senior Engineer, Payments</p></body></html>"#;
        let id = identity("Senior Engineer, Payments", "Acme", POSTING_URL);
        assert!(!extract(html, &id, POSTING_URL, POSTING_URL).contains(&S::TitleMatch));
    }

    #[test]
    fn disabled_apply_controls() {
        let variants = [
            r#"<button disabled>Apply</button>"#,
            r#"<button aria-disabled="true">Apply now</button>"#,
            r#"<a class="btn btn--disabled" href="/apply">Apply</a>"#,
            r##"<a class="btn disabled" href="#">Submit application</a>"##,
            r#"<fieldset disabled><input type="submit" value="Submit application"></fieldset>"#,
        ];
        let id = identity("Engineer", "Acme", POSTING_URL);
        for body in variants {
            let html =
                format!("<html><head><title>Engineer</title></head><body>{body}</body></html>");
            let got = extract(&html, &id, POSTING_URL, POSTING_URL);
            assert!(got.contains(&S::ApplyDisabled), "{body}: {got:?}");
            assert!(!got.contains(&S::ApplyEnabled), "{body}: {got:?}");
        }
    }

    #[test]
    fn hidden_and_non_apply_controls_do_not_count() {
        let html = r#"<html><body>
            <button hidden>Apply</button>
            <div hidden><a href="/x">Apply now</a></div>
            <input type="hidden" value="apply">
            <button>Applying tips</button>
            <a href="/jobs">Browse jobs</a></body></html>"#;
        let id = identity("Engineer", "Acme", POSTING_URL);
        let got = extract(html, &id, POSTING_URL, POSTING_URL);
        assert!(
            !got.contains(&S::ApplyEnabled) && !got.contains(&S::ApplyDisabled),
            "{got:?}"
        );
    }

    #[test]
    fn application_form_counts_as_enabled_apply() {
        let html =
            r#"<html><body><form id="application_form" action="/submit"></form></body></html>"#;
        let id = identity("Engineer", "Acme", POSTING_URL);
        assert!(extract(html, &id, POSTING_URL, POSTING_URL).contains(&S::ApplyEnabled));
    }

    #[test]
    fn closure_copy_matched_on_same_path() {
        let html = r#"<html><head><title>Acme Careers</title></head><body>
            <p>Sorry, this job is no longer available.</p></body></html>"#;
        let id = identity("Senior Engineer", "Acme", POSTING_URL);
        let with_slash = format!("{POSTING_URL}/");
        let got = extract(html, &id, POSTING_URL, &with_slash);
        assert!(got.contains(&S::ClosureCopyMatched), "{got:?}");
        assert!(!got.contains(&S::ClosureCopyUnmatched));
    }

    #[test]
    fn closure_copy_matched_by_title_after_redirect() {
        let html = r#"<html><head><title>Senior Engineer - Acme</title></head><body>
            <p>This position has been filled.</p></body></html>"#;
        let id = identity("Senior Engineer", "Acme", POSTING_URL);
        let got = extract(
            html,
            &id,
            POSTING_URL,
            "https://careers.acme.com/archive/123",
        );
        assert!(got.contains(&S::ClosureCopyMatched), "{got:?}");
    }

    #[test]
    fn closure_copy_unmatched_after_cross_path_redirect() {
        let html = r#"<html><head><title>Acme Careers</title></head><body>
            <div class="toast">That job is no longer open.</div>
            <a href="/jobs/1">One</a></body></html>"#;
        let id = identity("Senior Engineer", "Acme", POSTING_URL);
        let got = extract(html, &id, POSTING_URL, "https://careers.acme.com/careers");
        assert!(got.contains(&S::ClosureCopyUnmatched), "{got:?}");
        assert!(!got.contains(&S::ClosureCopyMatched));
    }

    #[test]
    fn closure_copy_in_script_is_ignored() {
        let html = r#"<html><head><title>Senior Engineer</title>
            <script>var msg = "job is closed";</script></head><body>Hi</body></html>"#;
        let id = identity("Senior Engineer", "Acme", POSTING_URL);
        let got = extract(html, &id, POSTING_URL, POSTING_URL);
        assert!(!got.contains(&S::ClosureCopyMatched) && !got.contains(&S::ClosureCopyUnmatched));
    }

    #[test]
    fn consent_wall_by_url_and_by_heading() {
        let id = identity("Senior Engineer", "Acme", POSTING_URL);
        let by_host = extract(
            "<html><body>Hi</body></html>",
            &id,
            POSTING_URL,
            "https://consent.example.com/ml?continue=x",
        );
        assert!(by_host.contains(&S::ConsentPage));
        let by_path = extract(
            "<html><body>Hi</body></html>",
            &id,
            POSTING_URL,
            "https://careers.acme.com/privacy-gateway",
        );
        assert!(by_path.contains(&S::ConsentPage));
        let by_heading = extract(
            "<html><head><title>Before you continue</title></head><body></body></html>",
            &id,
            POSTING_URL,
            POSTING_URL,
        );
        assert!(by_heading.contains(&S::ConsentPage));
    }

    #[test]
    fn cookie_banner_on_real_posting_is_not_consent() {
        let html = posting_page().replace(
            "<p>Build payment systems.</p>",
            r#"<div class="cookie-banner"><p>We use cookies. Manage cookies.</p><button>Accept</button></div>"#,
        );
        let id = identity("Senior Engineer, Payments", "Acme", POSTING_URL);
        assert!(!extract(&html, &id, POSTING_URL, POSTING_URL).contains(&S::ConsentPage));
    }

    #[test]
    fn sso_login_pages() {
        let id = identity("Senior Engineer", "Acme", POSTING_URL);
        let empty = "<html><body></body></html>";
        for final_url in [
            "https://acme.okta.com/login/login.htm?fromURI=x",
            "https://accounts.google.com/v3/signin/identifier",
            "https://careers.acme.com/oauth2/authorize",
            "https://careers.acme.com/sso/saml",
        ] {
            assert!(
                extract(empty, &id, POSTING_URL, final_url).contains(&S::AuthPage),
                "{final_url}"
            );
        }
        let by_title = "<html><head><title>Sign In | Acme SSO</title></head><body></body></html>";
        assert!(extract(by_title, &id, POSTING_URL, POSTING_URL).contains(&S::AuthPage));
        let by_password = r#"<html><head><title>Acme</title></head><body>
            <form><input type="PASSWORD" name="p"></form></body></html>"#;
        assert!(extract(by_password, &id, POSTING_URL, POSTING_URL).contains(&S::AuthPage));
        // Not an auth path: `/authors` only shares a prefix.
        assert!(!extract(
            empty,
            &id,
            POSTING_URL,
            "https://careers.acme.com/authors/1"
        )
        .contains(&S::AuthPage));
    }

    #[test]
    fn cloudflare_challenge_from_body_markers_and_title() {
        let id = identity("Senior Engineer", "Acme", POSTING_URL);
        let html = r#"<html><head><title>Just a moment...</title></head><body>
            <script src="/cdn-cgi/challenge-platform/h/b/orchestrate/chl_page/v1"></script>
            </body></html>"#;
        assert!(extract(html, &id, POSTING_URL, POSTING_URL).contains(&S::AntiBot));
        let marker_only = r#"<html><head><title>Acme</title></head><body>
            <script src="/cdn-cgi/challenge-platform/h/g/orchestrate/chl_page/v1"></script></body></html>"#;
        assert!(extract(marker_only, &id, POSTING_URL, POSTING_URL).contains(&S::AntiBot));
        let px = r#"<html><body><div id="px-captcha"></div></body></html>"#;
        assert!(extract(px, &id, POSTING_URL, POSTING_URL).contains(&S::AntiBot));
    }

    #[test]
    fn cloudflare_js_detection_script_alone_is_not_a_challenge() {
        let html = posting_page().replace(
            "</body>",
            r#"<script src="/cdn-cgi/challenge-platform/scripts/jsd/main.js"></script></body>"#,
        );
        let id = identity("Senior Engineer, Payments", "Acme", POSTING_URL);
        assert!(!extract(&html, &id, POSTING_URL, POSTING_URL).contains(&S::AntiBot));
    }

    #[test]
    fn cloudflare_challenge_from_header() {
        let id = identity("Senior Engineer", "Acme", POSTING_URL);
        let headers = vec![("CF-Mitigated".to_string(), " Challenge ".to_string())];
        let got = extract_signals("", &id, &url(POSTING_URL), &url(POSTING_URL), &headers);
        assert!(got.contains(&S::AntiBot));
        let other = vec![("cf-mitigated".to_string(), "none".to_string())];
        assert!(!anti_bot_header(&other));
    }

    #[test]
    fn access_denied_page() {
        let id = identity("Senior Engineer", "Acme", POSTING_URL);
        for html in [
            "<html><head><title>Access Denied</title></head><body></body></html>",
            "<html><body><h1>403 Forbidden</h1></body></html>",
            "<html><body><h1>You don’t have permission to access this page</h1></body></html>",
        ] {
            assert!(
                extract(html, &id, POSTING_URL, POSTING_URL).contains(&S::AccessDenied),
                "{html}"
            );
        }
    }

    #[test]
    fn generic_careers_index_by_links() {
        let html = r#"<html><head><title>Careers at Acme</title></head><body>
            <h1>Open positions</h1>
            <a href="/jobs/101">Designer</a>
            <a href="/jobs/102">Recruiter</a>
            <a href="/jobs/102#top">Recruiter again</a>
            <a href="https://boards.greenhouse.io/acme/jobs/555">Analyst</a>
            <a href="/jobs/search">Search</a>
            <button>Apply filters</button></body></html>"#;
        let id = identity("Senior Engineer", "Acme", POSTING_URL);
        let got = extract(html, &id, POSTING_URL, POSTING_URL);
        assert!(got.contains(&S::GenericCareers), "{got:?}");
        assert!(!got.contains(&S::TitleMatch));
    }

    #[test]
    fn generic_careers_after_redirect() {
        let id = identity(
            "Senior Engineer",
            "Acme",
            "https://boards.greenhouse.io/acme/jobs/127817",
        );
        let empty = "<html><head><title>Jobs at Acme</title></head><body></body></html>";
        let got = extract(
            empty,
            &id,
            "https://boards.greenhouse.io/acme/jobs/127817",
            "https://boards.greenhouse.io/acme?error=true",
        );
        assert!(got.contains(&S::GenericCareers), "{got:?}");
        // Board-slug company match still works on the ATS host.
        assert!(got.contains(&S::CompanyMatch));
        let got = extract(
            empty,
            &id,
            POSTING_URL,
            "https://careers.acme.com/en/careers/",
        );
        assert!(got.contains(&S::GenericCareers));
        // A redirect to another posting-like path is not a listing redirect.
        let got = extract(
            empty,
            &id,
            POSTING_URL,
            "https://careers.acme.com/jobs/senior-engineer-124",
        );
        assert!(!got.contains(&S::GenericCareers));
    }

    #[test]
    fn real_posting_with_similar_jobs_is_not_generic() {
        let html = posting_page().replace(
            "</body>",
            r#"<a href="/jobs/1">A</a><a href="/jobs/2">B</a><a href="/jobs/3">C</a></body>"#,
        );
        let id = identity("Senior Engineer, Payments", "Acme", POSTING_URL);
        assert!(!extract(&html, &id, POSTING_URL, POSTING_URL).contains(&S::GenericCareers));
    }

    #[test]
    fn company_match_from_host_label_and_not_ats_vendor() {
        let html = "<html><head><title>Role</title></head><body></body></html>";
        let id = identity("Engineer", "Acme Corp", "https://jobs.acme-corp.co.uk/r/1");
        let got = extract(
            html,
            &id,
            "https://jobs.acme-corp.co.uk/r/1",
            "https://jobs.acme-corp.co.uk/r/1",
        );
        assert!(got.contains(&S::CompanyMatch));
        let id = identity(
            "Engineer",
            "Greenhouse",
            "https://boards.greenhouse.io/other/jobs/1",
        );
        let got = extract(
            html,
            &id,
            "https://boards.greenhouse.io/other/jobs/1",
            "https://boards.greenhouse.io/other/jobs/1",
        );
        assert!(!got.contains(&S::CompanyMatch));
        // ATS og:site_name is ignored.
        let html = r#"<html><head><meta property="og:site_name" content="Lever"></head></html>"#;
        let id = identity("Engineer", "Lever", "https://example.com/j/1");
        assert!(!extract(
            html,
            &id,
            "https://example.com/j/1",
            "https://example.com/j/1"
        )
        .contains(&S::CompanyMatch));
    }

    #[test]
    fn signals_are_case_and_whitespace_invariant() {
        let base_id = identity("Senior Engineer, Payments", "Acme", POSTING_URL);
        let base = extract(&posting_page(), &base_id, POSTING_URL, POSTING_URL);

        let shouty = posting_page()
            .replace(
                "Senior Engineer, Payments",
                "  SENIOR   engineer,\n\tpayments  ",
            )
            .replace("Apply for this job", "APPLY   FOR\nthis JOB");
        let varied_id = identity("  senior ENGINEER ,   Payments ", "  ACME ", POSTING_URL);
        assert_eq!(extract(&shouty, &varied_id, POSTING_URL, POSTING_URL), base);

        let closed = |copy: &str| {
            format!("<html><head><title>Senior Engineer</title></head><body><p>{copy}</p></body></html>")
        };
        let id = identity("Senior Engineer", "Acme", POSTING_URL);
        let a = extract(
            &closed("This posting has expired."),
            &id,
            POSTING_URL,
            POSTING_URL,
        );
        let b = extract(
            &closed("THIS   POSTING\n HAS EXPIRED"),
            &id,
            POSTING_URL,
            POSTING_URL,
        );
        assert_eq!(a, b);
        assert!(a.contains(&S::ClosureCopyMatched));
    }

    #[test]
    fn empty_identity_never_matches() {
        let id = identity("  ", "", POSTING_URL);
        let got = extract(&posting_page(), &id, POSTING_URL, POSTING_URL);
        assert!(!got.contains(&S::TitleMatch) && !got.contains(&S::CompanyMatch));
    }
}
