pub mod careers;
pub mod sync;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::jobs::safe_fetch::safe_fetch;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtsJob {
    pub external_id: String,
    pub title: String,
    pub url: String,
    pub location: Option<String>,
}

pub async fn validate_board(provider: &str, board_slug: &str) -> AppResult<()> {
    let _ = list_jobs(provider, board_slug).await?;
    Ok(())
}

pub async fn list_jobs(provider: &str, board_slug: &str) -> AppResult<Vec<AtsJob>> {
    let (url, accept_err) = match provider {
        "greenhouse" => (
            format!(
                "https://boards-api.greenhouse.io/v1/boards/{}/jobs",
                urlencoding::encode(board_slug)
            ),
            "Greenhouse",
        ),
        "lever" => (
            format!(
                "https://api.lever.co/v0/postings/{}?mode=json",
                urlencoding::encode(board_slug)
            ),
            "Lever",
        ),
        "ashby" => (
            format!(
                "https://api.ashbyhq.com/posting-api/job-board/{}",
                urlencoding::encode(board_slug)
            ),
            "Ashby",
        ),
        _ => return Err(AppError::from(format!("Unhandled provider: {provider}"))),
    };

    let result = safe_fetch(&url, Some("GET"), Some("application/json")).await;
    if !result.ok {
        return Err(AppError::from(
            result
                .error
                .unwrap_or_else(|| format!("{accept_err} board unavailable (HTTP {})", result.status)),
        ));
    }
    parse_ats_jobs_from_json(provider, &result.body_text)
}

pub fn parse_ats_jobs_from_json(provider: &str, body_text: &str) -> AppResult<Vec<AtsJob>> {
    match provider {
        "greenhouse" => {
            let parsed: serde_json::Value =
                serde_json::from_str(body_text).map_err(|e| AppError::from(e.to_string()))?;
            let jobs = parsed
                .get("jobs")
                .and_then(|j| j.as_array())
                .ok_or_else(|| AppError::from("Unexpected Greenhouse response shape"))?;
            Ok(jobs
                .iter()
                .map(|job| AtsJob {
                    external_id: job
                        .get("id")
                        .map(|v| match v {
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default(),
                    title: job
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    url: job
                        .get("absolute_url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    location: job
                        .pointer("/location/name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                })
                .collect())
        }
        "lever" => {
            let parsed: Vec<serde_json::Value> =
                serde_json::from_str(body_text).map_err(|_| {
                    AppError::from("Unexpected Lever response shape")
                })?;
            Ok(parsed
                .into_iter()
                .map(|job| AtsJob {
                    external_id: job
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    title: job
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    url: job
                        .get("hostedUrl")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    location: job
                        .pointer("/categories/location")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                })
                .collect())
        }
        "ashby" => {
            let parsed: serde_json::Value =
                serde_json::from_str(body_text).map_err(|e| AppError::from(e.to_string()))?;
            let jobs = parsed
                .get("jobs")
                .and_then(|j| j.as_array())
                .ok_or_else(|| AppError::from("Unexpected Ashby response shape"))?;
            Ok(jobs
                .iter()
                .map(|job| AtsJob {
                    external_id: job
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    title: job
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    url: job
                        .get("jobUrl")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    location: job
                        .get("location")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                })
                .collect())
        }
        _ => Err(AppError::from(format!("Unhandled provider: {provider}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_greenhouse() {
        let jobs = parse_ats_jobs_from_json(
            "greenhouse",
            r#"{"jobs":[{"id":1,"title":"Designer","absolute_url":"https://boards.greenhouse.io/acme/jobs/1","location":{"name":"Remote"}}]}"#,
        )
        .unwrap();
        assert_eq!(jobs[0].external_id, "1");
        assert_eq!(jobs[0].title, "Designer");
        assert_eq!(jobs[0].location.as_deref(), Some("Remote"));
    }

    #[test]
    fn parses_lever() {
        let jobs = parse_ats_jobs_from_json(
            "lever",
            r#"[{"id":"abc","text":"Engineer","hostedUrl":"https://jobs.lever.co/acme/abc","categories":{"location":"NYC"}}]"#,
        )
        .unwrap();
        assert_eq!(jobs[0].external_id, "abc");
    }

    #[test]
    fn parses_ashby() {
        let jobs = parse_ats_jobs_from_json(
            "ashby",
            r#"{"jobs":[{"id":"ash-1","title":"PM","jobUrl":"https://jobs.ashbyhq.com/acme/ash-1","location":"SF"}]}"#,
        )
        .unwrap();
        assert_eq!(jobs[0].title, "PM");
    }

    #[test]
    fn rejects_malformed_greenhouse() {
        let err = parse_ats_jobs_from_json("greenhouse", "{}").unwrap_err();
        assert!(err.to_string().contains("Unexpected Greenhouse"));
    }
}
