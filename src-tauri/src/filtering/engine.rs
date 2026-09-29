//! Filter_Engine: the pure, deterministic matching authority.
//!
//! This module is the single source of truth for "does this criteria match this
//! job?". It operates purely on the job's title and location text with no I/O
//! and no mutation of its inputs.
//!
//! Task 3.1 establishes the text-processing foundation and the engine's public
//! view/result types:
//! - [`normalize`]: lowercase, trim, and collapse internal whitespace runs.
//! - [`tokenize`]: split normalized text into word-boundary tokens.
//! - [`JobView`]: a minimal read-only view of the fields the engine inspects.
//! - [`MatchResult`]: the inclusion decision plus a human-readable reason.
//!
//! `term_matches` (task 3.2) and `matches` (task 3.3) build on these.

use super::aliases::AliasTable;
use super::model::{FilterCriteria, MatchMode, RemoteMode};

/// A minimal read-only view of the fields the engine inspects.
///
/// Borrows the job's title and optional location so the engine can evaluate a
/// candidate without taking ownership of, or mutating, the underlying job.
pub struct JobView<'a> {
    /// The job title text.
    pub title: &'a str,
    /// The job location text, if any.
    pub location: Option<&'a str>,
}

/// The outcome of evaluating one `FilterCriteria` against one job.
pub struct MatchResult {
    /// Whether the job is included by the criteria.
    pub included: bool,
    /// A human-readable explanation of the decision, for debugging/telemetry
    /// (e.g. "excluded by title token 'contract'").
    pub reason: String,
}

/// Normalize text before matching (Req 1.4).
///
/// Converts all characters to lowercase, removes leading and trailing
/// whitespace, and replaces every run of one or more internal whitespace
/// characters with a single space character.
pub(crate) fn normalize(text: &str) -> String {
    // `split_whitespace` splits on any Unicode whitespace run and drops leading,
    // trailing, and repeated separators, so joining its pieces with a single
    // space collapses all internal whitespace runs and trims the ends.
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.to_lowercase()
}

/// Delimiter characters that separate tokens in addition to whitespace (Req 1.5):
/// comma, forward slash, opening parenthesis, closing parenthesis, hyphen, and
/// bullet.
const TOKEN_DELIMITERS: &[char] = &[',', '/', '(', ')', '-', '•'];

/// Tokenize normalized text into word-boundary tokens (Req 1.5, 1.7).
///
/// Splits on whitespace and on each of the punctuation delimiters comma,
/// forward slash, opening parenthesis, closing parenthesis, hyphen, and bullet,
/// excluding empty tokens from the result. Text containing zero tokens (empty
/// or delimiter-only input) yields an empty vector (Req 1.7).
///
/// The input is expected to already be normalized via [`normalize`]; the
/// function does not re-normalize, so callers should normalize first.
pub(crate) fn tokenize(normalized: &str) -> Vec<String> {
    normalized
        .split(|c: char| c.is_whitespace() || TOKEN_DELIMITERS.contains(&c))
        .filter(|tok| !tok.is_empty())
        .map(|tok| tok.to_string())
        .collect()
}

/// Decide whether a single term (`needle`) matches the given haystack under the
/// requested [`MatchMode`] (Req 1.1, 1.2, 1.3, 1.6, 1.7, 2.2, 2.3).
///
/// This is the shared matching primitive used for every include/exclude term on
/// both the title and location dimensions. Callers pass the haystack already
/// normalized and tokenized (`haystack_tokens` from [`tokenize`], `haystack_norm`
/// from [`normalize`]); the needle is normalized internally so criteria tokens
/// and job text compare case-insensitively and whitespace-invariantly.
///
/// Behavior:
/// - The `needle` is normalized via [`normalize`]. If it is empty after
///   normalization (empty or whitespace-only), no match is reported (Req 1.6, 2.3).
/// - `Word` mode (Req 1.1, 1.2, 1.7): the normalized needle is tokenized.
///   - A needle with zero tokens matches nothing.
///   - If the haystack has zero tokens, nothing matches (Req 1.7).
///   - A single-token needle matches iff it equals one of `haystack_tokens`
///     exactly (whole-token, case-insensitive) — no partial-token match (Req 1.1).
///   - A multi-token needle (2+ tokens) matches iff its token sequence appears
///     as an ordered, contiguous run within `haystack_tokens` (Req 1.2).
/// - `Substring` mode (Req 1.3): matches iff `haystack_norm` contains the
///   normalized needle as a contiguous substring, regardless of token boundaries.
pub(crate) fn term_matches(
    haystack_tokens: &[String],
    haystack_norm: &str,
    needle: &str,
    mode: MatchMode,
) -> bool {
    let needle_norm = normalize(needle);
    // Req 1.6 / 2.3: an empty-after-normalization term matches nothing.
    if needle_norm.is_empty() {
        return false;
    }

    match mode {
        MatchMode::Substring => {
            // Req 1.3: contiguous substring anywhere, ignoring token boundaries.
            haystack_norm.contains(&needle_norm)
        }
        MatchMode::Word => {
            let needle_tokens = tokenize(&needle_norm);
            // A needle that tokenizes to nothing cannot match.
            if needle_tokens.is_empty() {
                return false;
            }
            // Req 1.7: zero-token haystack yields no Word-mode match.
            if haystack_tokens.is_empty() {
                return false;
            }
            if needle_tokens.len() == 1 {
                // Req 1.1: single token matches a whole token only.
                haystack_tokens.iter().any(|tok| tok == &needle_tokens[0])
            } else {
                // Req 1.2: multi-word needle matches as an ordered contiguous run.
                if needle_tokens.len() > haystack_tokens.len() {
                    return false;
                }
                haystack_tokens
                    .windows(needle_tokens.len())
                    .any(|window| window == needle_tokens.as_slice())
            }
        }
    }
}

/// True when the location text carries no specific, non-remote place name.
///
/// Interpretation for Req 6.4's "no non-remote place present": after removing
/// recognized remote tokens, the location has no specific place when nothing
/// remains or the remainder is only a configured country name. Used to decide
/// whether a remote job may bypass the location-name include gate (e.g.
/// "Remote" or "Remote - US" should pass, but "Remote - New York" should still
/// be gated on the location name).
fn loc_has_no_place(loc_tokens: &[String], aliases: &AliasTable) -> bool {
    let remaining = loc_tokens
        .iter()
        .filter(|tok| !aliases.is_remote(tok))
        .map(String::as_str)
        .collect::<Vec<_>>();

    remaining.is_empty() || aliases.is_country_name(&remaining.join(" "))
}

/// Evaluate one [`FilterCriteria`] against one job (Req 2.x, 3.x, 5.x, 6.x).
///
/// This is the single, pure, deterministic matching authority: it performs no
/// I/O and never mutates `criteria`, `aliases`, or `job` (Req 11.1, 11.2). It
/// returns a [`MatchResult`] carrying the inclusion decision plus a
/// human-readable reason (Req 11.3).
///
/// Precedence and gating follow the design's `matches` algorithm:
/// 1. Match-all criteria short-circuit to included with reason "no criteria"
///    (Req 3.1, 3.3).
/// 2. Title: exclude wins over include (Req 2.1); include is any-of and an empty
///    include list satisfies the title dimension (Req 2.4, 2.6).
/// 3. Remote gate: `RemoteOnly` requires a remote location, `OnsiteOnly` rejects
///    remote locations, `Any` imposes nothing (Req 6.1, 6.2, 6.3).
/// 4. Location: the include set is expanded via the alias table and country
///    context, then deduplicated (Req 5.2, 5.3); location exclude wins (Req 2.1);
///    a remote job with no specific place bypasses the location-name gate when
///    remote is allowed (Req 6.4); otherwise include is any-of and an empty
///    include list satisfies the location dimension (Req 2.5, 2.7).
/// 5. A job not excluded by any gate is included only if both the title and
///    location dimensions are satisfied (Req 2.8).
pub fn matches(criteria: &FilterCriteria, aliases: &AliasTable, job: JobView<'_>) -> MatchResult {
    // --- Empty criteria includes everything (Req 3.1, 3.3). ---
    if criteria.is_match_all() {
        return MatchResult {
            included: true,
            reason: "no criteria".to_string(),
        };
    }

    let title_norm = normalize(job.title);
    let title_tokens = tokenize(&title_norm);
    let loc_norm = normalize(job.location.unwrap_or(""));
    let loc_tokens = tokenize(&loc_norm);

    // --- TITLE ---
    // Exclude wins first (Req 2.1).
    for term in &criteria.title.exclude {
        if term_matches(&title_tokens, &title_norm, term, criteria.title.match_mode) {
            return MatchResult {
                included: false,
                reason: format!("excluded by title '{term}'"),
            };
        }
    }
    // Include is any-of; empty include satisfies the title dimension (Req 2.4, 2.6).
    if !criteria.title.include.is_empty() {
        let matched =
            criteria.title.include.iter().any(|term| {
                term_matches(&title_tokens, &title_norm, term, criteria.title.match_mode)
            });
        if !matched {
            return MatchResult {
                included: false,
                reason: "no title include matched".to_string(),
            };
        }
    }

    // --- REMOTE (Req 6.1, 6.2, 6.3) ---
    let is_remote = aliases.is_remote(&loc_norm);
    if criteria.remote == RemoteMode::RemoteOnly && !is_remote {
        return MatchResult {
            included: false,
            reason: "not remote".to_string(),
        };
    }
    if criteria.remote == RemoteMode::OnsiteOnly && is_remote {
        return MatchResult {
            included: false,
            reason: "remote excluded".to_string(),
        };
    }

    // --- LOCATION ---
    // Build the effective include set by expanding aliases + country (Req 5.2),
    // then deduplicate preserving first-seen order (Req 5.3).
    let mut expanded_include: Vec<String> = Vec::new();
    let country = criteria.location.country.as_deref();
    for tok in &criteria.location.include {
        for expanded in aliases.expand_location(tok, country) {
            if !expanded_include.contains(&expanded) {
                expanded_include.push(expanded);
            }
        }
    }
    if let Some(c) = country {
        if !c.trim().is_empty() {
            for expanded in aliases.expand_country(c) {
                if !expanded_include.contains(&expanded) {
                    expanded_include.push(expanded);
                }
            }
        }
    }

    // Location exclude wins (Req 2.1).
    for term in &criteria.location.exclude {
        if term_matches(&loc_tokens, &loc_norm, term, criteria.location.match_mode) {
            return MatchResult {
                included: false,
                reason: format!("excluded by location '{term}'"),
            };
        }
    }

    if !expanded_include.is_empty() {
        // Remote-bypasses-location-name rule (Req 6.4): a remote job with no
        // specific non-remote place present passes the location gate when remote
        // is allowed, so rows like "Remote - US" are not dropped by a city list.
        if criteria.remote != RemoteMode::OnsiteOnly
            && is_remote
            && loc_has_no_place(&loc_tokens, aliases)
        {
            return MatchResult {
                included: true,
                reason: "remote passes location gate".to_string(),
            };
        }
        // Location include is any-of (Req 2.5).
        let matched = expanded_include
            .iter()
            .any(|term| term_matches(&loc_tokens, &loc_norm, term, criteria.location.match_mode));
        if !matched {
            return MatchResult {
                included: false,
                reason: "no location include matched".to_string(),
            };
        }
    }

    // Not excluded and both dimensions satisfied (Req 2.8).
    MatchResult {
        included: true,
        reason: "all criteria satisfied".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lowercases_trims_and_collapses_whitespace() {
        assert_eq!(normalize("  Hello   World  "), "hello world");
        assert_eq!(normalize("San Jose,   CA"), "san jose, ca");
        assert_eq!(
            normalize("Senior\tEngineer\n(Remote)"),
            "senior engineer (remote)"
        );
        assert_eq!(normalize("UPPER"), "upper");
    }

    #[test]
    fn normalize_empty_and_whitespace_only() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("     "), "");
        assert_eq!(normalize("\t\n  \r"), "");
    }

    #[test]
    fn normalize_collapses_whitespace_but_preserves_delimiter_spacing() {
        // `normalize` (Req 1.4) only collapses whitespace runs; it does not
        // rewrite spacing around punctuation delimiters. So a space before a
        // comma is preserved at the normalized-string level.
        assert_eq!(normalize("San Jose, CA"), "san jose, ca");
        assert_eq!(normalize(" San Jose ,  CA "), "san jose , ca");
    }

    #[test]
    fn tokenize_gives_whitespace_and_delimiter_spacing_invariance() {
        // Locations differing only in surrounding whitespace or delimiter
        // spacing tokenize identically — the level at which match results are
        // compared downstream (supports Req 4.3).
        let a = tokenize(&normalize("San Jose, CA"));
        let b = tokenize(&normalize(" San Jose ,  CA "));
        let c = tokenize(&normalize("San Jose,CA"));
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(
            a,
            vec!["san".to_string(), "jose".to_string(), "ca".to_string()]
        );
    }

    #[test]
    fn tokenize_splits_on_whitespace() {
        assert_eq!(
            tokenize("senior software engineer"),
            vec![
                "senior".to_string(),
                "software".to_string(),
                "engineer".to_string(),
            ]
        );
    }

    #[test]
    fn tokenize_splits_on_each_delimiter() {
        assert_eq!(
            tokenize("san jose, ca / remote (hybrid) - full-time • urgent"),
            vec![
                "san".to_string(),
                "jose".to_string(),
                "ca".to_string(),
                "remote".to_string(),
                "hybrid".to_string(),
                "full".to_string(),
                "time".to_string(),
                "urgent".to_string(),
            ]
        );
    }

    #[test]
    fn tokenize_drops_empty_tokens_between_delimiters() {
        // Adjacent delimiters and surrounding delimiters must not yield empties.
        assert_eq!(tokenize("a,,b"), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(tokenize("(remote)"), vec!["remote".to_string()]);
        assert_eq!(tokenize("- qa -"), vec!["qa".to_string()]);
    }

    #[test]
    fn tokenize_zero_token_cases_yield_empty_vec() {
        // Req 1.7: empty or delimiter-only text produces zero tokens.
        assert!(tokenize("").is_empty());
        assert!(tokenize(",,,").is_empty());
        assert!(tokenize(" / ( ) - • ").is_empty());
    }

    /// Helper: normalize + tokenize a raw haystack for `term_matches` tests.
    fn hay(text: &str) -> (Vec<String>, String) {
        let norm = normalize(text);
        (tokenize(&norm), norm)
    }

    #[test]
    fn word_single_token_matches_whole_token_only() {
        // Req 1.1: "qa" must NOT match the substring in "quality assurance
        // engineer" (no standalone "qa" token), but MUST match "qa engineer".
        let (no_tokens, no_norm) = hay("Quality Assurance Engineer");
        assert!(!term_matches(&no_tokens, &no_norm, "qa", MatchMode::Word));

        let (yes_tokens, yes_norm) = hay("QA Engineer");
        assert!(term_matches(&yes_tokens, &yes_norm, "qa", MatchMode::Word));
    }

    #[test]
    fn word_single_token_is_case_insensitive() {
        let (tokens, norm) = hay("Senior QA Engineer");
        assert!(term_matches(&tokens, &norm, "QA", MatchMode::Word));
        assert!(term_matches(&tokens, &norm, "engineer", MatchMode::Word));
    }

    #[test]
    fn word_multi_token_matches_ordered_contiguous_run() {
        let (tokens, norm) = hay("Senior Software Engineer, Backend");
        // Contiguous ordered run present.
        assert!(term_matches(
            &tokens,
            &norm,
            "software engineer",
            MatchMode::Word
        ));
        // Tokens present but not contiguous.
        assert!(!term_matches(
            &tokens,
            &norm,
            "senior engineer",
            MatchMode::Word
        ));
        // Right tokens, wrong order.
        assert!(!term_matches(
            &tokens,
            &norm,
            "engineer software",
            MatchMode::Word
        ));
    }

    #[test]
    fn word_multi_token_needle_longer_than_haystack_no_match() {
        let (tokens, norm) = hay("Software Engineer");
        assert!(!term_matches(
            &tokens,
            &norm,
            "senior software engineer manager",
            MatchMode::Word
        ));
    }

    #[test]
    fn substring_matches_regardless_of_token_boundaries() {
        // Req 1.3: substring match ignores boundaries — "qa" matches inside
        // "quality assurance" spelled contiguously would be false here, but a
        // genuine contiguous substring like "ware eng" matches.
        let (tokens, norm) = hay("Senior Software Engineer");
        assert!(term_matches(
            &tokens,
            &norm,
            "ware eng",
            MatchMode::Substring
        ));
        assert!(term_matches(
            &tokens,
            &norm,
            "software",
            MatchMode::Substring
        ));
        // Not present as a contiguous substring.
        assert!(!term_matches(
            &tokens,
            &norm,
            "backend",
            MatchMode::Substring
        ));
    }

    #[test]
    fn empty_needle_never_matches() {
        // Req 1.6 / 2.3: empty-after-normalization term reports no match.
        let (tokens, norm) = hay("QA Engineer");
        assert!(!term_matches(&tokens, &norm, "", MatchMode::Word));
        assert!(!term_matches(&tokens, &norm, "   ", MatchMode::Word));
        assert!(!term_matches(&tokens, &norm, "\t\n", MatchMode::Substring));
    }

    #[test]
    fn word_zero_token_haystack_no_match() {
        // Req 1.7: a haystack with zero tokens matches no Word-mode term.
        let (tokens, norm) = hay("");
        assert!(tokens.is_empty());
        assert!(!term_matches(&tokens, &norm, "qa", MatchMode::Word));
        assert!(!term_matches(
            &tokens,
            &norm,
            "software engineer",
            MatchMode::Word
        ));
    }

    // --- matches() tests ---

    use super::super::model::{FilterCriteria, LocationCriteria, RemoteMode, TitleCriteria};

    fn view<'a>(title: &'a str, location: Option<&'a str>) -> JobView<'a> {
        JobView { title, location }
    }

    fn title_include(terms: &[&str]) -> FilterCriteria {
        FilterCriteria {
            title: TitleCriteria {
                include: terms.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            ..FilterCriteria::match_all()
        }
    }

    #[test]
    fn match_all_short_circuits_to_included() {
        let aliases = AliasTable::default_seed();
        let result = matches(
            &FilterCriteria::match_all(),
            &aliases,
            view("Anything at all", Some("Anywhere city")),
        );
        assert!(result.included);
        assert_eq!(result.reason, "no criteria");
    }

    #[test]
    fn title_exclude_beats_include() {
        let aliases = AliasTable::default_seed();
        let criteria = FilterCriteria {
            title: TitleCriteria {
                include: vec!["engineer".to_string()],
                exclude: vec!["contract".to_string()],
                ..Default::default()
            },
            ..FilterCriteria::match_all()
        };
        // Both include and exclude match; exclude wins (Req 2.1).
        let result = matches(&criteria, &aliases, view("Contract Engineer", None));
        assert!(!result.included);
        assert_eq!(result.reason, "excluded by title 'contract'");
    }

    #[test]
    fn title_include_is_any_of() {
        let aliases = AliasTable::default_seed();
        let criteria = title_include(&["backend", "frontend"]);
        // Matches one of the include terms.
        assert!(matches(&criteria, &aliases, view("Senior Frontend Engineer", None)).included);
        // Matches none -> excluded.
        let miss = matches(&criteria, &aliases, view("Data Scientist", None));
        assert!(!miss.included);
        assert_eq!(miss.reason, "no title include matched");
    }

    #[test]
    fn empty_title_include_satisfies_dimension() {
        let aliases = AliasTable::default_seed();
        // Only a title exclude present; any non-excluded title is included.
        let criteria = FilterCriteria {
            title: TitleCriteria {
                exclude: vec!["intern".to_string()],
                ..Default::default()
            },
            ..FilterCriteria::match_all()
        };
        assert!(matches(&criteria, &aliases, view("Staff Engineer", None)).included);
        assert!(!matches(&criteria, &aliases, view("Engineering Intern", None)).included);
    }

    #[test]
    fn remote_only_gate() {
        let aliases = AliasTable::default_seed();
        let criteria = FilterCriteria {
            remote: RemoteMode::RemoteOnly,
            ..FilterCriteria::match_all()
        };
        assert!(matches(&criteria, &aliases, view("Engineer", Some("Remote - US"))).included);
        let onsite = matches(&criteria, &aliases, view("Engineer", Some("San Jose, CA")));
        assert!(!onsite.included);
        assert_eq!(onsite.reason, "not remote");
    }

    #[test]
    fn onsite_only_gate() {
        let aliases = AliasTable::default_seed();
        let criteria = FilterCriteria {
            remote: RemoteMode::OnsiteOnly,
            ..FilterCriteria::match_all()
        };
        assert!(matches(&criteria, &aliases, view("Engineer", Some("San Jose, CA"))).included);
        let remote = matches(&criteria, &aliases, view("Engineer", Some("Remote")));
        assert!(!remote.included);
        assert_eq!(remote.reason, "remote excluded");
    }

    #[test]
    fn location_include_uses_alias_expansion() {
        let aliases = AliasTable::default_seed();
        // "bay area" expands to member cities incl. "san jose".
        let criteria = FilterCriteria {
            location: LocationCriteria {
                include: vec!["bay area".to_string()],
                ..Default::default()
            },
            ..FilterCriteria::match_all()
        };
        assert!(matches(&criteria, &aliases, view("Engineer", Some("San Jose, CA"))).included);
        let miss = matches(&criteria, &aliases, view("Engineer", Some("Austin, TX")));
        assert!(!miss.included);
        assert_eq!(miss.reason, "no location include matched");
    }

    #[test]
    fn location_exclude_wins() {
        let aliases = AliasTable::default_seed();
        let criteria = FilterCriteria {
            location: LocationCriteria {
                include: vec!["bay area".to_string()],
                exclude: vec!["oakland".to_string()],
                ..Default::default()
            },
            ..FilterCriteria::match_all()
        };
        // Oakland is a bay-area member but is explicitly excluded (Req 2.1).
        let excluded = matches(&criteria, &aliases, view("Engineer", Some("Oakland, CA")));
        assert!(!excluded.included);
        assert_eq!(excluded.reason, "excluded by location 'oakland'");
    }

    #[test]
    fn remote_bypasses_specific_location_include() {
        let aliases = AliasTable::default_seed();
        // Location include is a specific city, remote mode Any.
        let criteria = FilterCriteria {
            location: LocationCriteria {
                include: vec!["san francisco".to_string()],
                ..Default::default()
            },
            ..FilterCriteria::match_all()
        };
        // Country qualifiers are not specific place names, so these remote
        // locations bypass the city include gate (Req 6.4).
        for location in ["Remote - US", "Remote - USA", "Remote - United States"] {
            let bypass = matches(&criteria, &aliases, view("Engineer", Some(location)));
            assert!(bypass.included, "expected {location:?} to bypass");
            assert_eq!(bypass.reason, "remote passes location gate");
        }

        // A remote job that also names a non-matching place is still gated.
        let gated = matches(
            &criteria,
            &aliases,
            view("Engineer", Some("Remote - New York")),
        );
        assert!(!gated.included);
        assert_eq!(gated.reason, "no location include matched");
    }

    // =====================================================================
    // Property-based and additional unit tests (tasks 3.4 - 3.12).
    //
    // These use the `proptest` crate for the universal correctness
    // properties from the design (Properties 1-12) and plain `#[test]`s for
    // the concrete engine edge cases. Each is annotated with its design
    // Property number and the requirement clauses it validates.
    // =====================================================================

    use proptest::prelude::*;

    /// Bounded free-text strategy for job title/location fields: lowercase
    /// letters and spaces keep generation cheap and the input space realistic
    /// for word/token matching, while staying small for fast shrinking.
    fn text_strategy() -> impl Strategy<Value = String> {
        "[a-z ]{0,40}"
    }

    /// A small set of word-like tokens (letters only), used to build include /
    /// exclude lists whose membership in a title we control explicitly.
    fn token_strategy() -> impl Strategy<Value = String> {
        "[a-z]{1,8}"
    }

    // --- 3.4 Property 1: Empty criteria matches all (Validates Requirements 3.1) ---

    proptest! {
        #[test]
        fn prop_empty_criteria_matches_all(
            title in text_strategy(),
            loc in text_strategy(),
            has_loc in any::<bool>(),
        ) {
            let aliases = AliasTable::default_seed();
            let location = if has_loc { Some(loc.as_str()) } else { None };
            let result = matches(
                &FilterCriteria::match_all(),
                &aliases,
                view(&title, location),
            );
            // Property 1: match-all includes every possible job.
            prop_assert!(result.included);
        }
    }

    // --- 3.5 Property 2: Exclude precedence (Validates Requirements 2.1) ---

    proptest! {
        #[test]
        fn prop_title_exclude_beats_include(
            exclude_tok in token_strategy(),
            include_toks in prop::collection::vec(token_strategy(), 0..4),
            prefix in "[a-z ]{0,10}",
            suffix in "[a-z ]{0,10}",
        ) {
            let aliases = AliasTable::default_seed();
            // Build a title that genuinely contains the exclude token as a whole
            // word, surrounded by arbitrary (space-separated) text.
            let title = format!("{prefix} {exclude_tok} {suffix}");
            let criteria = FilterCriteria {
                title: TitleCriteria {
                    include: include_toks,
                    exclude: vec![exclude_tok.clone()],
                    ..Default::default()
                },
                ..FilterCriteria::match_all()
            };
            let result = matches(&criteria, &aliases, view(&title, None));
            // Property 2: exclude always wins regardless of include contents.
            prop_assert!(!result.included);
            prop_assert_eq!(result.reason, format!("excluded by title '{exclude_tok}'"));
        }
    }

    proptest! {
        #[test]
        fn prop_location_exclude_beats_include(
            exclude_tok in token_strategy(),
            include_toks in prop::collection::vec(token_strategy(), 0..4),
            prefix in "[a-z ]{0,10}",
            suffix in "[a-z ]{0,10}",
        ) {
            let aliases = AliasTable::default_seed();
            let location = format!("{prefix} {exclude_tok} {suffix}");
            let criteria = FilterCriteria {
                location: LocationCriteria {
                    include: include_toks,
                    exclude: vec![exclude_tok.clone()],
                    ..Default::default()
                },
                ..FilterCriteria::match_all()
            };
            let result = matches(&criteria, &aliases, view("Engineer", Some(&location)));
            // Property 2: location exclude wins regardless of include contents.
            prop_assert!(!result.included);
            prop_assert_eq!(result.reason, format!("excluded by location '{exclude_tok}'"));
        }
    }

    // --- 3.6 Property 3: Include is any-of / disjunctive (Validates Requirements 2.4, 2.5) ---

    proptest! {
        #[test]
        fn prop_title_include_is_any_of(
            include_toks in prop::collection::vec(token_strategy(), 1..5),
            pick in 0usize..5,
            other in "[a-z]{9,14}",
        ) {
            let aliases = AliasTable::default_seed();
            // Deduplicate so a token we later treat as "absent" is genuinely absent.
            let mut includes = include_toks.clone();
            includes.sort();
            includes.dedup();
            let chosen = &includes[pick % includes.len()];

            // A title containing at least one include token is included on the
            // title dimension (location left as match-all so only title decides).
            let hit_title = format!("senior {chosen} engineer");
            let criteria = FilterCriteria {
                title: TitleCriteria {
                    include: includes.clone(),
                    ..Default::default()
                },
                ..FilterCriteria::match_all()
            };
            let hit = matches(&criteria, &aliases, view(&hit_title, None));
            prop_assert!(hit.included, "expected include hit for {:?} in {:?}", chosen, includes);

            // `other` is 9-14 letters so it cannot equal any 1-8 letter include
            // token: a title made only of it matches none and is excluded.
            let miss_title = other.clone();
            let miss = matches(&criteria, &aliases, view(&miss_title, None));
            prop_assert!(!miss.included, "expected miss for title {:?} vs {:?}", miss_title, includes);
            prop_assert_eq!(miss.reason, "no title include matched");
        }
    }

    // --- 3.7 Property 4: Case-insensitivity (Validates Requirements 4.1, 4.2) ---

    proptest! {
        #[test]
        fn prop_case_insensitivity(
            title in text_strategy(),
            loc in text_strategy(),
            title_toks in prop::collection::vec(token_strategy(), 0..3),
            loc_toks in prop::collection::vec(token_strategy(), 0..3),
        ) {
            let aliases = AliasTable::default_seed();
            // Vary case of criteria tokens too, to exercise Req 4.1.
            let criteria = FilterCriteria {
                title: TitleCriteria {
                    include: title_toks.iter().map(|t| t.to_uppercase()).collect(),
                    ..Default::default()
                },
                location: LocationCriteria {
                    include: loc_toks.clone(),
                    ..Default::default()
                },
                ..FilterCriteria::match_all()
            };

            let base = matches(&criteria, &aliases, view(&title, Some(&loc))).included;
            let upper = matches(
                &criteria,
                &aliases,
                view(&title.to_uppercase(), Some(&loc.to_uppercase())),
            ).included;
            let lower = matches(
                &criteria,
                &aliases,
                view(&title.to_lowercase(), Some(&loc.to_lowercase())),
            ).included;

            // Property 4: inclusion result is invariant under case changes.
            prop_assert_eq!(base, upper);
            prop_assert_eq!(base, lower);
        }
    }

    // --- 3.8 Property 5: No substring false positives in Word mode
    //         (Validates Requirements 1.1, 1.2) ---

    #[test]
    fn word_mode_ca_california_ambiguity_class() {
        // Concrete ca/California-style ambiguity: "qa" must NOT match inside
        // "Quality Assurance Engineer" (no standalone token) but MUST match
        // "QA Engineer".
        let aliases = AliasTable::default_seed();
        let criteria = title_include(&["qa"]);
        assert!(
            !matches(
                &criteria,
                &aliases,
                view("Quality Assurance Engineer", None)
            )
            .included
        );
        assert!(matches(&criteria, &aliases, view("QA Engineer", None)).included);
    }

    proptest! {
        #[test]
        fn prop_word_mode_no_substring_false_positive(
            needle in "[a-z]{2,6}",
            // Left/right padding of letters only, so the needle is embedded as a
            // proper substring of a single larger alphanumeric token (no
            // delimiter or whitespace to create a token boundary).
            left in "[a-z]{1,5}",
            right in "[a-z]{1,5}",
        ) {
            let aliases = AliasTable::default_seed();
            // The embedded token strictly contains `needle` as a substring but
            // is a different, longer token: e.g. needle "qa" inside "aqar".
            let embedded = format!("{left}{needle}{right}");
            prop_assume!(embedded != needle);

            let title = format!("senior {embedded} engineer");
            let criteria = title_include(&[needle.as_str()]);
            let result = matches(&criteria, &aliases, view(&title, None));
            // Property 5: Word-mode single-token needle never matches a proper
            // substring of a larger token.
            prop_assert!(
                !result.included,
                "needle {:?} should not match embedded token {:?}",
                needle,
                embedded
            );
        }
    }

    // --- 3.9 Property 7: Determinism & purity (Validates Requirements 11.1, 11.2) ---

    proptest! {
        #[test]
        fn prop_determinism_repeated_calls_equal(
            title in text_strategy(),
            loc in text_strategy(),
            has_loc in any::<bool>(),
            title_inc in prop::collection::vec(token_strategy(), 0..3),
            title_exc in prop::collection::vec(token_strategy(), 0..3),
            loc_inc in prop::collection::vec(token_strategy(), 0..3),
            remote_sel in 0u8..3,
        ) {
            let aliases = AliasTable::default_seed();
            let remote = match remote_sel {
                0 => RemoteMode::Any,
                1 => RemoteMode::RemoteOnly,
                _ => RemoteMode::OnsiteOnly,
            };
            let criteria = FilterCriteria {
                title: TitleCriteria {
                    include: title_inc,
                    exclude: title_exc,
                    ..Default::default()
                },
                location: LocationCriteria {
                    include: loc_inc,
                    ..Default::default()
                },
                remote,
                ..FilterCriteria::match_all()
            };
            let location = if has_loc { Some(loc.as_str()) } else { None };

            let first = matches(&criteria, &aliases, view(&title, location));
            let second = matches(&criteria, &aliases, view(&title, location));
            // Property 7: identical inputs yield identical decision + reason.
            prop_assert_eq!(first.included, second.included);
            prop_assert_eq!(first.reason, second.reason);
        }
    }

    // --- 3.10 Property 11: Remote gate soundness
    //          (Validates Requirements 6.1, 6.2, 6.3) ---

    /// Locations built from remote tokens vs concrete cities, used by the remote
    /// gate property tests.
    fn location_sample_strategy() -> impl Strategy<Value = String> {
        prop::sample::select(vec![
            "Remote".to_string(),
            "Remote - US".to_string(),
            "Anywhere".to_string(),
            "Fully Distributed".to_string(),
            "wfh".to_string(),
            "San Jose, CA".to_string(),
            "Austin, TX".to_string(),
            "New York, NY".to_string(),
            "Boston, MA".to_string(),
        ])
    }

    proptest! {
        #[test]
        fn prop_remote_only_implies_is_remote(loc in location_sample_strategy()) {
            let aliases = AliasTable::default_seed();
            let criteria = FilterCriteria {
                remote: RemoteMode::RemoteOnly,
                ..FilterCriteria::match_all()
            };
            let included = matches(&criteria, &aliases, view("Engineer", Some(&loc))).included;
            // Property 11: RemoteOnly inclusion implies the location is remote.
            if included {
                prop_assert!(aliases.is_remote(&normalize(&loc)));
            }
        }
    }

    proptest! {
        #[test]
        fn prop_onsite_only_excludes_remote(loc in location_sample_strategy()) {
            let aliases = AliasTable::default_seed();
            let criteria = FilterCriteria {
                remote: RemoteMode::OnsiteOnly,
                ..FilterCriteria::match_all()
            };
            let included = matches(&criteria, &aliases, view("Engineer", Some(&loc))).included;
            // Property 11: OnsiteOnly -> a remote location implies exclusion.
            if aliases.is_remote(&normalize(&loc)) {
                prop_assert!(!included);
            }
        }
    }

    proptest! {
        #[test]
        fn prop_any_mode_ignores_remote_status(loc in location_sample_strategy()) {
            let aliases = AliasTable::default_seed();
            // With match-all + RemoteMode::Any (the default), a job that passes
            // (everything does) is unaffected by its remote status.
            let criteria = FilterCriteria::match_all();
            let included = matches(&criteria, &aliases, view("Engineer", Some(&loc))).included;
            // Property 11: Any imposes no remote constraint.
            prop_assert!(included);
        }
    }

    // --- 3.11 Property 12: Whitespace/punctuation invariance
    //          (Validates Requirements 4.3) ---

    #[test]
    fn location_whitespace_and_delimiter_spacing_invariance() {
        let aliases = AliasTable::default_seed();
        let criteria = FilterCriteria {
            location: LocationCriteria {
                include: vec!["san jose".to_string()],
                ..Default::default()
            },
            ..FilterCriteria::match_all()
        };
        let variants = ["San Jose, CA", " San Jose ,  CA ", "San Jose,CA"];
        let base = matches(&criteria, &aliases, view("Engineer", Some(variants[0]))).included;
        assert!(base, "sanity: 'san jose' should match the first variant");
        for v in variants {
            let r = matches(&criteria, &aliases, view("Engineer", Some(v)));
            assert_eq!(r.included, base, "variant {v:?} differed");
        }
    }

    proptest! {
        #[test]
        fn prop_location_spacing_invariance(
            city in prop::sample::select(vec!["san jose", "new york", "los angeles", "san francisco"]),
        ) {
            let aliases = AliasTable::default_seed();
            let criteria = FilterCriteria {
                location: LocationCriteria {
                    include: vec![city.to_string()],
                    ..Default::default()
                },
                ..FilterCriteria::match_all()
            };
            // Three renderings differing only in whitespace / delimiter spacing.
            let compact = format!("{city},CA");
            let normal = format!("{city}, CA");
            let padded = format!("  {city}  ,   CA  ");
            let a = matches(&criteria, &aliases, view("Engineer", Some(&compact))).included;
            let b = matches(&criteria, &aliases, view("Engineer", Some(&normal))).included;
            let c = matches(&criteria, &aliases, view("Engineer", Some(&padded))).included;
            // Property 12: identical inclusion regardless of spacing.
            prop_assert_eq!(a, b);
            prop_assert_eq!(a, c);
        }
    }

    // --- 3.12 Unit tests for engine edge cases
    //          (Validates Requirements 1.2, 1.7, 2.1, 6.4) ---

    #[test]
    fn edge_empty_token_title_text_word_include_no_match() {
        // Req 1.7: a title that normalizes/tokenizes to zero tokens matches no
        // Word-mode include term, so a job with an active title include is
        // excluded.
        let aliases = AliasTable::default_seed();
        let criteria = title_include(&["engineer"]);
        for empty in ["", "   ", "\t\n"] {
            let r = matches(&criteria, &aliases, view(empty, None));
            assert!(
                !r.included,
                "empty title {empty:?} should not match include"
            );
            assert_eq!(r.reason, "no title include matched");
        }
    }

    #[test]
    fn edge_multi_word_contiguous_run_include_matches() {
        // Req 1.2: a multi-word include term matches only as an ordered
        // contiguous run of tokens.
        let aliases = AliasTable::default_seed();
        let criteria = title_include(&["software engineer"]);
        // Contiguous ordered run present -> included.
        assert!(matches(&criteria, &aliases, view("Senior Software Engineer", None)).included);
        // Tokens present but separated -> excluded.
        let split = matches(
            &criteria,
            &aliases,
            view("Software Platform Engineer", None),
        );
        assert!(!split.included);
        assert_eq!(split.reason, "no title include matched");
    }

    #[test]
    fn edge_exclude_over_include_on_location_dimension() {
        // Req 2.1: on the location dimension, an active exclude beats an include
        // that would otherwise match.
        let aliases = AliasTable::default_seed();
        let criteria = FilterCriteria {
            location: LocationCriteria {
                include: vec!["california".to_string()],
                exclude: vec!["san jose".to_string()],
                ..Default::default()
            },
            ..FilterCriteria::match_all()
        };
        let r = matches(&criteria, &aliases, view("Engineer", Some("San Jose, CA")));
        assert!(!r.included);
        assert_eq!(r.reason, "excluded by location 'san jose'");
    }

    #[test]
    fn edge_remote_bypasses_location_name_rule() {
        // Req 6.4: with RemoteMode::Any and a specific location include, a purely
        // remote location bypasses the location-name gate, while a remote
        // location that also names a non-matching place stays gated.
        let aliases = AliasTable::default_seed();
        let criteria = FilterCriteria {
            location: LocationCriteria {
                include: vec!["san francisco".to_string()],
                ..Default::default()
            },
            ..FilterCriteria::match_all()
        };
        let bypass = matches(&criteria, &aliases, view("Engineer", Some("Remote")));
        assert!(bypass.included);
        assert_eq!(bypass.reason, "remote passes location gate");

        let gated = matches(
            &criteria,
            &aliases,
            view("Engineer", Some("Remote - New York")),
        );
        assert!(!gated.included);
        assert_eq!(gated.reason, "no location include matched");
    }
}
