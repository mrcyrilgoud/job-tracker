use serde::{Deserialize, Serialize};
use url::Url;
use urlencoding::decode;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedBoard {
    pub provider: String,
    pub board_slug: String,
    pub board_url: String,
    pub posting_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlDiscovery {
    pub board: Option<DetectedBoard>,
    pub careers_url: Option<String>,
}

/// Classifies a posting URL without fetching it or making any other network request.
pub fn discover_from_url(raw_url: &str) -> AppResult<UrlDiscovery> {
    let url = parse_http_url(raw_url)?;
    let host = url
        .host_str()
        .ok_or_else(|| AppError::from("URL is missing a host"))?
        .to_ascii_lowercase();
    let path_segments = normalized_path_segments(&url)?;

    let discovery = match host.as_str() {
        "jobs.ashbyhq.com" => discover_ashby(&url, &path_segments)?,
        "boards.greenhouse.io" | "job-boards.greenhouse.io" => {
            discover_greenhouse(&url, &host, &path_segments)?
        }
        "jobs.lever.co" | "jobs.eu.lever.co" => discover_lever(&url, &host, &path_segments)?,
        _ => UrlDiscovery {
            board: None,
            careers_url: careers_root(&url, &host, &path_segments),
        },
    };

    Ok(discovery)
}

fn parse_http_url(raw_url: &str) -> AppResult<Url> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return Err(AppError::from("URL cannot be empty"));
    }

    let url = Url::parse(trimmed).map_err(|error| AppError::from(error.to_string()))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(AppError::from("Only HTTP and HTTPS URLs are supported"));
    }
    if url.host_str().is_none() {
        return Err(AppError::from("URL is missing a host"));
    }
    Ok(url)
}

fn normalized_path_segments(url: &Url) -> AppResult<Vec<String>> {
    let path = url.path().trim_matches('/');
    if path.is_empty() {
        return Ok(Vec::new());
    }

    path.split('/')
        .map(|segment| {
            let decoded = decode(segment)
                .map_err(|error| AppError::from(format!("Invalid URL path segment: {error}")))?;
            let normalized = decoded.trim().to_string();
            if normalized == "."
                || normalized == ".."
                || normalized.contains('/')
                || normalized.contains('\\')
            {
                return Err(AppError::from("URL path contains a traversal-like segment"));
            }
            Ok(normalized)
        })
        .collect()
}

fn discover_ashby(url: &Url, segments: &[String]) -> AppResult<UrlDiscovery> {
    let [board_slug, posting_id] = segments else {
        return Err(AppError::from(
            "Ashby URL must contain a board slug and posting ID",
        ));
    };

    let board_slug = normalize_board_slug(board_slug)?;
    let posting_id = normalize_posting_id(posting_id)?;
    Ok(UrlDiscovery {
        board: Some(DetectedBoard {
            provider: "ashby".into(),
            board_url: format!("{}://jobs.ashbyhq.com/{board_slug}", url.scheme()),
            board_slug,
            posting_id: Some(posting_id),
        }),
        careers_url: None,
    })
}

fn discover_greenhouse(url: &Url, host: &str, segments: &[String]) -> AppResult<UrlDiscovery> {
    let [board_slug, jobs, posting_id] = segments else {
        return Err(AppError::from(
            "Greenhouse URL must contain a board slug, jobs segment, and posting ID",
        ));
    };
    if !jobs.eq_ignore_ascii_case("jobs") {
        return Err(AppError::from(
            "Greenhouse URL must contain a jobs path segment",
        ));
    }

    let board_slug = normalize_board_slug(board_slug)?;
    let posting_id = normalize_posting_id(posting_id)?;
    Ok(UrlDiscovery {
        board: Some(DetectedBoard {
            provider: "greenhouse".into(),
            board_url: format!("{}://{host}/{board_slug}", url.scheme()),
            board_slug,
            posting_id: Some(posting_id),
        }),
        careers_url: None,
    })
}

fn discover_lever(url: &Url, host: &str, segments: &[String]) -> AppResult<UrlDiscovery> {
    let [board_slug, posting_id] = segments else {
        return Err(AppError::from(
            "Lever URL must contain a board slug and posting ID",
        ));
    };

    let board_slug = normalize_board_slug(board_slug)?;
    let posting_id = normalize_posting_id(posting_id)?;
    Ok(UrlDiscovery {
        board: Some(DetectedBoard {
            provider: "lever".into(),
            board_url: format!("{}://{host}/{board_slug}", url.scheme()),
            board_slug,
            posting_id: Some(posting_id),
        }),
        careers_url: None,
    })
}

fn normalize_board_slug(segment: &str) -> AppResult<String> {
    let normalized = normalize_identifier(segment, "board slug")?;
    Ok(normalized.to_ascii_lowercase())
}

fn normalize_posting_id(segment: &str) -> AppResult<String> {
    normalize_identifier(segment, "posting ID")
}

fn normalize_identifier(segment: &str, label: &str) -> AppResult<String> {
    if segment.is_empty()
        || segment
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(AppError::from(format!("URL is missing a valid {label}")));
    }
    Ok(segment.to_string())
}

fn careers_root(url: &Url, host: &str, segments: &[String]) -> Option<String> {
    segments
        .first()
        .filter(|segment| segment.eq_ignore_ascii_case("careers"))
        .map(|_| format!("{}://{host}/careers", url.scheme()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_all_csv_ashby_postings() {
        for (url, slug, posting_id) in [
            (
                "https://jobs.ashbyhq.com/bayesianhealth/a4bd37a8-644b-4889-a378-cb047a05669f?hl=en-US",
                "bayesianhealth",
                "a4bd37a8-644b-4889-a378-cb047a05669f",
            ),
            (
                "https://jobs.ashbyhq.com/chaidiscovery/49557cff-8121-4a6d-bfa3-83f2fabe080f",
                "chaidiscovery",
                "49557cff-8121-4a6d-bfa3-83f2fabe080f",
            ),
            (
                "https://jobs.ashbyhq.com/chaidiscovery/64fcb54e-d008-4551-858a-5c0f3b48f270",
                "chaidiscovery",
                "64fcb54e-d008-4551-858a-5c0f3b48f270",
            ),
        ] {
            let discovery = discover_from_url(url).unwrap();
            assert_eq!(
                discovery.board,
                Some(DetectedBoard {
                    provider: "ashby".into(),
                    board_slug: slug.into(),
                    board_url: format!("https://jobs.ashbyhq.com/{slug}"),
                    posting_id: Some(posting_id.into()),
                })
            );
            assert_eq!(discovery.careers_url, None);
        }
    }

    #[test]
    fn detects_all_csv_greenhouse_postings_and_host_variants() {
        for url in [
            "https://job-boards.greenhouse.io/thinkingmachines/jobs/5013911008",
            "https://job-boards.greenhouse.io/thinkingmachines/jobs/5111543008",
            "https://job-boards.greenhouse.io/thinkingmachines/jobs/5202369008",
        ] {
            let discovery = discover_from_url(url).unwrap();
            assert_eq!(discovery.board.as_ref().unwrap().provider, "greenhouse");
            assert_eq!(
                discovery.board.as_ref().unwrap().board_slug,
                "thinkingmachines"
            );
            assert_eq!(
                discovery.board.as_ref().unwrap().board_url,
                "https://job-boards.greenhouse.io/thinkingmachines"
            );
        }

        let legacy =
            discover_from_url("https://boards.greenhouse.io/thinkingmachines/jobs/123").unwrap();
        assert_eq!(
            legacy.board.unwrap().board_url,
            "https://boards.greenhouse.io/thinkingmachines"
        );
    }

    #[test]
    fn detects_csv_careers_only_urls_without_fabricating_ats_boards() {
        let onebrief = discover_from_url(
            "https://www.onebrief.com/careers?hl=en-US&ashby_jid=a88e10d4-66d8-4911-99e3-3d20351e73d9",
        )
        .unwrap();
        assert_eq!(onebrief.board, None);
        assert_eq!(
            onebrief.careers_url.as_deref(),
            Some("https://www.onebrief.com/careers")
        );

        let judgment = discover_from_url(
            "https://www.judgmentlabs.ai/careers/26ab8af7-8b33-43aa-9063-1ff782b36beb?utm_source=cv-newsletter&_bhlid=61aa2dcea0b9c3174bb0f7fb30b9f4a614bfa3ea",
        )
        .unwrap();
        assert_eq!(judgment.board, None);
        assert_eq!(
            judgment.careers_url.as_deref(),
            Some("https://www.judgmentlabs.ai/careers")
        );

        let sakana = discover_from_url(
            "https://sakana.ai/careers/software-engineer-research-and-development/",
        )
        .unwrap();
        assert_eq!(sakana.board, None);
        assert_eq!(
            sakana.careers_url.as_deref(),
            Some("https://sakana.ai/careers")
        );
    }

    #[test]
    fn detects_standard_lever_posting_shape() {
        let discovery =
            discover_from_url("https://jobs.lever.co/acme/abc-123?source=linkedin#details")
                .unwrap();
        assert_eq!(
            discovery.board,
            Some(DetectedBoard {
                provider: "lever".into(),
                board_slug: "acme".into(),
                board_url: "https://jobs.lever.co/acme".into(),
                posting_id: Some("abc-123".into()),
            })
        );
        assert_eq!(discovery.careers_url, None);
    }

    #[test]
    fn normalizes_host_casing_trailing_slashes_fragments_and_tracking_queries() {
        let discovery = discover_from_url(
            "HTTPS://JOBS.ASHBYHQ.COM/ChaiDiscovery/abc-123/?utm_source=test#posting",
        )
        .unwrap();
        assert_eq!(
            discovery.board.unwrap(),
            DetectedBoard {
                provider: "ashby".into(),
                board_slug: "chaidiscovery".into(),
                board_url: "https://jobs.ashbyhq.com/chaidiscovery".into(),
                posting_id: Some("abc-123".into()),
            }
        );

        let careers =
            discover_from_url("https://Example.COM/careers/role/?utm_source=test#details").unwrap();
        assert_eq!(
            careers.careers_url.as_deref(),
            Some("https://example.com/careers")
        );
    }

    #[test]
    fn rejects_unsupported_schemes_malformed_ats_paths_and_traversal() {
        for url in [
            "file:///tmp/job",
            "https://jobs.ashbyhq.com/only-a-slug",
            "https://boards.greenhouse.io/acme/not-jobs/123",
            "https://jobs.lever.co/acme",
            "https://jobs.ashbyhq.com/acme/%2e%2e",
        ] {
            assert!(discover_from_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn unrelated_careers_paths_never_become_ats_watches() {
        let discovery = discover_from_url("https://example.com/careers/team").unwrap();
        assert!(discovery.board.is_none());
        assert_eq!(
            discovery.careers_url.as_deref(),
            Some("https://example.com/careers")
        );

        let unrelated = discover_from_url("https://example.com/jobs/acme").unwrap();
        assert_eq!(unrelated, UrlDiscovery::default());
    }

    #[test]
    fn serializes_discovery_using_camel_case_fields() {
        let value = serde_json::to_value(UrlDiscovery {
            board: Some(DetectedBoard {
                provider: "ashby".into(),
                board_slug: "acme".into(),
                board_url: "https://jobs.ashbyhq.com/acme".into(),
                posting_id: Some("abc".into()),
            }),
            careers_url: None,
        })
        .unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "board": {
                    "provider": "ashby",
                    "boardSlug": "acme",
                    "boardUrl": "https://jobs.ashbyhq.com/acme",
                    "postingId": "abc"
                },
                "careersUrl": null
            })
        );
    }
}
