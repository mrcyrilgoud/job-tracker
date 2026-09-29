//! Alias_Table: regions, country expansions, and remote tokens.
//!
//! `AliasTable` is the data-driven replacement for the hardcoded
//! `expand_country_keywords` / `expand_location_keywords` logic that used to
//! live in `jobs::service`. It maps region tokens to their member location
//! tokens, country canonical names to their expansion tokens (states,
//! two-letter abbreviations, and major city hubs), and holds the set of tokens
//! that indicate remote work.
//!
//! This module (task 2.1) defines the struct and its default seed only.
//! Expansion / disambiguation / remote-detection behavior (`expand_location`,
//! `is_remote`) is implemented in task 2.2.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The current on-disk schema version for a serialized `AliasTable`.
pub const ALIAS_TABLE_VERSION: u32 = 1;

/// Data-driven table of location aliases and remote tokens.
///
/// All map keys and token values are stored lowercased and trimmed so that
/// later lookups can be performed case-insensitively against normalized input.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasTable {
    /// Schema version of this alias table.
    pub version: u32,
    /// Region token -> member location tokens
    /// (e.g. "bay area" -> ["san francisco", "san jose", ...]).
    pub regions: HashMap<String, Vec<String>>,
    /// Country canonical name -> expansion tokens (states, abbreviations, hubs).
    pub countries: HashMap<String, Vec<String>>,
    /// Tokens that indicate remote work.
    pub remote_tokens: Vec<String>,
}

impl AliasTable {
    /// Seed with parity to today's `expand_country_keywords` /
    /// `expand_location_keywords` coverage.
    ///
    /// Location coverage includes:
    /// - all United States state names and their two-letter abbreviations,
    /// - the major United States city hubs, and
    /// - the regions `bay area`, `greater new york`, `greater seattle`, and
    ///   `greater los angeles` with their member cities.
    ///
    /// Remote tokens are `remote`, `anywhere`, `distributed`, and `wfh`.
    pub fn default_seed() -> Self {
        AliasTable {
            version: ALIAS_TABLE_VERSION,
            regions: default_regions(),
            countries: default_countries(),
            remote_tokens: default_remote_tokens(),
        }
    }

    /// Expand a user-entered location token to itself plus any alias members.
    ///
    /// Behavior (Requirements 5.1, 5.4-5.7):
    /// - The token is normalized (trimmed + lowercased) before lookup.
    /// - A known region key (e.g. `bay area`) expands to itself plus its
    ///   member location tokens (Req 5.1).
    /// - An unknown token passes through unchanged as its own effective
    ///   include token (Req 5.4).
    /// - The ambiguous token `ca` is disambiguated by `country`: it expands to
    ///   California tokens ONLY when the (normalized) country is the United
    ///   States (`united states` / `usa` / `us`). When the country is Canada,
    ///   or absent, `ca` passes through as just `ca` and is NOT expanded to
    ///   California (Req 5.5, 5.6, 5.7).
    ///
    /// The returned vector is deduplicated while preserving first-seen order.
    pub fn expand_location(&self, token: &str, country: Option<&str>) -> Vec<String> {
        let norm = normalize_token(token);
        if norm.is_empty() {
            return Vec::new();
        }

        // Ambiguous `ca`: only expand to California when the country context is
        // the United States. Otherwise (Canada / absent / any other country)
        // the token passes through unchanged.
        if norm == "ca" {
            if country_is_united_states(country) {
                return dedup(california_tokens());
            }
            return vec![norm];
        }

        // Known region token: expand to self + members.
        if let Some(members) = self.regions.get(&norm) {
            let mut out = Vec::with_capacity(members.len() + 1);
            out.push(norm);
            out.extend(members.iter().cloned());
            return dedup(out);
        }

        // Unknown / non-region token: pass through unchanged (Req 5.4).
        vec![norm]
    }

    /// Return the expansion tokens for a country from the `countries` map.
    ///
    /// The country name is normalized (trimmed + lowercased) before lookup, and
    /// the common United States synonyms (`usa` / `us`) are canonicalized to
    /// `united states`. The engine adds these tokens to the effective location
    /// include set (Req 5.2). Returns an empty vector for an unknown country.
    pub fn expand_country(&self, country: &str) -> Vec<String> {
        let norm = normalize_token(country);
        if norm.is_empty() {
            return Vec::new();
        }
        let key = canonical_country(&norm);
        self.countries.get(&key).cloned().unwrap_or_default()
    }

    /// Return whether text names a configured country, including canonical
    /// synonyms such as `us` and `usa` for `united states`.
    pub fn is_country_name(&self, text: &str) -> bool {
        let norm = normalize_token(text);
        !norm.is_empty() && self.countries.contains_key(&canonical_country(&norm))
    }

    /// Identify a location as remote (Req 6.5).
    ///
    /// Returns true iff the location text contains at least one configured
    /// remote token as a case-insensitive substring. The input is normalized
    /// internally so callers may pass either raw or already-normalized text.
    pub fn is_remote(&self, location_norm: &str) -> bool {
        let haystack = location_norm.to_lowercase();
        if haystack.trim().is_empty() {
            return false;
        }
        self.remote_tokens
            .iter()
            .any(|tok| !tok.is_empty() && haystack.contains(&tok.to_lowercase()))
    }
}

/// Normalize a single token: trim surrounding whitespace and lowercase.
fn normalize_token(token: &str) -> String {
    token.trim().to_lowercase()
}

/// True when the (optional) country resolves to the United States.
fn country_is_united_states(country: Option<&str>) -> bool {
    match country {
        Some(c) => matches!(
            normalize_token(c).as_str(),
            "united states" | "usa" | "us"
        ),
        None => false,
    }
}

/// Canonicalize common country synonyms to their map key.
fn canonical_country(norm: &str) -> String {
    match norm {
        "usa" | "us" => "united states".to_string(),
        other => other.to_string(),
    }
}

/// California expansion tokens (name + abbreviation + major CA hubs).
fn california_tokens() -> Vec<String> {
    to_tokens(&[
        "california",
        "ca",
        "san francisco",
        "san jose",
        "oakland",
        "santa clara",
        "palo alto",
        "mountain view",
        "sunnyvale",
        "cupertino",
        "menlo park",
        "san mateo",
        "redwood city",
        "berkeley",
        "los angeles",
        "santa monica",
        "venice",
        "culver city",
        "irvine",
    ])
}

impl Default for AliasTable {
    fn default() -> Self {
        Self::default_seed()
    }
}

/// Tokens indicating remote work.
fn default_remote_tokens() -> Vec<String> {
    ["remote", "anywhere", "distributed", "wfh"]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Region token -> member location tokens.
///
/// Mirrors the region groupings that `expand_location_keywords` produced. Each
/// region maps to its member cities; the region token itself is intentionally
/// omitted from the member list (the expander adds the token itself in task
/// 2.2).
fn default_regions() -> HashMap<String, Vec<String>> {
    let mut regions: HashMap<String, Vec<String>> = HashMap::new();

    regions.insert(
        "bay area".to_string(),
        to_tokens(&[
            "san francisco",
            "san jose",
            "oakland",
            "santa clara",
            "palo alto",
            "mountain view",
            "sunnyvale",
            "cupertino",
            "menlo park",
            "san mateo",
            "redwood city",
            "berkeley",
        ]),
    );

    regions.insert(
        "greater new york".to_string(),
        to_tokens(&[
            "new york",
            "nyc",
            "brooklyn",
            "manhattan",
            "queens",
            "jersey city",
        ]),
    );

    regions.insert(
        "greater seattle".to_string(),
        to_tokens(&["seattle", "bellevue", "redmond", "kirkland"]),
    );

    regions.insert(
        "greater los angeles".to_string(),
        to_tokens(&[
            "los angeles",
            "santa monica",
            "venice",
            "culver city",
            "irvine",
        ]),
    );

    regions
}

/// Country canonical name -> expansion tokens.
///
/// The United States entry carries every state name, every two-letter state
/// abbreviation, and the major city hubs, matching the coverage of the former
/// `expand_country_keywords` function.
fn default_countries() -> HashMap<String, Vec<String>> {
    let mut countries: HashMap<String, Vec<String>> = HashMap::new();

    let mut us_tokens: Vec<String> = Vec::new();
    // Country self / synonyms.
    us_tokens.extend(to_tokens(&["united states", "usa", "us"]));
    // All US states: full name + two-letter abbreviation.
    us_tokens.extend(us_state_tokens());
    // Major US city hubs.
    us_tokens.extend(us_city_hubs());
    countries.insert("united states".to_string(), dedup(us_tokens));

    countries
}

/// All United States states as (abbreviation, full name) tokens, lowercased.
fn us_state_tokens() -> Vec<String> {
    const STATES: &[(&str, &str)] = &[
        ("al", "alabama"),
        ("ak", "alaska"),
        ("az", "arizona"),
        ("ar", "arkansas"),
        ("ca", "california"),
        ("co", "colorado"),
        ("ct", "connecticut"),
        ("de", "delaware"),
        ("fl", "florida"),
        ("ga", "georgia"),
        ("hi", "hawaii"),
        ("id", "idaho"),
        ("il", "illinois"),
        ("in", "indiana"),
        ("ia", "iowa"),
        ("ks", "kansas"),
        ("ky", "kentucky"),
        ("la", "louisiana"),
        ("me", "maine"),
        ("md", "maryland"),
        ("ma", "massachusetts"),
        ("mi", "michigan"),
        ("mn", "minnesota"),
        ("ms", "mississippi"),
        ("mo", "missouri"),
        ("mt", "montana"),
        ("ne", "nebraska"),
        ("nv", "nevada"),
        ("nh", "new hampshire"),
        ("nj", "new jersey"),
        ("nm", "new mexico"),
        ("ny", "new york"),
        ("nc", "north carolina"),
        ("nd", "north dakota"),
        ("oh", "ohio"),
        ("ok", "oklahoma"),
        ("or", "oregon"),
        ("pa", "pennsylvania"),
        ("ri", "rhode island"),
        ("sc", "south carolina"),
        ("sd", "south dakota"),
        ("tn", "tennessee"),
        ("tx", "texas"),
        ("ut", "utah"),
        ("vt", "vermont"),
        ("va", "virginia"),
        ("wa", "washington"),
        ("wv", "west virginia"),
        ("wi", "wisconsin"),
        ("wy", "wyoming"),
    ];

    let mut tokens = Vec::with_capacity(STATES.len() * 2);
    for (abbrev, name) in STATES {
        tokens.push((*abbrev).to_string());
        tokens.push((*name).to_string());
    }
    tokens
}

/// Major United States city hubs, lowercased.
fn us_city_hubs() -> Vec<String> {
    to_tokens(&[
        "san francisco",
        "san jose",
        "new york",
        "nyc",
        "seattle",
        "austin",
        "boston",
        "chicago",
        "los angeles",
        "palo alto",
        "mountain view",
        "sunnyvale",
        "santa clara",
        "menlo park",
        "redwood city",
        "san mateo",
        "oakland",
        "berkeley",
        "santa monica",
        "venice",
        "culver city",
        "irvine",
        "brooklyn",
        "manhattan",
        "queens",
        "jersey city",
        "bellevue",
        "redmond",
        "kirkland",
        "cambridge",
        "atlanta",
        "denver",
        "boulder",
        "salt lake city",
        "washington dc",
        "miami",
        "dallas",
        "houston",
        "raleigh",
        "bay area",
    ])
}

/// Normalize a slice of static strings into lowercased, trimmed owned tokens.
fn to_tokens(values: &[&str]) -> Vec<String> {
    values
        .iter()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Deduplicate tokens while preserving first-seen order.
fn dedup(tokens: Vec<String>) -> Vec<String> {
    let mut seen = Vec::with_capacity(tokens.len());
    for t in tokens {
        if !seen.contains(&t) {
            seen.push(t);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ca_expands_to_california_only_for_united_states() {
        let table = AliasTable::default_seed();

        // Req 5.6: US -> `ca` expands to California tokens.
        for country in ["united states", "USA", "us", " United States "] {
            let expanded = table.expand_location("ca", Some(country));
            assert!(
                expanded.contains(&"california".to_string()),
                "expected California expansion for country {country:?}, got {expanded:?}"
            );
            assert!(expanded.len() > 1, "expected multiple CA tokens for {country:?}");
        }

        // Req 5.5: Canada -> `ca` does NOT expand to California.
        let canada = table.expand_location("ca", Some("Canada"));
        assert_eq!(canada, vec!["ca".to_string()]);

        // Req 5.7: no country -> `ca` does NOT expand to California.
        let no_country = table.expand_location("ca", None);
        assert_eq!(no_country, vec!["ca".to_string()]);
    }

    #[test]
    fn is_remote_detects_configured_tokens_case_insensitively() {
        let table = AliasTable::default_seed();

        assert!(table.is_remote("Remote - US"));
        assert!(table.is_remote("ANYWHERE"));
        assert!(table.is_remote("Fully Distributed team"));
        assert!(table.is_remote("wfh"));

        assert!(!table.is_remote("San Francisco, CA"));
        assert!(!table.is_remote(""));
    }

    #[test]
    fn is_country_name_recognizes_configured_country_synonyms() {
        let table = AliasTable::default_seed();

        for country in ["United States", "USA", "us"] {
            assert!(table.is_country_name(country));
        }
        assert!(!table.is_country_name("New York"));
        assert!(!table.is_country_name(""));
    }

    // --- Task 2.4: unit tests for alias expansion and remote detection ---

    /// Req 5.1: a known region expands to itself plus its member cities, and
    /// the result contains no duplicate tokens.
    #[test]
    fn region_expands_to_self_plus_members_deduped() {
        let table = AliasTable::default_seed();

        let expanded = table.expand_location("bay area", None);

        // Contains the region token itself plus representative members.
        assert!(
            expanded.contains(&"bay area".to_string()),
            "expected region token itself, got {expanded:?}"
        );
        assert!(
            expanded.contains(&"san francisco".to_string()),
            "expected member 'san francisco', got {expanded:?}"
        );
        assert!(
            expanded.contains(&"san jose".to_string()),
            "expected member 'san jose', got {expanded:?}"
        );
        assert!(
            expanded.len() > 1,
            "expected multiple tokens for a region, got {expanded:?}"
        );

        // Dedup: no token appears more than once.
        let mut seen: Vec<&String> = Vec::new();
        for tok in &expanded {
            assert!(
                !seen.contains(&tok),
                "duplicate token {tok:?} in region expansion {expanded:?}"
            );
            seen.push(tok);
        }
    }

    /// Req 5.4: an unknown token passes through unchanged as exactly one token,
    /// and normalization (trim + lowercase) is applied to it.
    #[test]
    fn unknown_token_passes_through_normalized() {
        let table = AliasTable::default_seed();

        // Exact passthrough for a plain unknown token.
        assert_eq!(
            table.expand_location("atlantis", None),
            vec!["atlantis".to_string()]
        );

        // Surrounding whitespace and mixed case are trimmed + lowercased.
        assert_eq!(
            table.expand_location("  AtLantis  ", None),
            vec!["atlantis".to_string()]
        );
    }

    /// Req 6.5: `is_remote` detects configured tokens case-insensitively as a
    /// substring, and returns false for concrete places and empty input.
    #[test]
    fn is_remote_covers_all_configured_tokens_and_negatives() {
        let table = AliasTable::default_seed();

        assert!(table.is_remote("Remote"));
        assert!(table.is_remote("REMOTE - US"));
        assert!(table.is_remote("Anywhere"));
        assert!(table.is_remote("Fully Distributed"));
        assert!(table.is_remote("wfh"));

        assert!(!table.is_remote("San Francisco, CA"));
        assert!(!table.is_remote(""));
    }

    // --- Task 2.3: property test for country disambiguation (Property 6) ---
    // Validates: Requirements 5.5, 5.6, 5.7

    use proptest::prelude::*;

    /// Country context choices for the disambiguation property: United States
    /// (and synonyms) should enable California expansion; Canada and "no
    /// country" should not.
    #[derive(Debug, Clone)]
    enum CountryChoice {
        /// A country string that resolves to the United States.
        UnitedStates(String),
        /// A country string that resolves to Canada.
        Canada(String),
        /// No country context provided.
        None,
    }

    prop_compose! {
        /// Wrap a base token in random surrounding ASCII whitespace and randomized
        /// letter casing, without altering the token's meaning.
        fn noisy_variant(base: &'static str)
            (
                lead in prop::collection::vec(prop::sample::select(vec![' ', '\t']), 0..3),
                trail in prop::collection::vec(prop::sample::select(vec![' ', '\t']), 0..3),
                upper_mask in prop::collection::vec(any::<bool>(), base.len()),
            ) -> String
        {
            let cased: String = base
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    if *upper_mask.get(i).unwrap_or(&false) {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    }
                })
                .collect();
            let lead: String = lead.into_iter().collect();
            let trail: String = trail.into_iter().collect();
            format!("{lead}{cased}{trail}")
        }
    }

    /// Strategy over country contexts: US synonyms vs Canada vs None, each with
    /// case/whitespace noise applied to the country string.
    fn country_choice_strategy() -> impl Strategy<Value = CountryChoice> {
        prop_oneof![
            noisy_variant("united states").prop_map(CountryChoice::UnitedStates),
            noisy_variant("usa").prop_map(CountryChoice::UnitedStates),
            noisy_variant("us").prop_map(CountryChoice::UnitedStates),
            noisy_variant("canada").prop_map(CountryChoice::Canada),
            Just(CountryChoice::None),
        ]
    }

    proptest! {
        /// Property 6: `expand_location("ca", country)` contains "california"
        /// iff the country context resolves to the United States. Canada and an
        /// absent country never leak California; the US always includes it.
        /// There is no cross-country leakage.
        #[test]
        fn ca_disambiguation_invariant(choice in country_choice_strategy()) {
            let table = AliasTable::default_seed();

            let (arg, expect_california): (Option<&str>, bool) = match &choice {
                CountryChoice::UnitedStates(s) => (Some(s.as_str()), true),
                CountryChoice::Canada(s) => (Some(s.as_str()), false),
                CountryChoice::None => (None, false),
            };

            let expanded = table.expand_location("ca", arg);
            let has_california = expanded.contains(&"california".to_string());

            prop_assert_eq!(
                has_california,
                expect_california,
                "california membership mismatch for {:?}: got {:?}",
                choice,
                expanded
            );

            if expect_california {
                // US context: real expansion with multiple tokens including "ca".
                prop_assert!(expanded.contains(&"ca".to_string()));
                prop_assert!(expanded.len() > 1, "expected multiple CA tokens, got {:?}", expanded);
            } else {
                // Non-US / absent: pass through as exactly ["ca"].
                prop_assert_eq!(expanded, vec!["ca".to_string()]);
            }
        }
    }
}
