//! Pure `classify(&CheckEvidence) -> Classification` decision procedure.
//!
//! The classifier only reports Active or Closed when the evidence is conclusive
//! and nothing casts doubt on it (design.md, "Classification"):
//!
//! ```text
//! positive  = provider == ListedOpen
//!           ∨ (http ∈ 2xx ∧ TitleMatch ∧ CompanyMatch ∧ ApplyEnabled)            // 6.1, 6.2
//! closed    = final http ∈ {404, 410}                                              // 6.3
//!           ∨ provider ∈ {ListedClosed, AbsentFromListing}                         // 6.4
//!           ∨ ClosureCopyMatched                                                   // 6.5
//! blockers  = transient failure (timeout, dns_timeout, connect, provider_temporary)
//!           ∪ http 429 / 5xx ∪ http 401 / 403
//!           ∪ {ConsentPage, AuthPage, AntiBot, AccessDenied}                       // 6.10, 6.11, 6.14
//!
//! if blockers ≠ ∅            → Unknown, reason names every blocker
//! else if positive ∧ closed  → Unknown, reason "conflicting evidence (<pos> vs <closed>)"  // 6.8
//! else if positive           → Active                                              // 6.6
//! else if closed             → Closed (persisted "inactive")                       // 6.7
//! else                       → Unknown ("no conclusive evidence", or the
//!                              non-transient failure / generic careers page)       // 6.9, 6.13
//! ```
//!
//! Precedence notes (all follow the design):
//! - `http_status` is the *final* response, so a redirect chain ending in
//!   404/410 is Closed, and a redirect ending in 200 is judged on the final page
//!   (Req 6.12).
//! - `ApplyDisabled` is informational only. It neither cancels `ApplyEnabled`
//!   nor counts as closed evidence; it is named in Unknown reasons.
//! - `GenericCareers` never yields Active by itself (page-based positive needs
//!   `TitleMatch`, which the extractor never sets together with
//!   `GenericCareers`). It is not closed evidence either, so a generic careers
//!   redirect alone is Unknown (Req 6.13). When the provider listing was
//!   retrieved and omits the posting, that absence is independent conclusive
//!   closed evidence and the result is Closed (Req 6.4, design behavior change 2).
//! - A `ListingUnavailable` provider signal is never conclusive and is not a
//!   blocker: the page is still evaluated. Only `failure == ProviderTemporary`
//!   is a blocker. The unavailable listing is named as a note in the reason.
//! - Non-transient failures (dns, blocked_destination, invalid_url, too_large,
//!   ...) are not blockers. Without other conclusive evidence they are Unknown
//!   with a `failed_<category>` code.
//!
//! The reason is built only from enum categories, HTTP statuses, the provider
//! name, and the (bounded) provider posting id, in a fixed order. It is
//! deterministic and at most [`MAX_REASON_BYTES`] (Req 6.15, 6.16).
//! [`classify`] normalizes its input first, so `classify(e) ==
//! classify(&e.normalized())` for every `e`.

use serde::Serialize;

use crate::jobs::posting_check::evidence::{
    is_transient_http_status, CheckEvidence, ContentSignal, FailureCategory, Provider,
    ProviderSignalKind,
};
use crate::runs::model::PostingState;
use crate::runs::progress::{bounded, MAX_CATEGORY_BYTES, MAX_REASON_BYTES};

/// Stable `reason_code` values. Codes derived from a category are
/// `transient_<failure>` (for example `transient_timeout`) and
/// `failed_<failure>` (for example `failed_dns`), using
/// [`FailureCategory::as_str`].
pub mod reason_code {
    /// Active: the provider listing contains the posting id.
    pub const LISTED_OPEN: &str = "listed_open";
    /// Active: title, company, and an enabled Apply control on a 2xx page.
    pub const PAGE_CONFIRMED_OPEN: &str = "page_confirmed_open";
    /// Closed: final response was HTTP 404 or 410.
    pub const HTTP_GONE: &str = "http_gone";
    /// Closed: the provider listing marks the posting closed.
    pub const LISTED_CLOSED: &str = "listed_closed";
    /// Closed: the posting id is absent from a retrieved provider listing.
    pub const ABSENT_FROM_LISTING: &str = "absent_from_listing";
    /// Closed: closure copy matched to the posting.
    pub const CLOSURE_COPY: &str = "closure_copy";
    /// Unknown: positive and closed evidence at the same time.
    pub const CONFLICT: &str = "conflict";
    /// Unknown: HTTP 429.
    pub const TRANSIENT_HTTP_429: &str = "transient_http_429";
    /// Unknown: HTTP 5xx.
    pub const TRANSIENT_HTTP_5XX: &str = "transient_http_5xx";
    /// Unknown: anti-bot challenge.
    pub const BLOCKED_ANTI_BOT: &str = "blocked_anti_bot";
    /// Unknown: access-denied page.
    pub const BLOCKED_ACCESS_DENIED: &str = "blocked_access_denied";
    /// Unknown: sign-in page.
    pub const BLOCKED_AUTH: &str = "blocked_auth";
    /// Unknown: consent page.
    pub const BLOCKED_CONSENT: &str = "blocked_consent";
    /// Unknown: HTTP 401.
    pub const BLOCKED_HTTP_401: &str = "blocked_http_401";
    /// Unknown: HTTP 403.
    pub const BLOCKED_HTTP_403: &str = "blocked_http_403";
    /// Unknown: redirected to or landed on a generic careers page.
    pub const GENERIC_CAREERS: &str = "generic_careers";
    /// Unknown: nothing conclusive either way.
    pub const NO_CONCLUSIVE_EVIDENCE: &str = "no_conclusive_evidence";
    /// Prefix for blockers caused by a transient failure category.
    pub const TRANSIENT_PREFIX: &str = "transient_";
    /// Prefix for Unknown results caused by a non-transient failure category.
    pub const FAILED_PREFIX: &str = "failed_";
}

/// Result of classifying one `CheckEvidence`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Classification {
    pub state: PostingState,
    /// Stable snake_case code, at most `MAX_CATEGORY_BYTES`.
    pub reason_code: String,
    /// Classification_Reason: `"<Open|Closed|Unknown>: <detail>"`, non-empty,
    /// at most `MAX_REASON_BYTES`.
    pub reason: String,
}

impl Classification {
    /// Value for `jobs.last_check_result`.
    pub fn last_check_result(&self) -> String {
        format_last_check_result(self.state, &self.reason)
    }
}

/// User-facing label that prefixes every Classification_Reason.
pub fn state_label(state: PostingState) -> &'static str {
    match state {
        PostingState::Active => "Open",
        PostingState::Inactive => "Closed",
        PostingState::Unknown => "Unknown",
    }
}

/// `jobs.last_check_result` value: `"{persisted_state}: {detail}"`, where
/// `detail` is the reason without its `Open:` / `Closed:` / `Unknown:` label,
/// for example `inactive: posting returned HTTP 404`. This keeps the existing
/// `<state>: <detail>` convention read by `checkResultNote` and CSV consumers.
pub fn format_last_check_result(state: PostingState, reason: &str) -> String {
    let trimmed = reason.trim();
    let detail = [
        PostingState::Active,
        PostingState::Inactive,
        PostingState::Unknown,
    ]
    .into_iter()
    .find_map(|s| {
        trimmed
            .strip_prefix(state_label(s))
            .and_then(|r| r.strip_prefix(':'))
    })
    .map(str::trim_start)
    .unwrap_or(trimmed);
    let detail = if detail.is_empty() {
        "no reason recorded"
    } else {
        detail
    };
    bounded(format!("{}: {detail}", state.as_str()), MAX_REASON_BYTES)
}

/// Classify evidence. Pure, total, and deterministic. The input is normalized
/// first, so raw and normalized evidence classify identically.
pub fn classify(evidence: &CheckEvidence) -> Classification {
    let e = evidence.normalized();
    let facts = Facts::gather(&e);

    let (state, code, decisive, notes) = if !facts.blockers.is_empty() {
        (
            PostingState::Unknown,
            facts.blockers[0].code(),
            facts.blockers.iter().map(Blocker::phrase).collect(),
            facts.listing_note().into_iter().collect(),
        )
    } else if !facts.positive.is_empty() && !facts.closed.is_empty() {
        let pos: Vec<_> = facts.positive.iter().map(Positive::short).collect();
        let closed: Vec<_> = facts.closed.iter().map(Closed::short).collect();
        let mentions = facts.closed.iter().any(Closed::mentions_status);
        (
            PostingState::Unknown,
            reason_code::CONFLICT.to_string(),
            vec![Phrase::new(
                format!(
                    "conflicting evidence ({} vs {})",
                    pos.join(" and "),
                    closed.join(" and ")
                ),
                mentions,
            )],
            facts.listing_note().into_iter().collect(),
        )
    } else if !facts.positive.is_empty() {
        (
            PostingState::Active,
            facts.positive[0].code().to_string(),
            facts.positive.iter().map(Positive::phrase).collect(),
            facts.listing_note().into_iter().collect(),
        )
    } else if !facts.closed.is_empty() {
        (
            PostingState::Inactive,
            facts.closed[0].code().to_string(),
            facts.closed.iter().map(Closed::phrase).collect(),
            facts.listing_note().into_iter().collect(),
        )
    } else {
        facts.inconclusive(&e)
    };

    Classification {
        state,
        reason_code: bounded(code, MAX_CATEGORY_BYTES),
        reason: render(state, &decisive, &notes, e.http_status),
    }
}

/// One clause of a reason. `mentions_status` is true when the clause already
/// names the final HTTP status, so the `(HTTP n)` suffix is not repeated.
struct Phrase {
    text: String,
    mentions_status: bool,
}

impl Phrase {
    fn new(text: impl Into<String>, mentions_status: bool) -> Self {
        Self {
            text: text.into(),
            mentions_status,
        }
    }
}

/// `"<Label>: <decisive; ...>[ (HTTP n)][; <note; ...>]"`, bounded.
fn render(state: PostingState, decisive: &[Phrase], notes: &[Phrase], http: Option<u16>) -> String {
    let mut out = format!("{}: ", state_label(state));
    out.push_str(
        &decisive
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join("; "),
    );
    if let Some(status) = http {
        if !decisive.iter().any(|p| p.mentions_status) {
            out.push_str(&format!(" (HTTP {status})"));
        }
    }
    for note in notes {
        out.push_str("; ");
        out.push_str(&note.text);
    }
    bounded(out, MAX_REASON_BYTES)
}

fn provider_name(p: Provider) -> &'static str {
    match p {
        Provider::Greenhouse => "Greenhouse",
        Provider::Lever => "Lever",
        Provider::Ashby => "Ashby",
    }
}

fn failure_phrase(f: FailureCategory) -> &'static str {
    match f {
        FailureCategory::Timeout => "timed out",
        FailureCategory::DnsTimeout => "DNS lookup timed out",
        FailureCategory::Dns => "DNS lookup failed",
        FailureCategory::Connect => "connection failed",
        FailureCategory::BlockedDestination => "destination blocked (private or local address)",
        FailureCategory::InvalidUrl => "invalid posting URL",
        FailureCategory::RedirectFailure => "redirect could not be followed",
        FailureCategory::TooManyRedirects => "too many redirects",
        FailureCategory::TooLarge => "response too large",
        FailureCategory::ProviderTemporary => "provider temporarily unavailable",
        FailureCategory::ProviderFailure => "provider request failed",
        FailureCategory::Internal => "internal error",
        FailureCategory::Persistence => "could not save the result",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Positive {
    ListedOpen {
        provider: Provider,
        posting_id: String,
    },
    PageConfirmed,
}

impl Positive {
    fn code(&self) -> &'static str {
        match self {
            Self::ListedOpen { .. } => reason_code::LISTED_OPEN,
            Self::PageConfirmed => reason_code::PAGE_CONFIRMED_OPEN,
        }
    }

    fn phrase(&self) -> Phrase {
        match self {
            Self::ListedOpen {
                provider,
                posting_id,
            } if !posting_id.is_empty() => Phrase::new(
                format!(
                    "listed on {} board (id {posting_id})",
                    provider_name(*provider)
                ),
                false,
            ),
            Self::ListedOpen { provider, .. } => Phrase::new(
                format!("listed on {} board", provider_name(*provider)),
                false,
            ),
            Self::PageConfirmed => Phrase::new(
                "page matches title and company with an enabled Apply control",
                false,
            ),
        }
    }

    fn short(&self) -> String {
        match self {
            Self::ListedOpen { provider, .. } => {
                format!("listed open on {} board", provider_name(*provider))
            }
            Self::PageConfirmed => "page matches with an enabled Apply control".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closed {
    HttpGone(u16),
    /// The provider listing marks the posting closed.
    Listed(Provider),
    AbsentFromListing(Provider),
    ClosureCopy,
}

impl Closed {
    fn code(self) -> &'static str {
        match self {
            Self::HttpGone(_) => reason_code::HTTP_GONE,
            Self::Listed(_) => reason_code::LISTED_CLOSED,
            Self::AbsentFromListing(_) => reason_code::ABSENT_FROM_LISTING,
            Self::ClosureCopy => reason_code::CLOSURE_COPY,
        }
    }

    fn mentions_status(&self) -> bool {
        matches!(self, Self::HttpGone(_))
    }

    fn phrase(&self) -> Phrase {
        match *self {
            Self::HttpGone(s) => Phrase::new(format!("posting returned HTTP {s}"), true),
            Self::Listed(p) => Phrase::new(
                format!("marked closed on {} board", provider_name(p)),
                false,
            ),
            Self::AbsentFromListing(p) => {
                Phrase::new(format!("absent from {} listing", provider_name(p)), false)
            }
            Self::ClosureCopy => Phrase::new("page shows closure copy", false),
        }
    }

    fn short(&self) -> String {
        match *self {
            Self::HttpGone(s) => format!("HTTP {s}"),
            Self::Listed(p) => format!("marked closed on {} board", provider_name(p)),
            Self::AbsentFromListing(p) => format!("absent from {} listing", provider_name(p)),
            Self::ClosureCopy => "closure copy".to_string(),
        }
    }
}

/// Declaration order is the precedence for `reason_code` and the order of
/// clauses in the reason: transient failures first, then page blockers, then
/// the access-control statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blocker {
    TransientFailure(FailureCategory),
    TransientHttp(u16),
    AntiBot,
    AccessDenied,
    AuthPage,
    ConsentPage,
    HttpAuth(u16),
}

impl Blocker {
    fn code(&self) -> String {
        match *self {
            Self::TransientFailure(f) => format!("{}{}", reason_code::TRANSIENT_PREFIX, f.as_str()),
            Self::TransientHttp(429) => reason_code::TRANSIENT_HTTP_429.to_string(),
            Self::TransientHttp(_) => reason_code::TRANSIENT_HTTP_5XX.to_string(),
            Self::AntiBot => reason_code::BLOCKED_ANTI_BOT.to_string(),
            Self::AccessDenied => reason_code::BLOCKED_ACCESS_DENIED.to_string(),
            Self::AuthPage => reason_code::BLOCKED_AUTH.to_string(),
            Self::ConsentPage => reason_code::BLOCKED_CONSENT.to_string(),
            Self::HttpAuth(401) => reason_code::BLOCKED_HTTP_401.to_string(),
            Self::HttpAuth(_) => reason_code::BLOCKED_HTTP_403.to_string(),
        }
    }

    fn phrase(&self) -> Phrase {
        match *self {
            Self::TransientFailure(f) => Phrase::new(failure_phrase(f), false),
            Self::TransientHttp(429) => Phrase::new("rate limited (HTTP 429)", true),
            Self::TransientHttp(s) => Phrase::new(format!("server error (HTTP {s})"), true),
            Self::AntiBot => Phrase::new("blocked by anti-bot challenge", false),
            Self::AccessDenied => Phrase::new("blocked by access-denied page", false),
            Self::AuthPage => Phrase::new("blocked by sign-in page", false),
            Self::ConsentPage => Phrase::new("blocked by consent page", false),
            Self::HttpAuth(401) => Phrase::new("authentication required (HTTP 401)", true),
            Self::HttpAuth(s) => Phrase::new(format!("access forbidden (HTTP {s})"), true),
        }
    }
}

/// Evidence reduced to the categories the decision table reads.
struct Facts {
    positive: Vec<Positive>,
    closed: Vec<Closed>,
    blockers: Vec<Blocker>,
    /// Provider listing that could not be retrieved or parsed.
    listing_unavailable: Option<(Provider, Option<u16>)>,
}

impl Facts {
    fn gather(e: &CheckEvidence) -> Self {
        let has = |s: ContentSignal| e.content.contains(&s);
        let http = e.http_status;

        let mut positive = Vec::new();
        let mut closed = Vec::new();
        let mut listing_unavailable = None;

        if let Some(p) = &e.provider {
            match p.signal {
                ProviderSignalKind::ListedOpen => positive.push(Positive::ListedOpen {
                    provider: p.provider,
                    posting_id: p.posting_id.clone(),
                }),
                ProviderSignalKind::ListedClosed => closed.push(Closed::Listed(p.provider)),
                ProviderSignalKind::AbsentFromListing => {
                    closed.push(Closed::AbsentFromListing(p.provider))
                }
                ProviderSignalKind::ListingUnavailable { http_status } => {
                    listing_unavailable = Some((p.provider, http_status))
                }
            }
        }

        // Req 6.2: identity in identity fields plus an enabled Apply control on
        // a successful response. `ApplyDisabled` is informational only.
        let page_ok = http.is_some_and(|s| (200..=299).contains(&s));
        if page_ok
            && has(ContentSignal::TitleMatch)
            && has(ContentSignal::CompanyMatch)
            && has(ContentSignal::ApplyEnabled)
        {
            positive.push(Positive::PageConfirmed);
        }

        // Closed order: HTTP, then provider (already pushed), then page copy.
        if let Some(s @ (404 | 410)) = http {
            closed.insert(0, Closed::HttpGone(s));
        }
        if has(ContentSignal::ClosureCopyMatched) {
            closed.push(Closed::ClosureCopy);
        }

        let mut blockers = Vec::new();
        if let Some(f) = e.failure.filter(|f| f.is_transient()) {
            blockers.push(Blocker::TransientFailure(f));
        }
        if let Some(s) = http.filter(|s| is_transient_http_status(*s)) {
            blockers.push(Blocker::TransientHttp(s));
        }
        for (signal, blocker) in [
            (ContentSignal::AntiBot, Blocker::AntiBot),
            (ContentSignal::AccessDenied, Blocker::AccessDenied),
            (ContentSignal::AuthPage, Blocker::AuthPage),
            (ContentSignal::ConsentPage, Blocker::ConsentPage),
        ] {
            if has(signal) {
                blockers.push(blocker);
            }
        }
        if let Some(s @ (401 | 403)) = http {
            blockers.push(Blocker::HttpAuth(s));
        }

        Self {
            positive,
            closed,
            blockers,
            listing_unavailable,
        }
    }

    fn listing_note(&self) -> Option<Phrase> {
        self.listing_unavailable.map(|(p, status)| {
            let text = match status {
                Some(s) => format!(
                    "{} listing unavailable (listing HTTP {s})",
                    provider_name(p)
                ),
                None => format!("{} listing unavailable", provider_name(p)),
            };
            Phrase::new(text, false)
        })
    }

    /// Unknown without blockers, positive, or closed evidence (Req 6.9, 6.13).
    fn inconclusive(&self, e: &CheckEvidence) -> (PostingState, String, Vec<Phrase>, Vec<Phrase>) {
        let has = |s: ContentSignal| e.content.contains(&s);
        let mut code = None;
        let mut decisive = Vec::new();

        // Transient failures were handled as blockers; only non-transient here.
        if let Some(f) = e.failure {
            code = Some(format!("{}{}", reason_code::FAILED_PREFIX, f.as_str()));
            decisive.push(Phrase::new(failure_phrase(f), false));
        }
        if has(ContentSignal::GenericCareers) {
            code.get_or_insert_with(|| reason_code::GENERIC_CAREERS.to_string());
            let text = if e.redirect_statuses.is_empty() {
                "landed on a generic careers page"
            } else {
                "redirected to a generic careers page"
            };
            decisive.push(Phrase::new(text, false));
        }
        if decisive.is_empty() {
            decisive.push(Phrase::new("no conclusive evidence", false));
        }

        let mut notes: Vec<Phrase> = self.listing_note().into_iter().collect();
        if has(ContentSignal::ClosureCopyUnmatched) {
            notes.push(Phrase::new(
                "closure copy not matched to this posting",
                false,
            ));
        }
        if has(ContentSignal::ApplyDisabled) {
            notes.push(Phrase::new("Apply control disabled", false));
        }

        (
            PostingState::Unknown,
            code.unwrap_or_else(|| reason_code::NO_CONCLUSIVE_EVIDENCE.to_string()),
            decisive,
            notes,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::posting_check::evidence::ProviderSignal;
    use crate::runs::progress::MAX_ID_BYTES;
    use ContentSignal as C;
    use PostingState as S;

    const URL: &str = "https://boards.greenhouse.io/acme/jobs/127817";

    fn ev(http: Option<u16>, content: &[ContentSignal]) -> CheckEvidence {
        let mut e = CheckEvidence::new(URL, "2026-01-02T03:04:05Z");
        e.http_status = http;
        e.content = content.iter().copied().collect();
        e
    }

    fn with_provider(
        mut e: CheckEvidence,
        provider: Provider,
        signal: ProviderSignalKind,
    ) -> CheckEvidence {
        e.provider = Some(ProviderSignal {
            provider,
            signal,
            posting_id: "127817".into(),
        });
        e
    }

    fn with_failure(mut e: CheckEvidence, f: FailureCategory) -> CheckEvidence {
        e.failure = Some(f);
        e
    }

    const PAGE_OPEN: &[ContentSignal] = &[C::TitleMatch, C::CompanyMatch, C::ApplyEnabled];

    fn assert_class(c: &Classification, state: PostingState, code: &str) {
        assert_eq!(c.state, state, "{c:?}");
        assert_eq!(c.reason_code, code, "{c:?}");
        assert!(
            c.reason.starts_with(&format!("{}: ", state_label(state))),
            "{c:?}"
        );
    }

    // --- Active ---

    #[test]
    fn provider_listed_open_is_active() {
        let c = classify(&with_provider(
            ev(None, &[]),
            Provider::Greenhouse,
            ProviderSignalKind::ListedOpen,
        ));
        assert_class(&c, S::Active, reason_code::LISTED_OPEN);
        assert_eq!(c.reason, "Open: listed on Greenhouse board (id 127817)");
    }

    #[test]
    fn page_with_identity_and_enabled_apply_is_active() {
        let c = classify(&ev(Some(200), PAGE_OPEN));
        assert_class(&c, S::Active, reason_code::PAGE_CONFIRMED_OPEN);
        assert_eq!(
            c.reason,
            "Open: page matches title and company with an enabled Apply control (HTTP 200)"
        );
    }

    #[test]
    fn page_positive_requires_every_identity_signal_and_a_2xx() {
        for missing in PAGE_OPEN {
            let signals: Vec<_> = PAGE_OPEN.iter().copied().filter(|s| s != missing).collect();
            let c = classify(&ev(Some(200), &signals));
            assert_class(&c, S::Unknown, reason_code::NO_CONCLUSIVE_EVIDENCE);
        }
        assert_class(
            &classify(&ev(None, PAGE_OPEN)),
            S::Unknown,
            reason_code::NO_CONCLUSIVE_EVIDENCE,
        );
        assert_class(
            &classify(&ev(Some(302), PAGE_OPEN)),
            S::Unknown,
            reason_code::NO_CONCLUSIVE_EVIDENCE,
        );
    }

    #[test]
    fn apply_disabled_is_informational_only() {
        // Enabled control present as well: still Active (design: informational).
        let mut signals = PAGE_OPEN.to_vec();
        signals.push(C::ApplyDisabled);
        assert_class(
            &classify(&ev(Some(200), &signals)),
            S::Active,
            reason_code::PAGE_CONFIRMED_OPEN,
        );

        // Only a disabled control: not positive, not closed.
        let c = classify(&ev(
            Some(200),
            &[C::TitleMatch, C::CompanyMatch, C::ApplyDisabled],
        ));
        assert_class(&c, S::Unknown, reason_code::NO_CONCLUSIVE_EVIDENCE);
        assert_eq!(
            c.reason,
            "Unknown: no conclusive evidence (HTTP 200); Apply control disabled"
        );
    }

    #[test]
    fn listed_open_and_page_confirmed_names_both() {
        let c = classify(&with_provider(
            ev(Some(200), PAGE_OPEN),
            Provider::Lever,
            ProviderSignalKind::ListedOpen,
        ));
        assert_class(&c, S::Active, reason_code::LISTED_OPEN);
        assert!(
            c.reason.contains("listed on Lever board")
                && c.reason.contains("enabled Apply control")
        );
    }

    #[test]
    fn listing_unavailable_is_not_a_blocker_and_is_noted() {
        let e = with_provider(
            ev(Some(200), PAGE_OPEN),
            Provider::Ashby,
            ProviderSignalKind::ListingUnavailable {
                http_status: Some(503),
            },
        );
        let c = classify(&e);
        assert_class(&c, S::Active, reason_code::PAGE_CONFIRMED_OPEN);
        assert_eq!(
            c.reason,
            "Open: page matches title and company with an enabled Apply control (HTTP 200); Ashby listing unavailable (listing HTTP 503)"
        );

        let e = with_provider(
            ev(Some(200), &[]),
            Provider::Ashby,
            ProviderSignalKind::ListingUnavailable { http_status: None },
        );
        let c = classify(&e);
        assert_class(&c, S::Unknown, reason_code::NO_CONCLUSIVE_EVIDENCE);
        assert!(c.reason.contains("Ashby listing unavailable"));
    }

    // --- Closed ---

    #[test]
    fn http_404_and_410_are_closed() {
        for s in [404, 410] {
            let c = classify(&ev(Some(s), &[]));
            assert_class(&c, S::Inactive, reason_code::HTTP_GONE);
            assert_eq!(c.reason, format!("Closed: posting returned HTTP {s}"));
        }
    }

    #[test]
    fn redirect_ending_in_404_is_decided_by_final_response() {
        let mut e = ev(Some(404), &[]);
        e.final_url = Some("https://acme.com/careers/missing".into());
        e.redirect_statuses = vec![301, 302];
        assert_class(&classify(&e), S::Inactive, reason_code::HTTP_GONE);
    }

    #[test]
    fn provider_closed_and_absent_are_closed() {
        let c = classify(&with_provider(
            ev(None, &[]),
            Provider::Lever,
            ProviderSignalKind::ListedClosed,
        ));
        assert_class(&c, S::Inactive, reason_code::LISTED_CLOSED);
        assert_eq!(c.reason, "Closed: marked closed on Lever board");

        let c = classify(&with_provider(
            ev(Some(200), &[]),
            Provider::Lever,
            ProviderSignalKind::AbsentFromListing,
        ));
        assert_class(&c, S::Inactive, reason_code::ABSENT_FROM_LISTING);
        assert_eq!(c.reason, "Closed: absent from Lever listing (HTTP 200)");
    }

    #[test]
    fn absent_from_listing_plus_generic_careers_redirect_is_closed() {
        // Greenhouse redirects closed postings to `?error=true`. Provider absence
        // is independent conclusive closed evidence (design precedence).
        let mut e = with_provider(
            ev(Some(200), &[C::CompanyMatch, C::GenericCareers]),
            Provider::Greenhouse,
            ProviderSignalKind::AbsentFromListing,
        );
        e.final_url = Some("https://boards.greenhouse.io/acme?error=true".into());
        e.redirect_statuses = vec![302];
        assert_class(&classify(&e), S::Inactive, reason_code::ABSENT_FROM_LISTING);
    }

    #[test]
    fn absent_from_listing_with_non_transient_page_failure_is_closed() {
        let e = with_failure(
            with_provider(
                ev(None, &[]),
                Provider::Ashby,
                ProviderSignalKind::AbsentFromListing,
            ),
            FailureCategory::Dns,
        );
        assert_class(&classify(&e), S::Inactive, reason_code::ABSENT_FROM_LISTING);
    }

    #[test]
    fn matched_closure_copy_is_closed_and_unmatched_is_not() {
        let c = classify(&ev(Some(200), &[C::ClosureCopyMatched, C::TitleMatch]));
        assert_class(&c, S::Inactive, reason_code::CLOSURE_COPY);
        assert_eq!(c.reason, "Closed: page shows closure copy (HTTP 200)");

        let c = classify(&ev(Some(200), &[C::ClosureCopyUnmatched]));
        assert_class(&c, S::Unknown, reason_code::NO_CONCLUSIVE_EVIDENCE);
        assert!(c.reason.contains("closure copy not matched"));
    }

    #[test]
    fn multiple_closed_categories_are_all_named() {
        let e = with_provider(
            ev(Some(410), &[C::ClosureCopyMatched]),
            Provider::Greenhouse,
            ProviderSignalKind::AbsentFromListing,
        );
        let c = classify(&e);
        assert_class(&c, S::Inactive, reason_code::HTTP_GONE);
        assert_eq!(
            c.reason,
            "Closed: posting returned HTTP 410; absent from Greenhouse listing; page shows closure copy"
        );
    }

    // --- Conflict ---

    #[test]
    fn provider_open_vs_closure_copy_is_conflict() {
        let e = with_provider(
            ev(Some(200), &[C::ClosureCopyMatched]),
            Provider::Greenhouse,
            ProviderSignalKind::ListedOpen,
        );
        let c = classify(&e);
        assert_class(&c, S::Unknown, reason_code::CONFLICT);
        assert_eq!(
            c.reason,
            "Unknown: conflicting evidence (listed open on Greenhouse board vs closure copy) (HTTP 200)"
        );
    }

    #[test]
    fn unlisted_but_live_page_is_conflict() {
        let e = with_provider(
            ev(Some(200), PAGE_OPEN),
            Provider::Ashby,
            ProviderSignalKind::AbsentFromListing,
        );
        let c = classify(&e);
        assert_class(&c, S::Unknown, reason_code::CONFLICT);
        assert!(c
            .reason
            .contains("page matches with an enabled Apply control"));
        assert!(c.reason.contains("absent from Ashby listing"));
    }

    #[test]
    fn provider_open_vs_http_404_is_conflict_naming_status_once() {
        let e = with_provider(
            ev(Some(404), &[]),
            Provider::Lever,
            ProviderSignalKind::ListedOpen,
        );
        let c = classify(&e);
        assert_class(&c, S::Unknown, reason_code::CONFLICT);
        assert_eq!(
            c.reason,
            "Unknown: conflicting evidence (listed open on Lever board vs HTTP 404)"
        );
    }

    // --- Blockers ---

    #[test]
    fn transient_failures_are_unknown_and_named() {
        for f in FailureCategory::ALL
            .into_iter()
            .filter(|f| f.is_transient())
        {
            let c = classify(&with_failure(ev(None, &[]), f));
            assert_class(&c, S::Unknown, &format!("transient_{}", f.as_str()));
            assert!(c.reason.contains(failure_phrase(f)), "{c:?}");
        }
        assert_eq!(
            classify(&with_failure(ev(None, &[]), FailureCategory::Timeout)).reason,
            "Unknown: timed out"
        );
    }

    #[test]
    fn transient_and_access_statuses_are_unknown() {
        let cases = [
            (
                429,
                reason_code::TRANSIENT_HTTP_429,
                "rate limited (HTTP 429)",
            ),
            (
                500,
                reason_code::TRANSIENT_HTTP_5XX,
                "server error (HTTP 500)",
            ),
            (
                503,
                reason_code::TRANSIENT_HTTP_5XX,
                "server error (HTTP 503)",
            ),
            (
                401,
                reason_code::BLOCKED_HTTP_401,
                "authentication required (HTTP 401)",
            ),
            (
                403,
                reason_code::BLOCKED_HTTP_403,
                "access forbidden (HTTP 403)",
            ),
        ];
        for (status, code, phrase) in cases {
            let c = classify(&ev(Some(status), &[]));
            assert_class(&c, S::Unknown, code);
            assert_eq!(c.reason, format!("Unknown: {phrase}"));
        }
    }

    #[test]
    fn page_blockers_are_unknown_even_on_http_200() {
        let cases = [
            (
                C::AntiBot,
                reason_code::BLOCKED_ANTI_BOT,
                "anti-bot challenge",
            ),
            (
                C::AccessDenied,
                reason_code::BLOCKED_ACCESS_DENIED,
                "access-denied page",
            ),
            (C::AuthPage, reason_code::BLOCKED_AUTH, "sign-in page"),
            (C::ConsentPage, reason_code::BLOCKED_CONSENT, "consent page"),
        ];
        for (signal, code, phrase) in cases {
            let c = classify(&ev(Some(200), &[signal]));
            assert_class(&c, S::Unknown, code);
            assert_eq!(c.reason, format!("Unknown: blocked by {phrase} (HTTP 200)"));
        }
    }

    #[test]
    fn blockers_dominate_positive_closed_and_conflict_evidence() {
        let blocked = [
            with_provider(
                ev(Some(200), &[C::AntiBot]),
                Provider::Greenhouse,
                ProviderSignalKind::ListedOpen,
            ),
            with_provider(
                ev(Some(503), &[]),
                Provider::Greenhouse,
                ProviderSignalKind::ListedOpen,
            ),
            ev(
                Some(200),
                &[
                    C::TitleMatch,
                    C::CompanyMatch,
                    C::ApplyEnabled,
                    C::ConsentPage,
                ],
            ),
            ev(Some(404), &[C::AuthPage]),
            ev(Some(200), &[C::ClosureCopyMatched, C::AccessDenied]),
            with_failure(
                with_provider(
                    ev(None, &[]),
                    Provider::Lever,
                    ProviderSignalKind::AbsentFromListing,
                ),
                FailureCategory::Timeout,
            ),
            with_failure(
                with_provider(
                    ev(Some(200), PAGE_OPEN),
                    Provider::Ashby,
                    ProviderSignalKind::AbsentFromListing,
                ),
                FailureCategory::ProviderTemporary,
            ),
        ];
        for e in blocked {
            let c = classify(&e);
            assert_eq!(c.state, S::Unknown, "{e:?} -> {c:?}");
            assert_ne!(c.reason_code, reason_code::CONFLICT, "{c:?}");
        }
    }

    #[test]
    fn every_blocker_is_named_and_primary_code_follows_precedence() {
        let e = with_failure(
            ev(Some(403), &[C::ConsentPage, C::AntiBot, C::AccessDenied]),
            FailureCategory::Connect,
        );
        let c = classify(&e);
        assert_class(&c, S::Unknown, "transient_connect");
        assert_eq!(
            c.reason,
            "Unknown: connection failed; blocked by anti-bot challenge; blocked by access-denied page; blocked by consent page; access forbidden (HTTP 403)"
        );

        let c = classify(&ev(Some(403), &[C::AntiBot]));
        assert_class(&c, S::Unknown, reason_code::BLOCKED_ANTI_BOT);
        assert_eq!(
            c.reason,
            "Unknown: blocked by anti-bot challenge; access forbidden (HTTP 403)"
        );

        let c = classify(&ev(Some(503), &[C::AntiBot]));
        assert_class(&c, S::Unknown, reason_code::TRANSIENT_HTTP_5XX);
        assert!(c.reason.contains("server error (HTTP 503)") && c.reason.contains("anti-bot"));
    }

    // --- Inconclusive ---

    #[test]
    fn non_transient_failures_are_unknown_with_their_category() {
        for f in FailureCategory::ALL
            .into_iter()
            .filter(|f| !f.is_transient())
        {
            let c = classify(&with_failure(ev(None, &[]), f));
            assert_class(&c, S::Unknown, &format!("failed_{}", f.as_str()));
            assert_eq!(c.reason, format!("Unknown: {}", failure_phrase(f)));
        }
    }

    #[test]
    fn generic_careers_page_is_unknown() {
        let mut e = ev(Some(200), &[C::CompanyMatch, C::GenericCareers]);
        e.final_url = Some("https://acme.com/careers".into());
        e.redirect_statuses = vec![301];
        let c = classify(&e);
        assert_class(&c, S::Unknown, reason_code::GENERIC_CAREERS);
        assert_eq!(
            c.reason,
            "Unknown: redirected to a generic careers page (HTTP 200)"
        );

        let c = classify(&ev(Some(200), &[C::GenericCareers]));
        assert_eq!(
            c.reason,
            "Unknown: landed on a generic careers page (HTTP 200)"
        );
    }

    #[test]
    fn no_evidence_is_unknown() {
        let c = classify(&ev(Some(200), &[]));
        assert_class(&c, S::Unknown, reason_code::NO_CONCLUSIVE_EVIDENCE);
        assert_eq!(c.reason, "Unknown: no conclusive evidence (HTTP 200)");
        let c = classify(&ev(None, &[]));
        assert_eq!(c.reason, "Unknown: no conclusive evidence");
    }

    // --- Determinism, normalization, bounds ---

    #[test]
    fn reasons_are_deterministic_bounded_and_normalization_invariant() {
        let providers: [Option<ProviderSignalKind>; 4] = [
            None,
            Some(ProviderSignalKind::ListedOpen),
            Some(ProviderSignalKind::AbsentFromListing),
            Some(ProviderSignalKind::ListingUnavailable {
                http_status: Some(502),
            }),
        ];
        let failures = [
            None,
            Some(FailureCategory::Timeout),
            Some(FailureCategory::TooLarge),
        ];
        for mask in 0u32..(1 << ContentSignal::ALL.len()) {
            let content: Vec<_> = ContentSignal::ALL
                .into_iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, s)| s)
                .collect();
            for http in [None, Some(200), Some(404), Some(403)] {
                for provider in providers {
                    for failure in failures {
                        let mut e = ev(http, &content);
                        if let Some(kind) = provider {
                            e = with_provider(e, Provider::Greenhouse, kind);
                        }
                        e.failure = failure;
                        let c = classify(&e);
                        assert_eq!(c, classify(&e));
                        assert_eq!(c, classify(&e.normalized()));
                        assert!(!c.reason.is_empty() && c.reason.len() <= MAX_REASON_BYTES);
                        assert!(
                            !c.reason_code.is_empty() && c.reason_code.len() <= MAX_CATEGORY_BYTES
                        );
                        assert!(c
                            .reason_code
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
                        assert!(c.reason.starts_with(&format!("{}: ", state_label(c.state))));
                        let blocked = failure == Some(FailureCategory::Timeout)
                            || http == Some(403)
                            || content.iter().any(|s| {
                                matches!(
                                    s,
                                    C::AntiBot | C::AuthPage | C::ConsentPage | C::AccessDenied
                                )
                            });
                        if blocked {
                            assert_eq!(c.state, S::Unknown, "{e:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn classification_uses_normalized_evidence() {
        // Invalid HTTP status 0 is dropped by normalization: no "(HTTP 0)".
        let c = classify(&ev(Some(0), PAGE_OPEN));
        assert_class(&c, S::Unknown, reason_code::NO_CONCLUSIVE_EVIDENCE);
        assert_eq!(c.reason, "Unknown: no conclusive evidence");
    }

    #[test]
    fn worst_case_reason_stays_within_bound() {
        let mut e = with_failure(
            ev(Some(403), &ContentSignal::ALL),
            FailureCategory::DnsTimeout,
        );
        e.provider = Some(ProviderSignal {
            provider: Provider::Greenhouse,
            signal: ProviderSignalKind::ListedOpen,
            posting_id: "9".repeat(MAX_ID_BYTES * 4),
        });
        let c = classify(&e);
        assert!(c.reason.len() <= MAX_REASON_BYTES);

        let e = with_provider(
            ev(None, &[]),
            Provider::Greenhouse,
            ProviderSignalKind::ListedOpen,
        );
        let mut long = e.clone();
        long.provider.as_mut().unwrap().posting_id = "é".repeat(400);
        let c = classify(&long);
        assert_eq!(c.state, S::Active);
        assert!(c.reason.len() <= MAX_REASON_BYTES);
        assert!(c.reason.is_char_boundary(c.reason.len()));
    }

    // --- last_check_result ---

    #[test]
    fn format_last_check_result_keeps_state_detail_convention() {
        assert_eq!(
            format_last_check_result(S::Inactive, "Closed: posting returned HTTP 404"),
            "inactive: posting returned HTTP 404"
        );
        assert_eq!(
            format_last_check_result(S::Unknown, "Unknown: timed out"),
            "unknown: timed out"
        );
        assert_eq!(
            format_last_check_result(S::Active, "Open: listed on Greenhouse board (id 1)"),
            "active: listed on Greenhouse board (id 1)"
        );
        // Unlabeled or empty reasons still produce `<state>: <detail>`.
        assert_eq!(
            format_last_check_result(S::Unknown, "HTTP 503"),
            "unknown: HTTP 503"
        );
        assert_eq!(
            format_last_check_result(S::Unknown, "  "),
            "unknown: no reason recorded"
        );
        assert_eq!(
            format_last_check_result(S::Unknown, "Unknown:"),
            "unknown: no reason recorded"
        );
    }

    #[test]
    fn last_check_result_matches_legacy_shape_for_every_branch() {
        let samples = [
            with_provider(
                ev(None, &[]),
                Provider::Greenhouse,
                ProviderSignalKind::ListedOpen,
            ),
            ev(Some(200), PAGE_OPEN),
            ev(Some(410), &[]),
            ev(Some(200), &[C::AntiBot]),
            with_failure(ev(None, &[]), FailureCategory::Timeout),
            ev(Some(200), &[C::GenericCareers]),
        ];
        for e in samples {
            let c = classify(&e);
            let stored = c.last_check_result();
            let (state, detail) = stored.split_once(": ").expect("<state>: <detail>");
            assert_eq!(state, c.state.as_str());
            assert!(!detail.is_empty());
            // The legacy `error:` prefix is never produced; state words are the
            // persisted values `checkResultNote`/CSV consumers already read.
            assert!(!stored.starts_with("error:"));
            assert!(stored.len() <= MAX_REASON_BYTES);
        }
    }
}
