//! Filter_Criteria data model.
//!
//! The serializable, versioned representation of what to include/exclude when
//! filtering jobs. Stored as JSON in `app_settings` (global scope) and in
//! `company_watches.filter_criteria` (per-watch scope).
//!
//! Design references: `Core Types and Signatures` in the design document.
//! Requirements: 3.2 (Match_All value), 6.6 (absent/unknown Remote_Mode behaves
//! as Any via `Default`), 17.3 (token trim/drop-empty normalization helper).

use serde::{Deserialize, Serialize};

/// Default schema version for freshly constructed / deserialized criteria.
fn default_version() -> u32 {
    1
}

/// How include/exclude terms are matched against job text.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum MatchMode {
    /// Match on whitespace/punctuation-delimited token boundaries
    /// (e.g. "qa" does not match "quality"). This is the default.
    #[default]
    Word,
    /// Match as a contiguous substring, preserving legacy `LIKE '%term%'`
    /// behavior for users who want it.
    Substring,
}

/// Remote-status constraint applied to a job's location.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum RemoteMode {
    /// No remote constraint. This is the default and the fallback for any
    /// absent or unrecognized value on deserialization (Req 6.6).
    #[default]
    Any,
    /// Include a job only if its location is identified as remote.
    RemoteOnly,
    /// Exclude any job whose location is identified as remote.
    OnsiteOnly,
}

/// Title matching criteria: any-of include, none-of exclude.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TitleCriteria {
    /// Any-of include terms; an empty list means "match all titles".
    #[serde(default)]
    pub include: Vec<String>,
    /// None-of exclude terms; an empty list means "exclude nothing".
    #[serde(default)]
    pub exclude: Vec<String>,
    /// How include/exclude terms are matched.
    #[serde(default)]
    pub match_mode: MatchMode,
}

/// Location matching criteria: an optional country context plus any-of include
/// and none-of exclude terms that are expanded via the alias table at match time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocationCriteria {
    /// Canonical country name, or `None` for any country.
    #[serde(default)]
    pub country: Option<String>,
    /// Any-of include terms (regions/cities/states); expanded via aliases.
    #[serde(default)]
    pub include: Vec<String>,
    /// Explicit location exclusions.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// How include/exclude terms are matched.
    #[serde(default)]
    pub match_mode: MatchMode,
}

/// The top-level, versioned filter criteria value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FilterCriteria {
    /// Schema version, starts at 1. Defaults to 1 when absent.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Title matching criteria.
    #[serde(default)]
    pub title: TitleCriteria,
    /// Location matching criteria.
    #[serde(default)]
    pub location: LocationCriteria,
    /// Remote-status constraint.
    #[serde(default)]
    pub remote: RemoteMode,
}

impl Default for FilterCriteria {
    fn default() -> Self {
        Self::match_all()
    }
}

impl FilterCriteria {
    /// The identity criteria: includes every job.
    ///
    /// All title/location include and exclude lists are empty and
    /// `remote` is [`RemoteMode::Any`] (Req 3.2).
    pub fn match_all() -> Self {
        FilterCriteria {
            version: default_version(),
            title: TitleCriteria::default(),
            location: LocationCriteria::default(),
            remote: RemoteMode::Any,
        }
    }

    /// True when this criteria would include every possible job: all include
    /// and exclude lists empty across both dimensions and `remote == Any`.
    pub fn is_match_all(&self) -> bool {
        self.title.include.is_empty()
            && self.title.exclude.is_empty()
            && self.location.include.is_empty()
            && self.location.exclude.is_empty()
            && self.location.country.is_none()
            && self.remote == RemoteMode::Any
    }
}

/// Normalize a list of user-entered tokens for storage: trim surrounding
/// whitespace from each token and drop any that are empty or whitespace-only.
///
/// Used by the setter paths so persisted criteria never contain empty or
/// untrimmed tokens (Req 17.3). Preserves order and internal spacing of the
/// remaining tokens; comparison-time normalization (lowercasing, whitespace
/// collapsing) is handled separately by the engine.
pub fn normalize_tokens<I, S>(tokens: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    tokens
        .into_iter()
        .map(|t| t.as_ref().trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// camelCase JSON round-trip: serialize then deserialize yields an equal
    /// value, and the JSON uses camelCase keys (Req 3.2, 17.3).
    #[test]
    fn camel_case_json_round_trip() {
        let criteria = FilterCriteria {
            version: 1,
            title: TitleCriteria {
                include: vec!["engineer".to_string()],
                exclude: vec!["senior".to_string()],
                match_mode: MatchMode::Substring,
            },
            location: LocationCriteria {
                country: Some("United States".to_string()),
                include: vec!["bay area".to_string()],
                exclude: vec!["texas".to_string()],
                match_mode: MatchMode::Word,
            },
            remote: RemoteMode::RemoteOnly,
        };

        let json = serde_json::to_string(&criteria).expect("serialize");
        let round_tripped: FilterCriteria = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(criteria, round_tripped);

        // camelCase keys present, snake_case absent.
        assert!(
            json.contains("\"matchMode\""),
            "expected camelCase key in {json}"
        );
        assert!(
            !json.contains("match_mode"),
            "unexpected snake_case key in {json}"
        );

        // Enum values serialize as camelCase strings.
        assert!(
            json.contains("\"substring\""),
            "expected MatchMode value in {json}"
        );
        assert!(
            json.contains("\"remoteOnly\""),
            "expected RemoteMode value in {json}"
        );
    }

    /// A criteria omitting "version" deserializes with version == 1 (serde default).
    #[test]
    fn missing_version_defaults_to_one() {
        let from_empty: FilterCriteria = serde_json::from_str("{}").expect("deserialize {}");
        assert_eq!(from_empty.version, 1);

        let from_partial: FilterCriteria =
            serde_json::from_str("{\"title\":{}}").expect("deserialize partial");
        assert_eq!(from_partial.version, 1);
    }

    /// Enum values deserialize from their camelCase string forms.
    #[test]
    fn enum_values_deserialize_from_camel_case() {
        let onsite: FilterCriteria =
            serde_json::from_str("{\"remote\":\"onsiteOnly\"}").expect("deserialize");
        assert_eq!(onsite.remote, RemoteMode::OnsiteOnly);

        let substring: TitleCriteria =
            serde_json::from_str("{\"matchMode\":\"substring\"}").expect("deserialize");
        assert_eq!(substring.match_mode, MatchMode::Substring);
    }

    /// `match_all()` is the identity criteria: `is_match_all()` true and remote is Any (Req 3.2).
    #[test]
    fn match_all_is_match_all_and_remote_any() {
        let all = FilterCriteria::match_all();
        assert!(all.is_match_all());
        assert_eq!(all.remote, RemoteMode::Any);
    }

    /// A criteria with a non-empty title include is not match-all.
    #[test]
    fn non_empty_title_include_is_not_match_all() {
        let mut criteria = FilterCriteria::match_all();
        criteria.title.include.push("engineer".to_string());
        assert!(!criteria.is_match_all());
    }

    /// A non-Any remote mode makes a criteria not match-all.
    #[test]
    fn remote_only_is_not_match_all() {
        let mut criteria = FilterCriteria::match_all();
        criteria.remote = RemoteMode::RemoteOnly;
        assert!(!criteria.is_match_all());
    }

    /// The default enum discriminants are Word and Any.
    #[test]
    fn enum_defaults() {
        assert_eq!(MatchMode::default(), MatchMode::Word);
        assert_eq!(RemoteMode::default(), RemoteMode::Any);
    }

    /// `normalize_tokens` trims each token and drops empty/whitespace-only ones,
    /// preserving order (Req 17.3).
    #[test]
    fn normalize_tokens_trims_and_drops_empties() {
        let out = normalize_tokens(vec!["  a ", "", "  ", "b"]);
        assert_eq!(out, vec!["a".to_string(), "b".to_string()]);
    }

    /// `normalize_tokens` preserves internal spacing while trimming edges.
    #[test]
    fn normalize_tokens_preserves_internal_spacing() {
        let out = normalize_tokens(vec!["  bay area  ", "\tnew york\n"]);
        assert_eq!(out, vec!["bay area".to_string(), "new york".to_string()]);
    }

    /// `normalize_tokens` on an all-empty input yields an empty vec.
    #[test]
    fn normalize_tokens_all_empty() {
        let out = normalize_tokens(vec!["", "   ", "\t\n"]);
        assert!(out.is_empty());
    }
}
