//! Shared company-score input types and validation.
//!
//! This module owns the shared wire/domain rules and transactional persistence
//! for company scores. Every caller uses the same tri-state fields, source
//! values, metadata normalization, company-row writes, and cache fan-out.

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Deserializer, Serialize};
use url::Url;

use crate::error::{map_sqlite, AppError, AppResult};
use crate::models::{checked_company_stability, checked_employment_stability, CompanyScores};
pub use crate::models::{COMPANY_STABILITY_RANGE_MESSAGE, EMPLOYMENT_STABILITY_RANGE_MESSAGE};
use crate::util::now_iso;

pub const MAX_RATIONALE_CODE_POINTS: usize = 2_000;
pub const MAX_CITATIONS: usize = 20;
pub const MAX_CITATION_CODE_POINTS: usize = 2_048;

/// A request field that distinguishes omission (preserve) from a supplied
/// value. The inner `Option<T>` gives the supplied value its own clear state:
/// `Present(Some(value))` sets it and `Present(None)` clears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionalField<T> {
    Omitted,
    Present(T),
}

impl<T> Default for OptionalField<T> {
    fn default() -> Self {
        Self::Omitted
    }
}

impl<T> OptionalField<T> {
    pub fn omitted() -> Self {
        Self::Omitted
    }

    pub fn present(value: T) -> Self {
        Self::Present(value)
    }

    pub fn is_omitted(&self) -> bool {
        matches!(self, Self::Omitted)
    }

    pub fn as_ref(&self) -> OptionalField<&T> {
        match self {
            Self::Omitted => OptionalField::Omitted,
            Self::Present(value) => OptionalField::Present(value),
        }
    }
}

impl<T: Serialize> Serialize for OptionalField<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Omitted => serializer.serialize_none(),
            Self::Present(value) => value.serialize(serializer),
        }
    }
}

pub fn is_omitted_field<T>(field: &OptionalField<T>) -> bool {
    field.is_omitted()
}

/// Deserialize a JSON field as `Present(None)` for null, or
/// `Present(Some(value))` for a value. A missing field is supplied by serde's
/// `default` attribute and remains `Omitted`.
pub fn deserialize_optional_field<'de, T, D>(
    deserializer: D,
) -> Result<OptionalField<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(OptionalField::Present(Option::<T>::deserialize(
        deserializer,
    )?))
}

/// Deserialize IPC score fields with the same range error used by the score
/// service. Keeping this at the serde boundary rejects malformed payloads
/// before a database transaction can begin.
pub fn deserialize_optional_company_score<'de, D>(
    deserializer: D,
) -> Result<OptionalField<Option<i64>>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_optional_score(deserializer, COMPANY_STABILITY_RANGE_MESSAGE)
}

pub fn deserialize_optional_employment_score<'de, D>(
    deserializer: D,
) -> Result<OptionalField<Option<i64>>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_optional_score(deserializer, EMPLOYMENT_STABILITY_RANGE_MESSAGE)
}

fn deserialize_optional_score<'de, D>(
    deserializer: D,
    range_message: &'static str,
) -> Result<OptionalField<Option<i64>>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Null => Ok(OptionalField::Present(None)),
        serde_json::Value::Number(number) => number
            .as_i64()
            .map(|score| OptionalField::Present(Some(score)))
            .ok_or_else(|| D::Error::custom(range_message)),
        _ => Err(D::Error::custom(range_message)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScoreSource {
    #[default]
    Manual,
    Import,
}

/// The fields accepted by a company score write. Every score and metadata
/// field is tri-state; `scored_at_override` is reserved for score-file
/// imports and is not itself tri-state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScoreWriteInput {
    #[serde(
        default,
        deserialize_with = "deserialize_optional_company_score",
        skip_serializing_if = "is_omitted_field"
    )]
    pub company_stability: OptionalField<Option<i64>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_employment_score",
        skip_serializing_if = "is_omitted_field"
    )]
    pub employment_stability: OptionalField<Option<i64>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "is_omitted_field"
    )]
    pub company_stability_rationale: OptionalField<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "is_omitted_field"
    )]
    pub company_stability_citations: OptionalField<Option<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "is_omitted_field"
    )]
    pub employment_stability_rationale: OptionalField<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_field",
        skip_serializing_if = "is_omitted_field"
    )]
    pub employment_stability_citations: OptionalField<Option<Vec<String>>>,
    #[serde(default)]
    pub source: ScoreSource,
    #[serde(default)]
    pub scored_at_override: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataValidation {
    Valid,
    /// Importers may discard metadata attached to an unscored result and
    /// surface this as a warning. Manual callers receive an error instead.
    DiscardForImport,
}

/// Normalize a rationale without changing interior whitespace or line breaks.
/// Empty input after trimming is represented as `None`.
pub fn normalize_rationale(input: &str) -> Result<Option<String>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_RATIONALE_CODE_POINTS {
        return Err(format!(
            "Rationale must be at most {MAX_RATIONALE_CODE_POINTS} characters"
        ));
    }
    Ok(Some(trimmed.to_string()))
}

pub fn normalize_optional_rationale(input: Option<&str>) -> Result<Option<String>, String> {
    input.map_or(Ok(None), normalize_rationale)
}

/// Normalize and validate citations. Entries are trimmed in place, preserving
/// order and duplicates. The returned strings are the trimmed input values,
/// not URL re-serializations.
pub fn normalize_citations(citations: &[String]) -> Result<Option<Vec<String>>, String> {
    if citations.len() > MAX_CITATIONS {
        return Err(format!(
            "Citation list must contain at most {MAX_CITATIONS} entries"
        ));
    }

    let mut normalized = Vec::with_capacity(citations.len());
    for (index, citation) in citations.iter().enumerate() {
        let trimmed = citation.trim();
        let valid_length = (1..=MAX_CITATION_CODE_POINTS).contains(&trimmed.chars().count());
        let valid_url = Url::parse(trimmed).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some_and(|host| !host.is_empty())
        });
        if !valid_length || !valid_url {
            return Err(format!("Citation {} is invalid: {:?}", index + 1, trimmed));
        }
        normalized.push(trimmed.to_string());
    }

    Ok((!normalized.is_empty()).then_some(normalized))
}

pub fn normalize_optional_citations(
    citations: Option<&[String]>,
) -> Result<Option<Vec<String>>, String> {
    citations.map_or(Ok(None), normalize_citations)
}

/// Validate a score at the shared service boundary. Database constraints still
/// protect persisted values, but callers need the exact user-facing messages
/// before opening or mutating a transaction.
pub fn validate_company_stability(score: i64) -> Result<i64, &'static str> {
    checked_company_stability(score)
}

pub fn validate_employment_stability(score: i64) -> Result<i64, &'static str> {
    checked_employment_stability(score)
}

/// Enforce the null-score metadata invariant after metadata has been
/// normalized. Import files may intentionally discard such metadata and emit
/// a warning; manual writes must fail atomically instead.
pub fn validate_score_metadata(
    score_name: &str,
    score: Option<i64>,
    rationale: Option<&str>,
    citations: Option<&[String]>,
    source: ScoreSource,
) -> Result<MetadataValidation, String> {
    if score.is_some() || (rationale.is_none() && citations.is_none()) {
        return Ok(MetadataValidation::Valid);
    }

    if source == ScoreSource::Import {
        return Ok(MetadataValidation::DiscardForImport);
    }

    Err(format!(
        "{score_name} must be set before adding rationale or citations"
    ))
}

/// Validate a tri-state score field while retaining the exact distinction
/// between omitted, clear, and set for callers applying a write.
pub fn validate_optional_company_stability(
    field: &OptionalField<Option<i64>>,
) -> Result<(), &'static str> {
    if let OptionalField::Present(Some(score)) = field {
        validate_company_stability(*score)?;
    }
    Ok(())
}

pub fn validate_optional_employment_stability(
    field: &OptionalField<Option<i64>>,
) -> Result<(), &'static str> {
    if let OptionalField::Present(Some(score)) = field {
        validate_employment_stability(*score)?;
    }
    Ok(())
}

/// Validate the persisted representation of both scores. This catches stale
/// metadata even when a caller is reading or constructing a complete company
/// object rather than applying a write.
pub fn validate_company_scores_invariants(scores: &CompanyScores) -> Result<(), String> {
    validate_persisted_score_metadata(
        "Company stability",
        scores.company_stability,
        scores.company_stability_rationale.as_deref(),
        scores.company_stability_citations.as_deref(),
        scores.company_stability_scored_at.as_deref(),
        scores.company_stability_scored_by.as_deref(),
    )?;
    validate_persisted_score_metadata(
        "Employment stability",
        scores.employment_stability,
        scores.employment_stability_rationale.as_deref(),
        scores.employment_stability_citations.as_deref(),
        scores.employment_stability_scored_at.as_deref(),
        scores.employment_stability_scored_by.as_deref(),
    )
}

fn validate_persisted_score_metadata(
    score_name: &str,
    score: Option<i64>,
    rationale: Option<&str>,
    citations: Option<&[String]>,
    scored_at: Option<&str>,
    scored_by: Option<&str>,
) -> Result<(), String> {
    if score.is_some() {
        return Ok(());
    }
    if rationale.is_some() || citations.is_some() || scored_at.is_some() || scored_by.is_some() {
        return Err(format!(
            "{score_name} is null, so its rationale, citations, scored-at, and scored-by must also be null"
        ));
    }
    Ok(())
}

pub fn validate_scored_by(value: Option<&str>) -> Result<(), String> {
    match value {
        None | Some("manual") | Some("import") => Ok(()),
        Some(other) => Err(format!(
            "Scored by must be either 'manual' or 'import', got {other:?}"
        )),
    }
}

/// Convenience conversion used by read paths and tests. It makes the model
/// shape explicit without exposing SQLite's JSON-text representation.
pub fn company_scores_with_citations(
    company_stability: Option<i64>,
    company_stability_rationale: Option<String>,
    company_stability_citations: Option<Vec<String>>,
    company_stability_scored_at: Option<String>,
    company_stability_scored_by: Option<String>,
    employment_stability: Option<i64>,
    employment_stability_rationale: Option<String>,
    employment_stability_citations: Option<Vec<String>>,
    employment_stability_scored_at: Option<String>,
    employment_stability_scored_by: Option<String>,
) -> CompanyScores {
    CompanyScores {
        company_stability,
        company_stability_rationale,
        company_stability_citations,
        company_stability_scored_at,
        company_stability_scored_by,
        employment_stability,
        employment_stability_rationale,
        employment_stability_citations,
        employment_stability_scored_at,
        employment_stability_scored_by,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn company_scores_serialize_as_camel_case_with_null_citations() {
        let scores = CompanyScores {
            company_stability: Some(4),
            company_stability_rationale: Some("well funded".into()),
            company_stability_citations: None,
            company_stability_scored_at: None,
            company_stability_scored_by: Some("manual".into()),
            employment_stability: None,
            employment_stability_rationale: None,
            employment_stability_citations: Some(vec!["https://example.com".into()]),
            employment_stability_scored_at: None,
            employment_stability_scored_by: None,
        };
        let json = serde_json::to_value(scores).unwrap();
        assert_eq!(json["companyStability"], 4);
        assert_eq!(json["companyStabilityCitations"], serde_json::Value::Null);
        assert_eq!(
            json["employmentStabilityCitations"][0],
            "https://example.com"
        );
        assert!(json.get("company_stability").is_none());
    }

    #[test]
    fn score_write_input_distinguishes_missing_null_and_value() {
        let input: ScoreWriteInput =
            serde_json::from_str(r#"{"companyStability":null,"employmentStability":3}"#).unwrap();
        assert_eq!(input.company_stability, OptionalField::Present(None));
        assert_eq!(input.employment_stability, OptionalField::Present(Some(3)));
        assert_eq!(input.company_stability_rationale, OptionalField::Omitted);
        let encoded = serde_json::to_value(&input).unwrap();
        assert_eq!(encoded["companyStability"], serde_json::Value::Null);
        assert_eq!(encoded["employmentStability"], 3);
        assert!(encoded.get("companyStabilityRationale").is_none());
        let error = serde_json::from_str::<ScoreWriteInput>(r#"{"companyStability":"3"}"#)
            .unwrap_err()
            .to_string();
        assert!(error.contains(COMPANY_STABILITY_RANGE_MESSAGE));
        let error = serde_json::from_str::<ScoreWriteInput>(r#"{"employmentStability":2.5}"#)
            .unwrap_err()
            .to_string();
        assert!(error.contains(EMPLOYMENT_STABILITY_RANGE_MESSAGE));
    }

    #[test]
    fn rationale_trims_only_ends_and_counts_unicode_code_points() {
        assert_eq!(
            normalize_rationale("  hello  \n world  ").unwrap(),
            Some("hello  \n world".into())
        );
        assert_eq!(normalize_rationale(" \t\n ").unwrap(), None);
        assert!(normalize_rationale(&"é".repeat(MAX_RATIONALE_CODE_POINTS + 1)).is_err());
        assert!(normalize_rationale(&"é".repeat(MAX_RATIONALE_CODE_POINTS)).is_ok());
    }

    #[test]
    fn citations_trim_preserve_order_duplicates_and_validate_urls() {
        let citations = vec![
            " https://example.com/a ".to_string(),
            "https://example.com/a".to_string(),
        ];
        assert_eq!(
            normalize_citations(&citations).unwrap(),
            Some(vec![
                "https://example.com/a".to_string(),
                "https://example.com/a".to_string()
            ])
        );
        assert_eq!(normalize_citations(&[]).unwrap(), None);
        assert!(normalize_citations(&["ftp://example.com".into()]).is_err());
        assert!(normalize_citations(&["https://".into()]).is_err());
    }

    #[test]
    fn citation_count_limit_precedes_entry_validation() {
        let citations = (0..=MAX_CITATIONS)
            .map(|_| "not a URL".to_string())
            .collect::<Vec<_>>();
        let error = normalize_citations(&citations).unwrap_err();
        assert!(error.contains("20"));
        assert!(!error.contains("Citation 1"));
    }

    #[test]
    fn null_score_metadata_is_rejected_for_manual_and_discarded_for_import() {
        let manual = validate_score_metadata(
            "Company stability",
            None,
            Some("reason"),
            None,
            ScoreSource::Manual,
        )
        .unwrap_err();
        assert!(manual.contains("Company stability"));
        assert!(manual.contains("set before"));
        assert_eq!(
            validate_score_metadata(
                "Employment stability",
                None,
                None,
                Some(&["https://example.com".to_string()]),
                ScoreSource::Import,
            )
            .unwrap(),
            MetadataValidation::DiscardForImport
        );
    }

    #[test]
    fn score_validators_use_exact_range_messages() {
        assert_eq!(
            validate_company_stability(6),
            Err(COMPANY_STABILITY_RANGE_MESSAGE)
        );
        assert_eq!(
            validate_employment_stability(0),
            Err(EMPLOYMENT_STABILITY_RANGE_MESSAGE)
        );
    }
}

/// The result returned by a score write. Warnings are populated when an import
/// intentionally discards metadata that cannot be stored for an unscored
/// result. `changed` describes company-row changes; a carried company score
/// still fans out the cache when the value itself is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreWriteResult {
    #[serde(flatten)]
    pub scores: CompanyScores,
    pub changed: bool,
    pub warnings: Vec<String>,
}

/// Marker type for the shared persistence service. All callers use the same
/// implementation so IPC, CLI, CSV, and score-file imports cannot diverge.
pub struct ScoreService;

#[derive(Debug, Clone)]
struct StoredScore {
    value: Option<i64>,
    rationale: Option<String>,
    citations: Option<Vec<String>>,
    scored_at: Option<String>,
    scored_by: Option<String>,
}

#[derive(Debug, Clone)]
struct ScorePlan {
    carried: bool,
    resulting_value: Option<i64>,
    value_changed: bool,
    rationale: Option<Option<String>>,
    citations: Option<Option<Vec<String>>>,
    metadata_discarded: bool,
    metadata_changed: bool,
}

impl ScoreService {
    /// Apply one company score write atomically inside the caller's
    /// transaction. A savepoint is used as well, which makes direct calls on
    /// a connection atomic and allows the method to nest in a larger job or
    /// importer transaction.
    pub fn write(
        conn: &Connection,
        company_id: &str,
        input: &ScoreWriteInput,
    ) -> AppResult<ScoreWriteResult> {
        write(conn, company_id, input)
    }

    pub fn read(conn: &Connection, company_id: &str) -> AppResult<CompanyScores> {
        read_company_scores(conn, company_id)
    }

    pub fn read_optional(conn: &Connection, company_id: &str) -> AppResult<Option<CompanyScores>> {
        read_company_scores_optional(conn, company_id)
    }
}

/// Apply a score write and return the complete, freshly-read score metadata.
/// The function accepts `&Connection` deliberately: `rusqlite::Transaction`
/// dereferences to this type, so the same function works both standalone and
/// inside a caller-owned transaction.
pub fn write(
    conn: &Connection,
    company_id: &str,
    input: &ScoreWriteInput,
) -> AppResult<ScoreWriteResult> {
    // Validate every input before opening the mutation savepoint. This keeps
    // malformed requests from creating any SQLite side effects at all.
    validate_input(input)?;
    let normalized = normalize_input(input)?;

    conn.execute_batch("SAVEPOINT company_score_write")
        .map_err(map_sqlite)?;
    let result = write_in_savepoint(conn, company_id, input, normalized);
    match result {
        Ok(result) => {
            conn.execute_batch("RELEASE company_score_write")
                .map_err(map_sqlite)?;
            Ok(result)
        }
        Err(error) => {
            let _ =
                conn.execute_batch("ROLLBACK TO company_score_write; RELEASE company_score_write");
            Err(error)
        }
    }
}

/// Alias used by callers that prefer a verb describing the persistence
/// operation explicitly.
pub fn write_company_scores(
    conn: &Connection,
    company_id: &str,
    input: &ScoreWriteInput,
) -> AppResult<ScoreWriteResult> {
    write(conn, company_id, input)
}

/// Read all ten company score fields, decoding citation JSON into arrays.
pub fn read_company_scores(conn: &Connection, company_id: &str) -> AppResult<CompanyScores> {
    conn.query_row(
        "SELECT company_stability, company_stability_rationale,
                company_stability_citations, company_stability_scored_at,
                company_stability_scored_by, employment_stability,
                employment_stability_rationale, employment_stability_citations,
                employment_stability_scored_at, employment_stability_scored_by
         FROM companies WHERE id = ?1",
        params![company_id],
        map_company_scores,
    )
    .optional()
    .map_err(map_sqlite)?
    .ok_or_else(|| AppError::from("Company not found"))
}

/// Optional form of [`read_company_scores`] for list/detail queries where a
/// missing company is a normal absence rather than a user-facing error.
pub fn read_company_scores_optional(
    conn: &Connection,
    company_id: &str,
) -> AppResult<Option<CompanyScores>> {
    conn.query_row(
        "SELECT company_stability, company_stability_rationale,
                company_stability_citations, company_stability_scored_at,
                company_stability_scored_by, employment_stability,
                employment_stability_rationale, employment_stability_citations,
                employment_stability_scored_at, employment_stability_scored_by
         FROM companies WHERE id = ?1",
        params![company_id],
        map_company_scores,
    )
    .optional()
    .map_err(map_sqlite)
}

/// Compatibility alias for read paths that use a `get_*` naming convention.
pub fn get_company_scores(conn: &Connection, company_id: &str) -> AppResult<Option<CompanyScores>> {
    read_company_scores_optional(conn, company_id)
}

fn map_company_scores(row: &rusqlite::Row<'_>) -> rusqlite::Result<CompanyScores> {
    let company_stability: Option<i64> = row.get(0)?;
    let company_stability_rationale: Option<String> = row.get(1)?;
    let company_stability_citations = decode_citations(row.get(2)?)?;
    let company_stability_scored_at: Option<String> = row.get(3)?;
    let company_stability_scored_by: Option<String> = row.get(4)?;
    let employment_stability: Option<i64> = row.get(5)?;
    let employment_stability_rationale: Option<String> = row.get(6)?;
    let employment_stability_citations = decode_citations(row.get(7)?)?;
    let employment_stability_scored_at: Option<String> = row.get(8)?;
    let employment_stability_scored_by: Option<String> = row.get(9)?;

    let scores = company_scores_with_citations(
        company_stability,
        company_stability_rationale,
        company_stability_citations,
        company_stability_scored_at,
        company_stability_scored_by,
        employment_stability,
        employment_stability_rationale,
        employment_stability_citations,
        employment_stability_scored_at,
        employment_stability_scored_by,
    );
    validate_company_scores_invariants(&scores).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    })?;
    Ok(scores)
}

fn decode_citations(raw: Option<String>) -> rusqlite::Result<Option<Vec<String>>> {
    raw.map(|value| {
        serde_json::from_str::<Vec<String>>(&value).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    })
    .transpose()
}

#[derive(Debug, Clone)]
struct NormalizedInput {
    company_rationale: Option<Option<String>>,
    company_citations: Option<Option<Vec<String>>>,
    employment_rationale: Option<Option<String>>,
    employment_citations: Option<Option<Vec<String>>>,
    scored_at_override: Option<String>,
    warnings: Vec<String>,
}

fn validate_input(input: &ScoreWriteInput) -> AppResult<()> {
    validate_optional_company_stability(&input.company_stability).map_err(AppError::from)?;
    validate_optional_employment_stability(&input.employment_stability).map_err(AppError::from)?;
    if input.scored_at_override.is_some() && input.source != ScoreSource::Import {
        return Err(AppError::from(
            "scored_at_override is only valid for import writes",
        ));
    }
    Ok(())
}

fn normalize_input(input: &ScoreWriteInput) -> AppResult<NormalizedInput> {
    let company_rationale = normalize_rationale_field(&input.company_stability_rationale)?;
    let company_citations = normalize_citations_field(&input.company_stability_citations)?;
    let employment_rationale = normalize_rationale_field(&input.employment_stability_rationale)?;
    let employment_citations = normalize_citations_field(&input.employment_stability_citations)?;
    let mut warnings = Vec::new();
    let scored_at_override = input
        .scored_at_override
        .as_deref()
        .map(|value| normalize_scored_at(value, &mut warnings))
        .transpose()?;
    Ok(NormalizedInput {
        company_rationale,
        company_citations,
        employment_rationale,
        employment_citations,
        scored_at_override,
        warnings,
    })
}

fn normalize_rationale_field(
    field: &OptionalField<Option<String>>,
) -> AppResult<Option<Option<String>>> {
    match field {
        OptionalField::Omitted => Ok(None),
        OptionalField::Present(value) => normalize_optional_rationale(value.as_deref())
            .map(Some)
            .map_err(AppError::from),
    }
}

fn normalize_citations_field(
    field: &OptionalField<Option<Vec<String>>>,
) -> AppResult<Option<Option<Vec<String>>>> {
    match field {
        OptionalField::Omitted => Ok(None),
        OptionalField::Present(value) => normalize_optional_citations(value.as_deref())
            .map(Some)
            .map_err(AppError::from),
    }
}

fn normalize_scored_at(value: &str, warnings: &mut Vec<String>) -> AppResult<String> {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(parsed
            .with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    }
    if let Ok(parsed) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Ok(parsed
            .and_hms_opt(0, 0, 0)
            .expect("midnight is valid")
            .and_utc()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    }
    warnings.push(format!(
        "Invalid as_of/scoredAt {:?}; used the import time instead",
        value
    ));
    Ok(now_iso())
}

fn write_in_savepoint(
    conn: &Connection,
    company_id: &str,
    input: &ScoreWriteInput,
    normalized: NormalizedInput,
) -> AppResult<ScoreWriteResult> {
    let current = read_company_scores(conn, company_id)?;
    let current_company = stored_score_from_values(
        current.company_stability,
        current.company_stability_rationale.clone(),
        current.company_stability_citations.clone(),
        current.company_stability_scored_at.clone(),
        current.company_stability_scored_by.clone(),
    );
    let current_employment = stored_score_from_values(
        current.employment_stability,
        current.employment_stability_rationale.clone(),
        current.employment_stability_citations.clone(),
        current.employment_stability_scored_at.clone(),
        current.employment_stability_scored_by.clone(),
    );

    let company_plan = build_score_plan(
        "Company stability",
        &input.company_stability,
        current_company,
        normalized.company_rationale,
        normalized.company_citations,
        input.source,
    )?;
    let employment_plan = build_score_plan(
        "Employment stability",
        &input.employment_stability,
        current_employment,
        normalized.employment_rationale,
        normalized.employment_citations,
        input.source,
    )?;

    let mut warnings = normalized.warnings;
    if company_plan.metadata_discarded {
        warnings.push(
            "Discarded company stability rationale/citations because the imported score is null"
                .to_string(),
        );
    }
    if employment_plan.metadata_discarded {
        warnings.push(
            "Discarded employment stability rationale/citations because the imported score is null"
                .to_string(),
        );
    }

    let score_timestamp = if company_plan.value_changed || employment_plan.value_changed {
        Some(
            normalized
                .scored_at_override
                .clone()
                .unwrap_or_else(now_iso),
        )
    } else {
        None
    };

    let changed = company_plan.value_changed
        || employment_plan.value_changed
        || company_plan.metadata_changed
        || employment_plan.metadata_changed;
    apply_company_changes(
        conn,
        company_id,
        input,
        &company_plan,
        &employment_plan,
        score_timestamp.as_deref(),
    )?;

    // A carried company score always rewrites the compatibility cache, even
    // when the value equals the stored value or only metadata changed.
    if company_plan.carried {
        conn.execute(
            "UPDATE jobs SET company_stability = ?1 WHERE company_id = ?2",
            params![company_plan.resulting_value, company_id],
        )
        .map_err(map_sqlite)?;
    }

    let scores = read_company_scores(conn, company_id)?;
    Ok(ScoreWriteResult {
        scores,
        changed,
        warnings,
    })
}

fn stored_score_from_values(
    value: Option<i64>,
    rationale: Option<String>,
    citations: Option<Vec<String>>,
    scored_at: Option<String>,
    scored_by: Option<String>,
) -> StoredScore {
    StoredScore {
        value,
        rationale,
        citations,
        scored_at,
        scored_by,
    }
}

fn build_score_plan(
    score_name: &str,
    score_field: &OptionalField<Option<i64>>,
    current: StoredScore,
    rationale: Option<Option<String>>,
    citations: Option<Option<Vec<String>>>,
    source: ScoreSource,
) -> AppResult<ScorePlan> {
    let carried = !score_field.is_omitted();
    let resulting_value = match score_field {
        OptionalField::Omitted => current.value,
        OptionalField::Present(value) => *value,
    };
    let metadata_discarded = validate_score_metadata(
        score_name,
        resulting_value,
        supplied_rationale_ref(&rationale),
        supplied_citations_ref(&citations),
        source,
    )? == MetadataValidation::DiscardForImport;

    let retained_rationale = if metadata_discarded {
        None
    } else {
        rationale.clone()
    };
    let retained_citations = if metadata_discarded {
        None
    } else {
        citations.clone()
    };
    let value_changed = resulting_value != current.value;
    let clear = resulting_value.is_none() && (value_changed || carried);
    let metadata_changed = if clear {
        current.rationale.is_some()
            || current.citations.is_some()
            || current.scored_at.is_some()
            || current.scored_by.is_some()
    } else {
        supplied_metadata_differs(&retained_rationale, &retained_citations, &current)
    };

    Ok(ScorePlan {
        carried,
        resulting_value,
        value_changed,
        rationale: retained_rationale,
        citations: retained_citations,
        metadata_discarded,
        metadata_changed,
    })
}

fn supplied_rationale_ref<'a>(field: &'a Option<Option<String>>) -> Option<&'a str> {
    field.as_ref().and_then(|value| value.as_deref())
}

fn supplied_citations_ref<'a>(field: &'a Option<Option<Vec<String>>>) -> Option<&'a [String]> {
    field.as_ref().and_then(|value| value.as_deref())
}

fn supplied_metadata_differs(
    rationale: &Option<Option<String>>,
    citations: &Option<Option<Vec<String>>>,
    current: &StoredScore,
) -> bool {
    rationale
        .as_ref()
        .is_some_and(|value| value.as_ref() != current.rationale.as_ref())
        || citations
            .as_ref()
            .is_some_and(|value| value.as_ref() != current.citations.as_ref())
}

fn apply_company_changes(
    conn: &Connection,
    company_id: &str,
    input: &ScoreWriteInput,
    company: &ScorePlan,
    employment: &ScorePlan,
    score_timestamp: Option<&str>,
) -> AppResult<()> {
    let mut assignments = Vec::new();
    let mut values = Vec::new();
    append_score_changes(
        "company_stability",
        "company_stability_rationale",
        "company_stability_citations",
        "company_stability_scored_at",
        "company_stability_scored_by",
        company,
        input.source,
        score_timestamp,
        &mut assignments,
        &mut values,
    );
    append_score_changes(
        "employment_stability",
        "employment_stability_rationale",
        "employment_stability_citations",
        "employment_stability_scored_at",
        "employment_stability_scored_by",
        employment,
        input.source,
        score_timestamp,
        &mut assignments,
        &mut values,
    );
    if assignments.is_empty() {
        return Ok(());
    }

    values.push(Value::Text(company_id.to_string()));
    let sql = format!(
        "UPDATE companies SET {} WHERE id = ?{}",
        assignments.join(", "),
        values.len()
    );
    conn.execute(&sql, params_from_iter(values))
        .map_err(map_sqlite)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_score_changes(
    score_column: &str,
    rationale_column: &str,
    citations_column: &str,
    scored_at_column: &str,
    scored_by_column: &str,
    plan: &ScorePlan,
    source: ScoreSource,
    score_timestamp: Option<&str>,
    assignments: &mut Vec<String>,
    values: &mut Vec<Value>,
) {
    if !plan.value_changed && !plan.metadata_changed {
        return;
    }

    if plan.value_changed {
        push_assignment(assignments, values, score_column, plan.resulting_value);
        if plan.resulting_value.is_none() {
            push_assignment(assignments, values, rationale_column, None::<String>);
            push_assignment(assignments, values, citations_column, None::<String>);
            push_assignment(assignments, values, scored_at_column, None::<String>);
            push_assignment(assignments, values, scored_by_column, None::<String>);
        } else {
            if let Some(rationale) = &plan.rationale {
                push_assignment(assignments, values, rationale_column, rationale.clone());
            }
            if let Some(citations) = &plan.citations {
                push_assignment(
                    assignments,
                    values,
                    citations_column,
                    citations
                        .as_ref()
                        .map(|value| serde_json::to_string(value).expect("citation serialization")),
                );
            }
            push_assignment(
                assignments,
                values,
                scored_at_column,
                score_timestamp.map(str::to_string),
            );
            push_assignment(
                assignments,
                values,
                scored_by_column,
                Some(match source {
                    ScoreSource::Manual => "manual".to_string(),
                    ScoreSource::Import => "import".to_string(),
                }),
            );
        }
        return;
    }

    if let Some(rationale) = &plan.rationale {
        push_assignment(assignments, values, rationale_column, rationale.clone());
    }
    if let Some(citations) = &plan.citations {
        push_assignment(
            assignments,
            values,
            citations_column,
            citations
                .as_ref()
                .map(|value| serde_json::to_string(value).expect("citation serialization")),
        );
    }
}

fn push_assignment<T>(
    assignments: &mut Vec<String>,
    values: &mut Vec<Value>,
    column: &str,
    value: Option<T>,
) where
    T: Into<Value>,
{
    assignments.push(format!("{column} = ?{}", values.len() + 1));
    values.push(value.map_or(Value::Null, Into::into));
}

#[cfg(test)]
mod service_tests {
    use super::*;
    use crate::db::migrate::migrate;
    use crate::util::now_iso;
    use rusqlite::Connection;

    fn test_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn insert_company(conn: &Connection, id: &str, name: &str) {
        let timestamp = now_iso();
        conn.execute(
            "INSERT INTO companies (id, name, careers_url, created_at, updated_at)
             VALUES (?1, ?2, NULL, ?3, ?3)",
            params![id, name, timestamp],
        )
        .unwrap();
    }

    fn insert_job(conn: &Connection, id: &str, company_id: &str, stability: Option<i64>) {
        let timestamp = now_iso();
        conn.execute(
            "INSERT INTO jobs
             (id, company_id, title, url, canonical_url, status, posting_state, source,
              created_at, updated_at, appeal, application_ease, company_stability)
             VALUES (?1, ?2, 'Role', ?3, ?3, 'wishlist', 'unknown', 'manual', ?4, ?4, 2, 4, ?5)",
            params![
                id,
                company_id,
                format!("https://example.com/{id}"),
                timestamp,
                stability
            ],
        )
        .unwrap();
    }

    #[test]
    fn writes_both_scores_with_shared_timestamp_and_reads_complete_metadata() {
        let conn = test_connection();
        insert_company(&conn, "c1", "Acme");
        let input = ScoreWriteInput {
            company_stability: OptionalField::present(Some(4)),
            employment_stability: OptionalField::present(Some(3)),
            company_stability_rationale: OptionalField::present(Some("  cash flow  ".into())),
            company_stability_citations: OptionalField::present(Some(vec![
                " https://example.com/stability ".into(),
            ])),
            employment_stability_rationale: OptionalField::present(Some("steady team".into())),
            employment_stability_citations: OptionalField::present(Some(vec![
                "https://example.com/layoffs".into(),
            ])),
            source: ScoreSource::Manual,
            scored_at_override: None,
        };

        let result = write(&conn, "c1", &input).unwrap();
        assert!(result.changed);
        assert_eq!(result.scores.company_stability, Some(4));
        assert_eq!(
            result.scores.company_stability_rationale.as_deref(),
            Some("cash flow")
        );
        assert_eq!(
            result.scores.company_stability_citations,
            Some(vec!["https://example.com/stability".into()])
        );
        assert_eq!(
            result.scores.company_stability_scored_at,
            result.scores.employment_stability_scored_at
        );
        assert_eq!(read_company_scores(&conn, "c1").unwrap(), result.scores);
    }

    #[test]
    fn carried_company_score_fans_out_only_the_cache_and_preserves_other_job_fields() {
        let conn = test_connection();
        insert_company(&conn, "c1", "Acme");
        insert_company(&conn, "c2", "Other");
        insert_job(&conn, "j1", "c1", Some(1));
        insert_job(&conn, "j2", "c1", Some(1));
        insert_job(&conn, "j3", "c2", Some(5));
        conn.execute(
            "INSERT INTO job_events (id, job_id, type, note, occurred_at)
             VALUES ('e1', 'j1', 'note_added', 'keep', '2026-01-01T00:00:00.000Z')",
            [],
        )
        .unwrap();
        let before: (String, i64, i64, i64) = conn
            .query_row(
                "SELECT updated_at, appeal, application_ease, is_favorite FROM jobs WHERE id = 'j1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        let input = ScoreWriteInput {
            company_stability: OptionalField::present(Some(4)),
            ..Default::default()
        };
        write(&conn, "c1", &input).unwrap();

        let cache_values: Vec<(String, Option<i64>)> = conn
            .prepare("SELECT id, company_stability FROM jobs ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            cache_values,
            vec![
                ("j1".into(), Some(4)),
                ("j2".into(), Some(4)),
                ("j3".into(), Some(5))
            ]
        );
        let after: (String, i64, i64, i64) = conn
            .query_row(
                "SELECT updated_at, appeal, application_ease, is_favorite FROM jobs WHERE id = 'j1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(after, before);
        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM job_events WHERE job_id = 'j1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(event_count, 1);
    }

    #[test]
    fn metadata_only_writes_preserve_score_timestamp_and_manual_null_metadata_is_atomic() {
        let conn = test_connection();
        insert_company(&conn, "c1", "Acme");
        let set = ScoreWriteInput {
            company_stability: OptionalField::present(Some(3)),
            company_stability_rationale: OptionalField::present(Some("first".into())),
            ..Default::default()
        };
        let first = write(&conn, "c1", &set).unwrap().scores;
        let update = ScoreWriteInput {
            company_stability_rationale: OptionalField::present(Some("second".into())),
            ..Default::default()
        };
        let second = write(&conn, "c1", &update).unwrap().scores;
        assert_eq!(second.company_stability, Some(3));
        assert_eq!(
            second.company_stability_rationale.as_deref(),
            Some("second")
        );
        assert_eq!(
            second.company_stability_scored_at,
            first.company_stability_scored_at
        );
        assert_eq!(
            second.company_stability_scored_by,
            first.company_stability_scored_by
        );

        let invalid = ScoreWriteInput {
            company_stability: OptionalField::present(None),
            company_stability_rationale: OptionalField::present(Some("stale".into())),
            ..Default::default()
        };
        let error = write(&conn, "c1", &invalid).unwrap_err().to_string();
        assert!(error.contains("Company stability"));
        assert_eq!(read_company_scores(&conn, "c1").unwrap(), second);
    }

    #[test]
    fn import_discards_metadata_for_null_score_and_supports_as_of() {
        let conn = test_connection();
        insert_company(&conn, "c1", "Acme");
        let input = ScoreWriteInput {
            company_stability: OptionalField::present(None),
            company_stability_rationale: OptionalField::present(Some("discard me".into())),
            source: ScoreSource::Import,
            scored_at_override: Some("2026-11-02".into()),
            ..Default::default()
        };
        let result = write(&conn, "c1", &input).unwrap();
        assert!(!result.changed);
        assert_eq!(result.scores.company_stability, None);
        assert_eq!(result.warnings.len(), 1);

        let set = ScoreWriteInput {
            company_stability: OptionalField::present(Some(5)),
            source: ScoreSource::Import,
            scored_at_override: Some("2026-11-02".into()),
            ..Default::default()
        };
        let result = write(&conn, "c1", &set).unwrap();
        assert_eq!(
            result.scores.company_stability_scored_at.as_deref(),
            Some("2026-11-02T00:00:00.000Z")
        );
        assert_eq!(
            result.scores.company_stability_scored_by.as_deref(),
            Some("import")
        );
    }
}

#[cfg(test)]
mod task_14_tests {
    use super::*;
    use proptest::prelude::*;
    use rusqlite::Connection;

    fn test_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE companies (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                company_stability INTEGER CHECK (company_stability IS NULL OR (typeof(company_stability) = 'integer' AND company_stability BETWEEN 1 AND 5)),
                employment_stability INTEGER CHECK (employment_stability IS NULL OR (typeof(employment_stability) = 'integer' AND employment_stability BETWEEN 1 AND 5)),
                company_stability_rationale TEXT,
                company_stability_citations TEXT,
                company_stability_scored_at TEXT,
                company_stability_scored_by TEXT CHECK (company_stability_scored_by IS NULL OR company_stability_scored_by IN ('manual','import')),
                employment_stability_rationale TEXT,
                employment_stability_citations TEXT,
                employment_stability_scored_at TEXT,
                employment_stability_scored_by TEXT CHECK (employment_stability_scored_by IS NULL OR employment_stability_scored_by IN ('manual','import'))
             );
             CREATE TABLE jobs (
                id TEXT PRIMARY KEY NOT NULL,
                company_id TEXT NOT NULL,
                company_stability INTEGER CHECK (company_stability IS NULL OR (typeof(company_stability) = 'integer' AND company_stability BETWEEN 1 AND 5))
             );
             INSERT INTO companies (id, name) VALUES ('c1', 'Acme');",
        )
        .unwrap();
        conn
    }

    fn rationale_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(any::<char>(), 0..=32).prop_map(|chars| chars.into_iter().collect())
    }

    // Feature: company-employment-stability, Property 2: Company score writes preserve strict domain and metadata invariants
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_score_writes_reject_invalid_values_and_normalize_valid_metadata(
            score in any::<i64>(),
            rationale in rationale_strategy(),
            citation_count in 0usize..=20,
            source in prop::sample::select(vec![ScoreSource::Manual, ScoreSource::Import]),
        ) {
            let conn = test_connection();
            let citations = (0..citation_count)
                .map(|index| format!(" https://example.com/source/{index} "))
                .collect::<Vec<_>>();
            let input = ScoreWriteInput {
                company_stability: OptionalField::present(Some(score)),
                company_stability_rationale: OptionalField::present(Some(rationale.clone())),
                company_stability_citations: OptionalField::present(Some(citations.clone())),
                source,
                ..Default::default()
            };

            if !(1..=5).contains(&score) {
                prop_assert!(write(&conn, "c1", &input).is_err());
                let persisted: (Option<i64>, Option<String>, Option<String>) = conn.query_row(
                    "SELECT company_stability, company_stability_rationale, company_stability_citations FROM companies WHERE id = 'c1'",
                    [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                ).unwrap();
                prop_assert_eq!(persisted, (None, None, None));
            } else {
                let result = write(&conn, "c1", &input).unwrap();
                prop_assert_eq!(result.scores.company_stability, Some(score));
                prop_assert_eq!(&result.scores.company_stability_rationale, &normalize_rationale(&rationale).unwrap());
                let expected_citations = normalize_citations(&citations).unwrap();
                prop_assert_eq!(result.scores.company_stability_citations.as_ref(), expected_citations.as_ref());
                prop_assert!(validate_company_scores_invariants(&result.scores).is_ok());
            }
        }
    }

    #[test]
    fn invalid_score_and_source_writes_are_atomic_at_the_service_boundary() {
        let conn = test_connection();
        let initial = ScoreWriteInput {
            company_stability: OptionalField::present(Some(3)),
            employment_stability: OptionalField::present(Some(4)),
            company_stability_rationale: OptionalField::present(Some("keep".into())),
            source: ScoreSource::Manual,
            ..Default::default()
        };
        let before = write(&conn, "c1", &initial).unwrap().scores;
        assert!(validate_scored_by(Some("csv")).is_err());
        let invalid = ScoreWriteInput {
            company_stability: OptionalField::present(Some(6)),
            employment_stability: OptionalField::present(Some(0)),
            company_stability_rationale: OptionalField::present(Some("changed".into())),
            ..Default::default()
        };
        assert!(write(&conn, "c1", &invalid).is_err());
        assert_eq!(read_company_scores(&conn, "c1").unwrap(), before);
    }
}

#[cfg(test)]
mod task_24_score_properties {
    use super::*;
    use proptest::prelude::*;
    use rusqlite::Connection;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE companies (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                company_stability INTEGER CHECK (company_stability IS NULL OR (typeof(company_stability) = 'integer' AND company_stability BETWEEN 1 AND 5)),
                employment_stability INTEGER CHECK (employment_stability IS NULL OR (typeof(employment_stability) = 'integer' AND employment_stability BETWEEN 1 AND 5)),
                company_stability_rationale TEXT,
                company_stability_citations TEXT,
                company_stability_scored_at TEXT,
                company_stability_scored_by TEXT CHECK (company_stability_scored_by IS NULL OR company_stability_scored_by IN ('manual','import')),
                employment_stability_rationale TEXT,
                employment_stability_citations TEXT,
                employment_stability_scored_at TEXT,
                employment_stability_scored_by TEXT CHECK (employment_stability_scored_by IS NULL OR employment_stability_scored_by IN ('manual','import'))
            );
            CREATE TABLE jobs (id TEXT PRIMARY KEY NOT NULL, company_id TEXT NOT NULL, company_stability INTEGER);
            CREATE TABLE job_events (id TEXT PRIMARY KEY NOT NULL, job_id TEXT NOT NULL, type TEXT NOT NULL, note TEXT, occurred_at TEXT NOT NULL);
            INSERT INTO companies (id, name) VALUES ('c1', 'Acme');",
        )
        .unwrap();
        conn
    }

    fn citations(count: usize) -> Vec<String> {
        (0..count)
            .map(|index| format!(" https://example.com/source/{index} "))
            .collect()
    }

    // Feature: company-employment-stability, Property 3: Score metadata round-trips without unintended normalization
    // Validates: Requirements 3.1, 3.3, 3.6, 3.7, 3.8, 3.9, 3.10, 3.11, 3.21
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_metadata_round_trip_preserves_score_timestamp_and_ordered_citations(
            rationale in prop::collection::vec(any::<char>(), 0..=64),
            citation_count in 0usize..=20,
        ) {
            let conn = connection();
            let first = write(&conn, "c1", &ScoreWriteInput {
                company_stability: OptionalField::present(Some(3)),
                company_stability_rationale: OptionalField::present(Some("initial".to_string())),
                company_stability_citations: OptionalField::present(Some(citations(1))),
                ..Default::default()
            }).unwrap().scores;
            let raw_rationale: String = rationale.into_iter().collect();
            let raw_citations = citations(citation_count);
            let expected_rationale = normalize_rationale(&raw_rationale).unwrap();
            let expected_citations = normalize_citations(&raw_citations).unwrap();
            let second = write(&conn, "c1", &ScoreWriteInput {
                company_stability_rationale: OptionalField::present(Some(raw_rationale)),
                company_stability_citations: OptionalField::present(Some(raw_citations)),
                ..Default::default()
            }).unwrap().scores;

            prop_assert_eq!(&second.company_stability, &Some(3));
            prop_assert_eq!(&second.company_stability_rationale, &expected_rationale);
            prop_assert_eq!(&second.company_stability_citations, &expected_citations);
            prop_assert_eq!(&second.company_stability_scored_at, &first.company_stability_scored_at);
            prop_assert_eq!(&second.company_stability_scored_by, &first.company_stability_scored_by);
            prop_assert!(validate_company_scores_invariants(&second).is_ok());
        }
    }

    #[test]
    fn clear_semantics_clear_score_metadata_and_an_unchanged_carried_score_keeps_timestamp() {
        let conn = connection();
        let initial = write(
            &conn,
            "c1",
            &ScoreWriteInput {
                company_stability: OptionalField::present(Some(4)),
                employment_stability: OptionalField::present(Some(2)),
                company_stability_rationale: OptionalField::present(Some("cash".into())),
                employment_stability_rationale: OptionalField::present(Some("cuts".into())),
                ..Default::default()
            },
        )
        .unwrap()
        .scores;
        let unchanged = write(
            &conn,
            "c1",
            &ScoreWriteInput {
                company_stability: OptionalField::present(Some(4)),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!unchanged.changed);
        assert_eq!(
            unchanged.scores.company_stability_scored_at,
            initial.company_stability_scored_at
        );
        assert_eq!(
            unchanged.scores.company_stability_scored_by,
            initial.company_stability_scored_by
        );

        let cleared = write(
            &conn,
            "c1",
            &ScoreWriteInput {
                company_stability: OptionalField::present(None),
                employment_stability: OptionalField::present(None),
                ..Default::default()
            },
        )
        .unwrap()
        .scores;
        assert_eq!(cleared, CompanyScores::default());
    }
}
