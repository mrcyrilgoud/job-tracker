use scraper::{Html, Selector};
use serde::Serialize;
use serde_json::Value;

use crate::error::AppResult;
use crate::jobs::board_discovery::{discover_from_url, UrlDiscovery};
use crate::jobs::safe_fetch::safe_fetch;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobMetadata {
    pub title: Option<String>,
    pub company_name: Option<String>,
    #[serde(flatten)]
    pub url_discovery: UrlDiscovery,
}

pub async fn resolve_job_metadata(url: &str) -> AppResult<JobMetadata> {
    let original_discovery = discover_from_url(url)?;
    let fetched = safe_fetch(url, Some("GET"), Some("text/html,application/xhtml+xml")).await;
    if !fetched.ok {
        return Ok(JobMetadata {
            url_discovery: original_discovery,
            ..JobMetadata::default()
        });
    }

    let final_discovery = discover_from_url(&fetched.final_url).unwrap_or_default();
    let mut metadata = extract_job_metadata(&fetched.body_text);
    metadata.url_discovery = if final_discovery != UrlDiscovery::default() {
        final_discovery
    } else {
        original_discovery
    };
    Ok(metadata)
}

pub fn extract_job_metadata(html: &str) -> JobMetadata {
    let document = Html::parse_document(html);
    let mut metadata = extract_json_ld(&document).unwrap_or_default();

    if metadata.title.is_none() {
        metadata.title = meta_content(&document, "property", "og:title")
            .or_else(|| meta_content(&document, "name", "twitter:title"))
            .or_else(|| html_title(&document));
    }

    if metadata.company_name.is_none() {
        metadata.company_name =
            meta_content(&document, "property", "og:site_name").filter(|name| !is_ats_name(name));
    }

    metadata
}

fn extract_json_ld(document: &Html) -> Option<JobMetadata> {
    let selector = Selector::parse("script[type]").ok()?;
    for element in document.select(&selector) {
        let is_json_ld = element
            .value()
            .attr("type")
            .is_some_and(|value| value.eq_ignore_ascii_case("application/ld+json"));
        if !is_json_ld {
            continue;
        }

        let source = element.text().collect::<String>();
        let Ok(value) = serde_json::from_str::<Value>(&source) else {
            continue;
        };
        if let Some(posting) = find_job_posting(&value) {
            return Some(JobMetadata {
                title: string_field(posting, "title"),
                company_name: company_name(posting),
                url_discovery: UrlDiscovery::default(),
            });
        }
    }
    None
}

fn find_job_posting(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    match value {
        Value::Object(object) => {
            if object
                .get("@type")
                .is_some_and(|value| schema_type_is(value, "JobPosting"))
            {
                return Some(object);
            }

            if let Some(found) = object.get("@graph").and_then(find_job_posting) {
                return Some(found);
            }

            object.values().find_map(find_job_posting)
        }
        Value::Array(items) => items.iter().find_map(find_job_posting),
        _ => None,
    }
}

fn schema_type_is(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(value) => value.eq_ignore_ascii_case(expected),
        Value::Array(values) => values.iter().any(|value| schema_type_is(value, expected)),
        _ => false,
    }
}

fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).and_then(normalize)
}

fn company_name(posting: &serde_json::Map<String, Value>) -> Option<String> {
    let organization = posting.get("hiringOrganization")?;
    match organization {
        Value::Object(object) => string_field(object, "name"),
        Value::Array(items) => items.iter().find_map(|item| {
            item.as_object()
                .and_then(|object| string_field(object, "name"))
        }),
        _ => None,
    }
}

fn meta_content(document: &Html, attribute: &str, expected: &str) -> Option<String> {
    let selector = Selector::parse("meta").ok()?;
    document.select(&selector).find_map(|element| {
        let matches = element
            .value()
            .attr(attribute)
            .is_some_and(|value| value.eq_ignore_ascii_case(expected));
        matches
            .then(|| element.value().attr("content"))
            .flatten()
            .and_then(normalize)
    })
}

fn html_title(document: &Html) -> Option<String> {
    let selector = Selector::parse("title").ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|element| normalize(&element.text().collect::<String>()))
}

fn normalize(value: &str) -> Option<String> {
    let decoded = html_escape::decode_html_entities(value);
    let normalized = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn is_ats_name(name: &str) -> bool {
    const ATS_NAMES: &[&str] = &[
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

    let normalized = name.to_ascii_lowercase();
    ATS_NAMES.iter().any(|ats| normalized.contains(ats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::board_discovery::DetectedBoard;

    #[test]
    fn extracts_job_posting_fields() {
        let html = r#"
            <script type="application/ld+json">
              {"@type":"JobPosting","title":" Senior  Engineer ","hiringOrganization":{"name":"Acme &amp; Co"}}
            </script>
        "#;

        assert_eq!(
            extract_job_metadata(html),
            JobMetadata {
                title: Some("Senior Engineer".into()),
                company_name: Some("Acme & Co".into()),
                ..JobMetadata::default()
            }
        );
    }

    #[test]
    fn finds_job_posting_in_array_and_graph() {
        let array = r#"
            <script type="application/ld+json">
              [{"@type":"WebSite"},{"@type":"JobPosting","title":"Array Role","hiringOrganization":{"name":"Array Co"}}]
            </script>
        "#;
        let graph = r#"
            <script type="application/ld+json">
              {"@context":"https://schema.org","@graph":[{"@type":"Organization","name":"Other"},{"@type":"JobPosting","title":"Graph Role","hiringOrganization":[{"name":"Graph Co"}]}]}
            </script>
        "#;

        assert_eq!(
            extract_job_metadata(array),
            JobMetadata {
                title: Some("Array Role".into()),
                company_name: Some("Array Co".into()),
                ..JobMetadata::default()
            }
        );
        assert_eq!(
            extract_job_metadata(graph),
            JobMetadata {
                title: Some("Graph Role".into()),
                company_name: Some("Graph Co".into()),
                ..JobMetadata::default()
            }
        );
    }

    #[test]
    fn skips_unrelated_json_ld_blocks() {
        let html = r#"
            <script type="application/ld+json">{"@type":"WebSite","name":"Wrong"}</script>
            <script type="application/ld+json">{"@type":"JobPosting","title":"Right","hiringOrganization":{"name":"Employer"}}</script>
        "#;

        assert_eq!(
            extract_job_metadata(html),
            JobMetadata {
                title: Some("Right".into()),
                company_name: Some("Employer".into()),
                ..JobMetadata::default()
            }
        );
    }

    #[test]
    fn malformed_json_ld_uses_metadata_fallback_order() {
        let html = r#"
            <script type="application/ld+json">{not-json}</script>
            <meta property="og:title" content=" Open &amp; Graph ">
            <meta name="twitter:title" content="Twitter">
            <title>Document</title>
        "#;
        assert_eq!(
            extract_job_metadata(html).title.as_deref(),
            Some("Open & Graph")
        );

        let twitter = r#"<meta name="twitter:title" content="Twitter"><title>Document</title>"#;
        assert_eq!(
            extract_job_metadata(twitter).title.as_deref(),
            Some("Twitter")
        );
        assert_eq!(
            extract_job_metadata("<title> Document   Title </title>")
                .title
                .as_deref(),
            Some("Document Title")
        );
    }

    #[test]
    fn returns_partial_and_empty_results_without_guessing() {
        let title_only =
            r#"<script type="application/ld+json">{"@type":"JobPosting","title":"Role"}</script>"#;
        let company_only = r#"<script type="application/ld+json">{"@type":"JobPosting","hiringOrganization":{"name":"Acme"}}</script>"#;

        assert_eq!(
            extract_job_metadata(title_only),
            JobMetadata {
                title: Some("Role".into()),
                company_name: None,
                ..JobMetadata::default()
            }
        );
        assert_eq!(
            extract_job_metadata(company_only),
            JobMetadata {
                title: None,
                company_name: Some("Acme".into()),
                ..JobMetadata::default()
            }
        );
        assert_eq!(
            extract_job_metadata("<html></html>"),
            JobMetadata::default()
        );
    }

    #[test]
    fn rejects_ats_site_name_as_company() {
        let ats = r#"<meta property="og:site_name" content="LinkedIn Jobs">"#;
        let employer = r#"<meta property="og:site_name" content="Acme Corporation">"#;

        assert_eq!(extract_job_metadata(ats).company_name, None);
        assert_eq!(
            extract_job_metadata(employer).company_name.as_deref(),
            Some("Acme Corporation")
        );
    }

    #[test]
    fn serializes_command_response_as_camel_case() {
        let value = serde_json::to_value(JobMetadata {
            title: Some("Role".into()),
            company_name: None,
            ..JobMetadata::default()
        })
        .unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "title": "Role",
                "companyName": null,
                "board": null,
                "careersUrl": null
            })
        );
    }

    #[test]
    fn serializes_detected_board_and_careers_fallback_at_preview_top_level() {
        let board = serde_json::to_value(JobMetadata {
            title: Some("Role".into()),
            company_name: Some("Acme".into()),
            url_discovery: UrlDiscovery {
                board: Some(DetectedBoard {
                    provider: "greenhouse".into(),
                    board_slug: "acme".into(),
                    board_url: "https://job-boards.greenhouse.io/acme".into(),
                    posting_id: Some("123".into()),
                }),
                careers_url: None,
            },
        })
        .unwrap();
        assert_eq!(
            board,
            serde_json::json!({
                "title": "Role",
                "companyName": "Acme",
                "board": {
                    "provider": "greenhouse",
                    "boardSlug": "acme",
                    "boardUrl": "https://job-boards.greenhouse.io/acme",
                    "postingId": "123"
                },
                "careersUrl": null
            })
        );

        let careers = serde_json::to_value(JobMetadata {
            title: None,
            company_name: None,
            url_discovery: UrlDiscovery {
                board: None,
                careers_url: Some("https://acme.example/careers".into()),
            },
        })
        .unwrap();
        assert_eq!(
            careers,
            serde_json::json!({
                "title": null,
                "companyName": null,
                "board": null,
                "careersUrl": "https://acme.example/careers"
            })
        );
    }
}
