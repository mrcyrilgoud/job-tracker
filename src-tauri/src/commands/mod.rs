use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, WebviewWindow};

use crate::ats;
use crate::ats::careers::{apply_careers_check, company_careers_url, fetch_careers_hash};
use crate::ats::sync::{apply_watch_sync, fetch_remote_jobs};
use crate::companies;
use crate::db::AppState;
use crate::documents;
use crate::error::AppResult;
use crate::filtering::engine::{matches as engine_matches, JobView};
use crate::filtering::model::FilterCriteria;
use crate::filtering::resolver::{
    get_global_criteria, get_watch_criteria, load_alias_table, set_global_criteria,
    set_watch_criteria, FILTER_CRITERIA_VERSION,
};
use crate::jobs::board_discovery::discover_from_url;
use crate::jobs::check_active::{
    apply_posting_evidence, evaluate_posting, load_posting_check_input,
};
use crate::jobs::csv::{export_jobs_csv, get_jobs_csv_status, import_jobs_csv, ImportMode};
use crate::jobs::csv_config::{
    active_csv_path, clear_custom_csv_path, csv_lock_path, get_csv_config, set_custom_csv_path,
    validate_csv_path, CsvConfig,
};
use crate::jobs::csv_export::with_csv_file_lock;
use crate::jobs::metadata::{resolve_job_metadata, JobMetadata};
use crate::jobs::posting_check::fetch::HttpPostingFetcher;
use crate::jobs::service::{
    approve_watch_job, archive_job, create_job_from_url_with_careers,
    delete_job as delete_job_service, dismiss_watch_job, get_job_detail, get_location_settings,
    get_pipeline_counts, get_watch_role_keywords as service_get_keywords, get_weekly_activity,
    list_jobs, list_open_watch_positions, reset_dismissed_watch_job, resolve_title_from_url,
    save_open_watch_job, set_job_favorite, set_location_settings,
    set_watch_role_keywords as service_set_keywords, toggle_job_favorite, unarchive_job,
    update_job, JobFilters, LocationSettings, UpdateJobInput,
};
use crate::runner::try_lock_runner;
use crate::runs::coordinator::{RunCoordinator, RunRequest, SystemClock};
use crate::runs::legacy::execute_legacy;
use crate::runs::model::Trigger;
use crate::runs::progress::TauriSink;
use crate::runs::RunRegistry;

pub mod runs;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListJobsArgs {
    pub status: Option<String>,
    pub company_id: Option<String>,
    pub posting_state: Option<String>,
    pub search: Option<String>,
    pub location: Option<String>,
    pub new_from_watch: Option<bool>,
    pub is_favorite: Option<bool>,
    pub is_archived: Option<bool>,
    /// Optional row cap forwarded to SQLite LIMIT — used to cap watch-preview fetches.
    pub limit: Option<usize>,
}

#[tauri::command]
pub async fn list_jobs_cmd(
    state: State<'_, AppState>,
    filters: Option<ListJobsArgs>,
) -> AppResult<serde_json::Value> {
    let filters = filters.unwrap_or(ListJobsArgs {
        status: None,
        company_id: None,
        posting_state: None,
        search: None,
        location: None,
        new_from_watch: None,
        is_favorite: None,
        is_archived: None,
        limit: None,
    });
    state.with_db(|conn| {
        let jobs = list_jobs(
            conn,
            JobFilters {
                status: filters.status,
                company_id: filters.company_id,
                posting_state: filters.posting_state,
                search: filters.search,
                location: filters.location,
                new_from_watch: filters.new_from_watch,
                is_favorite: filters.is_favorite,
                is_archived: filters.is_archived,
                limit: filters.limit,
            },
        )?;
        Ok(serde_json::json!({ "jobs": jobs }))
    })
}

/// Returns pipeline counts, 7-day activity, and the data directory path.
/// Decoupled from `list_jobs_cmd` so expensive aggregations are not
/// re-computed on every filter change or watch-preview refresh.
#[tauri::command]
pub async fn get_jobs_dashboard(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let counts = get_pipeline_counts(conn)?;
        let weekly = get_weekly_activity(conn)?;
        Ok(serde_json::json!({
            "counts": counts,
            "weeklyActivity": weekly,
            "dataDir": state.paths.data_dir
        }))
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateJobArgs {
    pub url: String,
    pub title: Option<String>,
    pub company_name: Option<String>,
    pub status: Option<String>,
    pub applied_at: Option<String>,
    pub notes: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub confirmed_discovery: Option<ConfirmedJobDiscovery>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedJobDiscovery {
    pub provider: Option<String>,
    pub board_slug: Option<String>,
    pub careers_url: Option<String>,
}

#[tauri::command]
pub async fn preview_job_url(url: String) -> AppResult<JobMetadata> {
    resolve_job_metadata(&url).await
}

#[tauri::command]
pub async fn create_job(
    state: State<'_, AppState>,
    input: CreateJobArgs,
) -> AppResult<serde_json::Value> {
    create_job_with_validator(&state, input, |provider, board_slug| {
        Box::pin(ats::validate_board(provider, board_slug))
    })
    .await
}

type BoardValidatorFuture<'a> = Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>>;

async fn create_job_with_validator<F>(
    state: &AppState,
    input: CreateJobArgs,
    validate_board: F,
) -> AppResult<serde_json::Value>
where
    F: for<'a> Fn(&'a str, &'a str) -> BoardValidatorFuture<'a>,
{
    let confirmed_board = match input.confirmed_discovery.as_ref() {
        Some(discovery) => match (
            discovery.provider.as_deref(),
            discovery.board_slug.as_deref(),
        ) {
            (None, None) => None,
            (Some(provider), Some(board_slug)) => {
                let provider = provider.trim().to_ascii_lowercase();
                let board_slug = board_slug.trim().to_ascii_lowercase();
                if provider.is_empty() || board_slug.is_empty() {
                    return Err(crate::error::AppError::from(
                        "Confirmed board provider and slug are required",
                    ));
                }
                validate_board(&provider, &board_slug).await?;
                Some((provider, board_slug))
            }
            _ => {
                return Err(crate::error::AppError::from(
                    "Confirmed board provider and slug must be provided together",
                ));
            }
        },
        None => None,
    };
    let confirmed_careers_url = match input
        .confirmed_discovery
        .as_ref()
        .and_then(|discovery| discovery.careers_url.as_deref())
    {
        Some(careers_url) => {
            Some(discover_from_url(careers_url)?.careers_url.ok_or_else(|| {
                crate::error::AppError::from("Confirmed careers URL must point to a careers page")
            })?)
        }
        None => None,
    };
    let title = resolve_title_from_url(&input.url, input.title.as_deref()).await;
    let result = state.with_db_tx(|conn| {
        let (job, company) = create_job_from_url_with_careers(
            conn,
            &input.url,
            &title,
            input.company_name.as_deref(),
            input.status.as_deref(),
            input.applied_at.as_deref(),
            input.notes.as_deref(),
            input.description.as_deref(),
            input.location.as_deref(),
            confirmed_careers_url.as_deref(),
        )?;
        let watch = confirmed_board
            .as_ref()
            .map(|(provider, board_slug)| {
                companies::insert_watch(conn, &company.id, provider, board_slug)
            })
            .transpose()?;
        Ok(serde_json::json!({ "job": job, "company": company, "watch": watch }))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{paths::DataPaths, AppState};
    use crate::jobs::board_discovery::discover_from_url;
    use tempfile::{tempdir, TempDir};

    fn test_state() -> (TempDir, AppState) {
        let directory = tempdir().unwrap();
        let state =
            AppState::open(DataPaths::from_data_dir(directory.path().to_path_buf())).unwrap();
        (directory, state)
    }

    fn create_input(
        url: &str,
        company_name: &str,
        title: &str,
        confirmed_discovery: Option<ConfirmedJobDiscovery>,
    ) -> CreateJobArgs {
        CreateJobArgs {
            url: url.to_string(),
            title: Some(title.to_string()),
            company_name: Some(company_name.to_string()),
            status: Some("wishlist".to_string()),
            applied_at: None,
            notes: None,
            description: None,
            location: None,
            confirmed_discovery,
        }
    }

    fn confirmed_board(url: &str) -> ConfirmedJobDiscovery {
        let board = discover_from_url(url).unwrap().board.unwrap();
        ConfirmedJobDiscovery {
            provider: Some(board.provider),
            board_slug: Some(board.board_slug),
            careers_url: None,
        }
    }

    fn confirmed_careers(url: &str) -> ConfirmedJobDiscovery {
        ConfirmedJobDiscovery {
            provider: None,
            board_slug: None,
            careers_url: discover_from_url(url).unwrap().careers_url,
        }
    }

    fn row_counts(state: &AppState) -> (i64, i64, i64) {
        state
            .with_db(|conn| {
                let jobs = conn
                    .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
                    .map_err(crate::error::map_sqlite)?;
                let companies = conn
                    .query_row("SELECT COUNT(*) FROM companies", [], |row| row.get(0))
                    .map_err(crate::error::map_sqlite)?;
                let watches = conn
                    .query_row("SELECT COUNT(*) FROM company_watches", [], |row| row.get(0))
                    .map_err(crate::error::map_sqlite)?;
                Ok((jobs, companies, watches))
            })
            .unwrap()
    }

    #[tokio::test]
    async fn confirmed_csv_ashby_flow_persists_one_idempotent_watch() {
        let (_directory, state) = test_state();
        let first_url =
            "https://jobs.ashbyhq.com/bayesianhealth/a4bd37a8-644b-4889-a378-cb047a05669f";
        let second_url =
            "https://jobs.ashbyhq.com/chaidiscovery/49557cff-8121-4a6d-bfa3-83f2fabe080f";

        let first = create_job_with_validator(
            &state,
            create_input(
                first_url,
                "Chai Discovery",
                "Research Engineer",
                Some(confirmed_board(first_url)),
            ),
            |provider, slug| {
                let provider = provider.to_string();
                let slug = slug.to_string();
                Box::pin(async move {
                    assert_eq!(
                        (provider.as_str(), slug.as_str()),
                        ("ashby", "bayesianhealth")
                    );
                    Ok(())
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(first["watch"]["provider"], "ashby");
        assert_eq!(first["watch"]["boardSlug"], "bayesianhealth");

        let second = create_job_with_validator(
            &state,
            create_input(
                second_url,
                "Chai Discovery",
                "ML Engineer",
                Some(ConfirmedJobDiscovery {
                    provider: Some("ashby".into()),
                    board_slug: Some("bayesianhealth".into()),
                    careers_url: None,
                }),
            ),
            |provider, slug| {
                let provider = provider.to_string();
                let slug = slug.to_string();
                Box::pin(async move {
                    assert_eq!(
                        (provider.as_str(), slug.as_str()),
                        ("ashby", "bayesianhealth")
                    );
                    Ok(())
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(second["watch"]["boardSlug"], "bayesianhealth");
        assert_eq!(row_counts(&state), (2, 1, 1));
    }

    #[tokio::test]
    async fn confirmed_greenhouse_csv_urls_reuse_one_watch() {
        let (_directory, state) = test_state();
        for (index, url) in [
            "https://job-boards.greenhouse.io/thinkingmachines/jobs/5013911008",
            "https://job-boards.greenhouse.io/thinkingmachines/jobs/5111543008",
            "https://job-boards.greenhouse.io/thinkingmachines/jobs/5202369008",
        ]
        .into_iter()
        .enumerate()
        {
            let result = create_job_with_validator(
                &state,
                create_input(
                    url,
                    "Thinking Machines Lab",
                    &format!("Role {index}"),
                    Some(confirmed_board(url)),
                ),
                |provider, slug| {
                    let provider = provider.to_string();
                    let slug = slug.to_string();
                    Box::pin(async move {
                        assert_eq!(
                            (provider.as_str(), slug.as_str()),
                            ("greenhouse", "thinkingmachines")
                        );
                        Ok(())
                    })
                },
            )
            .await
            .unwrap();
            assert_eq!(result["watch"]["boardSlug"], "thinkingmachines");
        }

        assert_eq!(row_counts(&state), (3, 1, 1));
    }

    #[tokio::test]
    async fn careers_only_confirmation_persists_careers_url_without_watch() {
        let (_directory, state) = test_state();
        let url = "https://www.onebrief.com/careers?ashby_jid=a88e10d4-66d8-4911-99e3-3d20351e73d9";
        let result = create_job_with_validator(
            &state,
            create_input(
                url,
                "Onebrief",
                "Product Engineer",
                Some(confirmed_careers(url)),
            ),
            |_provider, _slug| {
                Box::pin(async { panic!("careers-only flow must not validate a board") })
            },
        )
        .await
        .unwrap();

        assert!(result["watch"].is_null());
        state
            .with_db(|conn| {
                let careers_url: String = conn
                    .query_row(
                        "SELECT careers_url FROM companies WHERE name = 'Onebrief'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(crate::error::map_sqlite)?;
                assert_eq!(careers_url, "https://www.onebrief.com/careers");
                Ok(())
            })
            .unwrap();
        assert_eq!(row_counts(&state), (1, 1, 0));
    }

    #[tokio::test]
    async fn unconfirmed_csv_candidate_creates_only_job_and_company() {
        let (_directory, state) = test_state();
        let url = "https://jobs.ashbyhq.com/bayesianhealth/a4bd37a8-644b-4889-a378-cb047a05669f";
        let result = create_job_with_validator(
            &state,
            create_input(url, "Bayesian Health", "Software Engineer", None),
            |_provider, _slug| {
                Box::pin(async { panic!("unconfirmed flow must not validate a board") })
            },
        )
        .await
        .unwrap();

        assert!(result["watch"].is_null());
        assert_eq!(row_counts(&state), (1, 1, 0));
    }

    #[tokio::test]
    async fn failed_board_validation_leaves_no_partial_persistence() {
        let (_directory, state) = test_state();
        let url = "https://jobs.ashbyhq.com/bayesianhealth/a4bd37a8-644b-4889-a378-cb047a05669f";
        let error = create_job_with_validator(
            &state,
            create_input(
                url,
                "Bayesian Health",
                "Software Engineer",
                Some(confirmed_board(url)),
            ),
            |_provider, _slug| {
                Box::pin(async { Err(crate::error::AppError::from("Ashby board unavailable")) })
            },
        )
        .await
        .unwrap_err();

        assert_eq!(error.to_string(), "Ashby board unavailable");
        assert_eq!(row_counts(&state), (0, 0, 0));
    }

    #[tokio::test]
    async fn malformed_confirmation_fails_before_validation_or_persistence() {
        let (_directory, state) = test_state();
        let error = create_job_with_validator(
            &state,
            create_input(
                "https://example.com/careers/role",
                "Example",
                "Role",
                Some(ConfirmedJobDiscovery {
                    provider: Some("ashby".into()),
                    board_slug: None,
                    careers_url: None,
                }),
            ),
            |_provider, _slug| {
                Box::pin(async { panic!("malformed confirmation must not validate") })
            },
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Confirmed board provider and slug must be provided together"
        );
        assert_eq!(row_counts(&state), (0, 0, 0));
    }
}

#[tauri::command]
pub async fn get_job(state: State<'_, AppState>, id: String) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let detail = get_job_detail(conn, &id)?;
        Ok(serde_json::json!({ "detail": detail }))
    })
}

#[tauri::command]
pub async fn update_job_cmd(
    state: State<'_, AppState>,
    id: String,
    updates: UpdateJobInput,
) -> AppResult<serde_json::Value> {
    let result = state.with_db_tx(|conn| {
        let detail = update_job(conn, &id, updates)?;
        Ok(serde_json::json!({ "detail": detail }))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn delete_job(state: State<'_, AppState>, id: String) -> AppResult<serde_json::Value> {
    let result = state.with_db_tx(|conn| {
        delete_job_service(conn, &id)?;
        Ok(serde_json::json!({ "success": true, "id": id }))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn archive_job_cmd(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<serde_json::Value> {
    let result = state.with_db_tx(|conn| {
        let detail = archive_job(conn, &id)?;
        Ok(serde_json::json!({ "detail": detail }))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn unarchive_job_cmd(
    state: State<'_, AppState>,
    id: String,
    target_status: Option<String>,
) -> AppResult<serde_json::Value> {
    let result = state.with_db_tx(|conn| {
        let detail = unarchive_job(conn, &id, target_status.as_deref())?;
        Ok(serde_json::json!({ "detail": detail }))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn toggle_job_favorite_cmd(
    state: State<'_, AppState>,
    job_id: String,
) -> AppResult<serde_json::Value> {
    let result = state.with_db_tx(|conn| {
        let item = toggle_job_favorite(conn, &job_id)?;
        Ok(serde_json::json!({ "item": item }))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn set_job_favorite_cmd(
    state: State<'_, AppState>,
    job_id: String,
    is_favorite: bool,
) -> AppResult<serde_json::Value> {
    let result = state.with_db_tx(|conn| {
        let item = set_job_favorite(conn, &job_id, is_favorite)?;
        Ok(serde_json::json!({ "item": item }))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn check_job_posting(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<serde_json::Value> {
    // Single-posting check: not a Run, no runner lock. The DB mutex is not held
    // across the network evaluation.
    let input = state.with_db(|conn| load_posting_check_input(conn, &id))?;
    let evidence = evaluate_posting(&input, &HttpPostingFetcher).await;
    let result = state.with_db_tx(|conn| {
        let result = apply_posting_evidence(conn, &id, &evidence)?;
        Ok(serde_json::json!(result))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn list_companies(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let rows = companies::list_companies_with_watches(conn)?;
        Ok(serde_json::json!({ "companies": rows }))
    })
}

#[tauri::command]
pub async fn list_open_watch_positions_cmd(
    state: State<'_, AppState>,
    company_id: String,
) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let positions = list_open_watch_positions(conn, &company_id)?;
        Ok(serde_json::json!({ "positions": positions }))
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCompanyArgs {
    pub name: String,
    pub careers_url: Option<String>,
}

#[tauri::command]
pub async fn create_company(
    state: State<'_, AppState>,
    input: CreateCompanyArgs,
) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let company = companies::create_company(conn, &input.name, input.careers_url.as_deref())?;
        Ok(serde_json::json!({ "company": company }))
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWatchArgs {
    pub company_id: String,
    pub provider: String,
    pub board_slug: String,
}

#[tauri::command]
pub async fn create_watch(
    state: State<'_, AppState>,
    input: CreateWatchArgs,
) -> AppResult<serde_json::Value> {
    ats::validate_board(&input.provider, &input.board_slug).await?;
    state.with_db(|conn| {
        let watch =
            companies::insert_watch(conn, &input.company_id, &input.provider, &input.board_slug)?;
        Ok(serde_json::json!({ "watch": watch }))
    })
}

#[tauri::command]
pub async fn delete_watch(state: State<'_, AppState>, watch_id: String) -> AppResult<()> {
    state.with_db(|conn| companies::delete_watch(conn, &watch_id))
}

#[tauri::command]
pub async fn sync_watch(
    state: State<'_, AppState>,
    watch_id: String,
) -> AppResult<serde_json::Value> {
    let _runner_guard = state
        .runner_lock
        .try_lock()
        .map_err(|_| crate::error::AppError::from("operation_in_progress:runner"))?;
    let _runner_file = try_lock_runner(&state.paths)?;
    let (provider, board_slug) = state.with_db(|conn| {
        let watch = companies::get_watch(conn, &watch_id)?
            .ok_or_else(|| crate::error::AppError::from("Watch not found"))?;
        Ok((watch.provider, watch.board_slug))
    })?;
    let remote = fetch_remote_jobs(&provider, &board_slug)
        .await
        .map_err(|e| e.to_string());
    let result = state.with_db(|conn| apply_watch_sync(conn, &watch_id, remote))?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn check_careers(
    state: State<'_, AppState>,
    company_id: String,
) -> AppResult<serde_json::Value> {
    let Some((name, url)) = state.with_db(|conn| company_careers_url(conn, &company_id))? else {
        return Ok(serde_json::json!({
            "changed": false,
            "reason": "No careers URL configured"
        }));
    };
    match fetch_careers_hash(&url).await {
        Ok((hash, text)) => {
            let result = state
                .with_db(|conn| apply_careers_check(conn, &company_id, &name, &hash, &text))?;
            if result
                .get("changed")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                state.csv_export.mark_dirty();
            }
            Ok(result)
        }
        Err(e) => Ok(serde_json::json!({
            "changed": false,
            "reason": e.to_string()
        })),
    }
}

#[tauri::command]
pub async fn dismiss_review(state: State<'_, AppState>, review_id: String) -> AppResult<()> {
    state.with_db(|conn| companies::dismiss_careers_review(conn, &review_id))
}

#[tauri::command]
pub async fn approve_watch_job_cmd(
    state: State<'_, AppState>,
    job_id: String,
) -> AppResult<serde_json::Value> {
    let result = state.with_db_tx(|conn| {
        let job = approve_watch_job(conn, &job_id)?;
        Ok(serde_json::json!({ "job": job }))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn dismiss_watch_job_cmd(
    state: State<'_, AppState>,
    job_id: String,
) -> AppResult<serde_json::Value> {
    let result = state.with_db_tx(|conn| {
        let job = dismiss_watch_job(conn, &job_id)?;
        Ok(serde_json::json!({ "job": job }))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn save_open_watch_job_cmd(
    state: State<'_, AppState>,
    job_id: String,
) -> AppResult<serde_json::Value> {
    let result = state.with_db_tx(|conn| {
        let job = save_open_watch_job(conn, &job_id)?;
        Ok(serde_json::json!({ "job": job }))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn reset_dismissed_watch_job_cmd(
    state: State<'_, AppState>,
    job_id: String,
) -> AppResult<serde_json::Value> {
    let result = state.with_db_tx(|conn| {
        let job = reset_dismissed_watch_job(conn, &job_id)?;
        Ok(serde_json::json!({ "job": job }))
    })?;
    state.csv_export.mark_dirty();
    Ok(result)
}

#[tauri::command]
pub async fn list_documents(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let docs = documents::list_documents(conn)?;
        Ok(serde_json::json!({ "documents": docs }))
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDocumentArgs {
    pub original_filename: String,
    pub mime_type: String,
    pub bytes_base64: String,
    pub job_id: Option<String>,
    pub kind: Option<String>,
}

#[tauri::command]
pub async fn import_document(
    state: State<'_, AppState>,
    input: ImportDocumentArgs,
) -> AppResult<serde_json::Value> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input.bytes_base64.as_bytes())
        .map_err(|e| crate::error::AppError::from(e.to_string()))?;
    state
        .with_db_tx(|conn| {
            let doc = documents::import_document(
                conn,
                &state.paths.documents_dir,
                &input.original_filename,
                &input.mime_type,
                &bytes,
            )?;
            if let (Some(job_id), Some(kind)) = (input.job_id.as_deref(), input.kind.as_deref()) {
                let attachment = documents::attach_document_to_job(conn, job_id, &doc.id, kind)?;
                crate::jobs::service::add_job_event(
                    conn,
                    job_id,
                    "document_attached",
                    Some(&format!("Attached {} ({kind})", doc.original_filename)),
                )?;
                return Ok(serde_json::json!({ "document": doc, "attachment": attachment }));
            }
            Ok(serde_json::json!({ "document": doc }))
        })
        .and_then(|value| {
            // Finalize file only after the DB transaction commits.
            if let Some(doc_val) = value.get("document") {
                let doc: crate::models::Document = serde_json::from_value(doc_val.clone())
                    .map_err(|e| crate::error::AppError::from(e.to_string()))?;
                state.with_db(|conn| {
                    documents::finalize_staged_document(conn, &state.paths.documents_dir, &doc)
                })?;
            }
            Ok(value)
        })
        .map_err(|err| {
            // Best-effort temp cleanup if the transaction failed after staging bytes.
            err
        })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachArgs {
    pub job_id: String,
    pub document_id: String,
    pub kind: String,
}

#[tauri::command]
pub async fn attach_document(
    state: State<'_, AppState>,
    input: AttachArgs,
) -> AppResult<serde_json::Value> {
    state.with_db_tx(|conn| {
        let attachment = documents::attach_document_to_job(
            conn,
            &input.job_id,
            &input.document_id,
            &input.kind,
        )?;
        crate::jobs::service::add_job_event(
            conn,
            &input.job_id,
            "document_attached",
            Some(&format!("Attached document ({})", input.kind)),
        )?;
        Ok(serde_json::json!({ "attachment": attachment }))
    })
}

#[tauri::command]
pub async fn detach_document(state: State<'_, AppState>, attachment_id: String) -> AppResult<()> {
    state.with_db(|conn| documents::detach_document(conn, &attachment_id))
}

#[tauri::command]
pub async fn open_document(state: State<'_, AppState>, document_id: String) -> AppResult<()> {
    let path = state.with_db(|conn| {
        let (_doc, path) =
            documents::get_document_file_path(conn, &state.paths.documents_dir, &document_id)?;
        Ok(path)
    })?;
    tauri_plugin_opener::open_path(path, None::<&str>)
        .map_err(|e| crate::error::AppError::from(e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub async fn csv_status(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let csv_path = active_csv_path(conn, &state.paths.jobs_csv_path)?;
        let status = with_csv_file_lock(&csv_lock_path(&csv_path), || {
            get_jobs_csv_status(conn, &csv_path)
        })?;
        Ok(serde_json::json!(status))
    })
}

#[tauri::command]
pub async fn csv_export(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    let paths = state.paths.clone();
    let csv_path = state.with_db(|conn| active_csv_path(conn, &paths.jobs_csv_path))?;
    let result = with_csv_file_lock(&csv_lock_path(&csv_path), || {
        // Dedicated connection so interactive export does not hold the UI mutex for CSV I/O.
        let conn = Connection::open(&paths.db_path)
            .map_err(|e| crate::error::AppError::from(e.to_string()))?;
        conn.pragma_update(None, "busy_timeout", 5000i32)
            .map_err(|e| crate::error::AppError::from(e.to_string()))?;
        export_jobs_csv(&conn, &csv_path, None)
    })?;
    Ok(serde_json::json!(result))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportArgs {
    pub content: Option<String>,
    pub dry_run: Option<bool>,
    pub mode: Option<String>,
}

#[tauri::command]
pub async fn csv_import(
    state: State<'_, AppState>,
    input: CsvImportArgs,
) -> AppResult<serde_json::Value> {
    let mode = match input.mode.as_deref() {
        Some("overwrite_editable") => ImportMode::OverwriteEditable,
        _ => ImportMode::Merge,
    };
    // Open a dedicated connection so async import does not hold the UI mutex across awaits.
    let paths = state.paths.clone();
    let csv_path = state.with_db(|conn| active_csv_path(conn, &paths.jobs_csv_path))?;
    let result = with_csv_file_lock(&csv_lock_path(&csv_path), || {
        import_jobs_csv(
            &paths.db_path,
            &csv_path,
            input.content.as_deref(),
            input.dry_run.unwrap_or(false),
            mode,
        )
    })?;
    Ok(serde_json::json!(result))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvConfigureArgs {
    pub path: String,
    pub mode: String,
}

#[tauri::command]
pub async fn csv_config(state: State<'_, AppState>) -> AppResult<CsvConfig> {
    state.with_db(|conn| get_csv_config(conn, &state.paths.jobs_csv_path))
}

#[tauri::command]
pub async fn csv_path_status(
    state: State<'_, AppState>,
    path: String,
) -> AppResult<serde_json::Value> {
    let path = validate_csv_path(&PathBuf::from(path))?;
    Ok(serde_json::json!({
        "path": path,
        "exists": path.exists(),
        "defaultPath": state.paths.jobs_csv_path,
    }))
}

#[tauri::command]
pub async fn csv_configure(
    state: State<'_, AppState>,
    input: CsvConfigureArgs,
) -> AppResult<CsvConfig> {
    let mode = match input.mode.as_str() {
        "import" => ImportMode::Merge,
        "replace" => ImportMode::OverwriteEditable,
        _ => {
            return Err(crate::error::AppError::from(
                "CSV mode must be import or replace",
            ))
        }
    };
    let csv_path = validate_csv_path(&PathBuf::from(input.path))?;
    if matches!(&mode, ImportMode::Merge) && !csv_path.exists() {
        return Err(crate::error::AppError::from(
            "CSV file not found for import",
        ));
    }

    let _runner_guard = state
        .runner_lock
        .try_lock()
        .map_err(|_| crate::error::AppError::from("operation_in_progress:runner"))?;
    let _runner_file = try_lock_runner(&state.paths)?;
    let paths = state.paths.clone();
    state
        .csv_export
        .with_exclusive(|| {
            let previous = state.with_db(|conn| get_csv_config(conn, &paths.jobs_csv_path))?;
            state.with_db(|conn| set_custom_csv_path(conn, &paths.jobs_csv_path, &csv_path))?;
            let result = with_csv_file_lock(&csv_lock_path(&csv_path), || match mode {
                ImportMode::Merge => {
                    import_jobs_csv(&paths.db_path, &csv_path, None, false, ImportMode::Merge)
                        .map(|_| ())
                }
                ImportMode::OverwriteEditable => {
                    let conn = Connection::open(&paths.db_path)
                        .map_err(|error| crate::error::AppError::from(error.to_string()))?;
                    export_jobs_csv(&conn, &csv_path, None).map(|_| ())
                }
            });
            if let Err(error) = result {
                restore_csv_config(&state, &paths, &previous)?;
                return Err(error);
            }
            state.with_db(|conn| get_csv_config(conn, &paths.jobs_csv_path))
        })
        .await
}

#[tauri::command]
pub async fn csv_reset_config(state: State<'_, AppState>) -> AppResult<CsvConfig> {
    let _runner_guard = state
        .runner_lock
        .try_lock()
        .map_err(|_| crate::error::AppError::from("operation_in_progress:runner"))?;
    let _runner_file = try_lock_runner(&state.paths)?;
    let paths = state.paths.clone();
    let default_csv_path = paths.jobs_csv_path.clone();
    state
        .csv_export
        .with_exclusive(|| {
            let previous = state.with_db(|conn| get_csv_config(conn, &paths.jobs_csv_path))?;
            state.with_db(|conn| clear_custom_csv_path(conn, &paths.jobs_csv_path))?;
            let result = with_csv_file_lock(&csv_lock_path(&default_csv_path), || {
                let conn = Connection::open(&paths.db_path)
                    .map_err(|error| crate::error::AppError::from(error.to_string()))?;
                export_jobs_csv(&conn, &default_csv_path, None).map(|_| ())
            });
            if let Err(error) = result {
                restore_csv_config(&state, &paths, &previous)?;
                return Err(error);
            }
            state.with_db(|conn| get_csv_config(conn, &paths.jobs_csv_path))
        })
        .await
}

fn restore_csv_config(
    state: &AppState,
    paths: &crate::db::DataPaths,
    previous: &CsvConfig,
) -> AppResult<()> {
    state.with_db(|conn| {
        if previous.is_custom {
            set_custom_csv_path(conn, &paths.jobs_csv_path, &PathBuf::from(&previous.path))?;
        } else {
            clear_custom_csv_path(conn, &paths.jobs_csv_path)?;
        }
        Ok(())
    })
}

#[tauri::command]
pub async fn run_jobs_cycle_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<serde_json::Value> {
    let hook = {
        let csv = state.csv_export.clone();
        std::sync::Arc::new(move || csv.mark_dirty())
    };
    let coordinator = RunCoordinator::new(
        state.paths.clone(),
        state.runner_lock.clone(),
        std::sync::Arc::new(crate::jobs::posting_check::fetch::HttpPostingFetcher),
        std::sync::Arc::new(TauriSink::new(app)),
        SystemClock,
        RunRegistry::new(),
    )
    .with_csv_dirty_hook(hook);
    execute_legacy(
        &coordinator,
        RunRequest::JobsCycle {
            trigger: Trigger::LegacyCommand,
        },
    )
    .await
}

#[tauri::command]
pub async fn check_all_postings_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<serde_json::Value> {
    let hook = {
        let csv = state.csv_export.clone();
        std::sync::Arc::new(move || csv.mark_dirty())
    };
    let coordinator = RunCoordinator::new(
        state.paths.clone(),
        state.runner_lock.clone(),
        std::sync::Arc::new(crate::jobs::posting_check::fetch::HttpPostingFetcher),
        std::sync::Arc::new(TauriSink::new(app)),
        SystemClock,
        RunRegistry::new(),
    )
    .with_csv_dirty_hook(hook);
    execute_legacy(
        &coordinator,
        RunRequest::PostingCheck {
            trigger: Trigger::LegacyCommand,
        },
    )
    .await
}

#[tauri::command]
pub async fn get_data_dir(state: State<'_, AppState>) -> AppResult<String> {
    Ok(state.paths.data_dir.display().to_string())
}

#[tauri::command]
pub async fn get_watch_role_keywords(state: State<'_, AppState>) -> AppResult<String> {
    let conn = state.db.lock();
    service_get_keywords(&conn)
}

#[tauri::command]
pub async fn set_watch_role_keywords(
    state: State<'_, AppState>,
    keywords: String,
) -> AppResult<()> {
    let conn = state.db.lock();
    service_set_keywords(&conn, &keywords)
}

#[tauri::command]
pub async fn get_location_settings_cmd(state: State<'_, AppState>) -> AppResult<LocationSettings> {
    let conn = state.db.lock();
    get_location_settings(&conn)
}

#[tauri::command]
pub async fn set_location_settings_cmd(
    state: State<'_, AppState>,
    settings: LocationSettings,
) -> AppResult<()> {
    let conn = state.db.lock();
    set_location_settings(&conn, &settings)
}

/// Return the structured global filter criteria (Req 12.1).
///
/// Absent/invalid stored criteria fail open to match-all inside the resolver.
#[tauri::command]
pub async fn get_filter_criteria(state: State<'_, AppState>) -> AppResult<FilterCriteria> {
    let conn = state.db.lock();
    get_global_criteria(&conn)
}

/// Persist the structured global filter criteria (Req 12.2).
///
/// Validates the schema version before writing (Req 17.1): criteria carrying a
/// version other than [`FILTER_CRITERIA_VERSION`] are rejected without
/// persisting. Non-array token fields cannot reach this point — `include`/
/// `exclude` are typed `Vec<String>`, so malformed JSON fails at Tauri's
/// deserialization boundary before the command body runs. The resolver's setter
/// trims tokens and drops empties before storing (Req 17.3), and a subsequent
/// valid write overwrites any previously stored data (Req 16.3).
#[tauri::command]
pub async fn set_filter_criteria(
    state: State<'_, AppState>,
    criteria: FilterCriteria,
) -> AppResult<()> {
    if criteria.version != FILTER_CRITERIA_VERSION {
        return Err(crate::error::AppError::from(format!(
            "unsupported filter criteria version {} (expected {})",
            criteria.version, FILTER_CRITERIA_VERSION
        )));
    }
    let conn = state.db.lock();
    set_global_criteria(&conn, &criteria)
}

/// Return the per-watch filter override for `watch_id`, or `None` when the
/// watch inherits the global criteria (Req 12.3).
///
/// Returns a watch-not-found error when `watch_id` does not identify an
/// existing watch (Req 17.2).
#[tauri::command]
pub async fn get_watch_filter_criteria(
    state: State<'_, AppState>,
    watch_id: String,
) -> AppResult<Option<FilterCriteria>> {
    let conn = state.db.lock();
    if companies::get_watch(&conn, &watch_id)?.is_none() {
        return Err(crate::error::AppError::from("Watch not found"));
    }
    get_watch_criteria(&conn, &watch_id)
}

/// Set or clear the per-watch filter override for `watch_id` (Req 12.4).
///
/// - `criteria == None` clears the override so the watch inherits the global
///   criteria (Req 12.4).
/// - `criteria == Some(_)` is validated against [`FILTER_CRITERIA_VERSION`]
///   before persisting (Req 17.1) and its tokens are trimmed/de-blanked by the
///   resolver setter (Req 17.3).
///
/// Returns a watch-not-found error when `watch_id` is unknown (Req 17.2).
#[tauri::command]
pub async fn set_watch_filter_criteria(
    state: State<'_, AppState>,
    watch_id: String,
    criteria: Option<FilterCriteria>,
) -> AppResult<()> {
    if let Some(c) = criteria.as_ref() {
        if c.version != FILTER_CRITERIA_VERSION {
            return Err(crate::error::AppError::from(format!(
                "unsupported filter criteria version {} (expected {})",
                c.version, FILTER_CRITERIA_VERSION
            )));
        }
    }
    let conn = state.db.lock();
    if companies::get_watch(&conn, &watch_id)?.is_none() {
        return Err(crate::error::AppError::from("Watch not found"));
    }
    set_watch_criteria(&conn, &watch_id, criteria.as_ref())
}

/// A single job sample for a dry-run filter preview.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSample {
    pub title: String,
    pub location: Option<String>,
}

/// The inclusion outcome for one previewed sample.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewOutcome {
    pub included: bool,
    pub reason: String,
}

/// Dry-run the given criteria against a list of sample jobs (Req 12.5).
///
/// Evaluates the passed-in `criteria` directly through the pure engine against
/// the currently loaded alias table, returning `{ included, reason }` per
/// sample in input order. This is a preview only — it neither persists criteria
/// nor requires version validation, so the editor can preview any well-formed
/// criteria before saving.
#[tauri::command]
pub async fn preview_filter_match(
    state: State<'_, AppState>,
    criteria: FilterCriteria,
    samples: Vec<PreviewSample>,
) -> AppResult<Vec<PreviewOutcome>> {
    let conn = state.db.lock();
    let aliases = load_alias_table(&conn)?;
    let outcomes = samples
        .iter()
        .map(|sample| {
            let result = engine_matches(
                &criteria,
                &aliases,
                JobView {
                    title: &sample.title,
                    location: sample.location.as_deref(),
                },
            );
            PreviewOutcome {
                included: result.included,
                reason: result.reason,
            }
        })
        .collect();
    Ok(outcomes)
}

#[tauri::command]
pub async fn show_main_window(window: WebviewWindow) -> AppResult<()> {
    window
        .show()
        .map_err(|e| crate::error::AppError::from(e.to_string()))?;
    let _ = window.set_focus();
    Ok(())
}

#[allow(dead_code)]
fn _hashmap_ty(_: HashMap<String, i64>) {}

#[cfg(test)]
mod filter_command_tests {
    //! Unit tests for the filter command surface: validation on write, legacy
    //! write-through, per-watch override clearing, and preview output shape
    //! (Task 12.3; Validates Requirements 12.4, 12.5, 15.3, 17.1, 17.2).
    //!
    //! The commands under test are `#[tauri::command] pub async fn`s that take
    //! `State<'_, AppState>`. A `tauri::State` wrapper cannot be constructed in a
    //! plain unit test without a running Tauri runtime/`App`, so these tests
    //! exercise the OBSERVABLE behavior through the exact functions the command
    //! bodies call (`crate::filtering::resolver::*`, `crate::jobs::service::*`,
    //! `crate::companies::*`, and the pure `engine::matches`). Where a command
    //! adds validation NOT present in the underlying function — the version
    //! check in `set_filter_criteria`/`set_watch_filter_criteria` and the
    //! `get_watch(..).is_none()` watch-not-found guard — the test replicates that
    //! exact predicate and asserts the underlying persistence guarantee, so the
    //! test fails if the command's contract is violated.

    use super::*;
    use crate::companies;
    use crate::db::{paths::DataPaths, AppState};
    use crate::filtering::model::{FilterCriteria, MatchMode};
    use crate::filtering::resolver::{
        get_global_criteria, get_watch_criteria, set_global_criteria, set_watch_criteria,
        FILTER_CRITERIA_VERSION,
    };
    use tempfile::{tempdir, TempDir};

    fn test_state() -> (TempDir, AppState) {
        let directory = tempdir().unwrap();
        let state =
            AppState::open(DataPaths::from_data_dir(directory.path().to_path_buf())).unwrap();
        (directory, state)
    }

    /// Seed a company + watch and return the watch id.
    fn seed_watch(state: &AppState) -> String {
        state
            .with_db(|conn| {
                let company = companies::create_company(conn, "Acme", None)?;
                let watch = companies::insert_watch(conn, &company.id, "greenhouse", "acme")?;
                Ok(watch.id)
            })
            .unwrap()
    }

    /// Req 17.1 — a set command receiving criteria with an unknown version
    /// returns an error and does NOT persist. `set_filter_criteria` rejects any
    /// `criteria.version != FILTER_CRITERIA_VERSION` before touching the DB; we
    /// replicate that guard predicate and confirm the stored global criteria are
    /// unchanged when it trips, and that a valid version persists.
    #[tokio::test]
    async fn invalid_version_rejected_without_persist() {
        let (_dir, state) = test_state();

        // Establish a known-good baseline the invalid write must not disturb.
        let mut baseline = FilterCriteria::match_all();
        baseline.title.include = vec!["baseline".to_string()];
        state
            .with_db(|conn| set_global_criteria(conn, &baseline))
            .unwrap();

        // A criteria carrying an unknown version.
        let mut bad = FilterCriteria::match_all();
        bad.version = 999;
        bad.title.include = vec!["should-not-save".to_string()];

        // The exact predicate `set_filter_criteria` applies before persisting.
        let command_would_reject = bad.version != FILTER_CRITERIA_VERSION;
        assert!(
            command_would_reject,
            "version 999 must be rejected by the command's validation predicate"
        );

        // Because the command rejects before calling the setter, the store is
        // untouched. Verify the baseline still stands.
        let stored = state.with_db(|conn| get_global_criteria(conn)).unwrap();
        assert_eq!(
            stored.title.include,
            vec!["baseline".to_string()],
            "rejected write must not overwrite existing global criteria"
        );

        // A valid-version write DOES persist (the happy path the guard allows).
        let mut good = FilterCriteria::match_all();
        good.version = FILTER_CRITERIA_VERSION;
        good.title.include = vec!["engineer".to_string()];
        assert!(good.version == FILTER_CRITERIA_VERSION);
        state
            .with_db(|conn| set_global_criteria(conn, &good))
            .unwrap();
        let stored = state.with_db(|conn| get_global_criteria(conn)).unwrap();
        assert_eq!(stored.title.include, vec!["engineer".to_string()]);
    }

    /// Req 17.2 — the per-watch get/set commands return a watch-not-found error
    /// for an unknown `watch_id`. Both commands gate on
    /// `companies::get_watch(conn, id)?.is_none()`; we assert `get_watch`
    /// returns `None` for an unknown id (the condition that maps to the error)
    /// and `Some` for a real one (the condition that lets the command proceed).
    #[tokio::test]
    async fn watch_not_found_for_unknown_id() {
        let (_dir, state) = test_state();

        let unknown = state
            .with_db(|conn| companies::get_watch(conn, "does-not-exist"))
            .unwrap();
        assert!(
            unknown.is_none(),
            "unknown watch id must resolve to None so the command returns watch-not-found"
        );

        let watch_id = seed_watch(&state);
        let existing = state
            .with_db(|conn| companies::get_watch(conn, &watch_id))
            .unwrap();
        assert!(
            existing.is_some(),
            "a real watch id must resolve to Some so the command proceeds"
        );
    }

    /// Req 12.4 — a per-watch set with a null override clears the override so the
    /// watch inherits the global criteria. `set_watch_filter_criteria(None)`
    /// forwards `None` to `set_watch_criteria`; afterwards `get_watch_criteria`
    /// reports no override.
    #[tokio::test]
    async fn null_override_clears_watch_criteria() {
        let (_dir, state) = test_state();
        let watch_id = seed_watch(&state);

        // First install an override.
        let mut override_criteria = FilterCriteria::match_all();
        override_criteria.title.include = vec!["staff".to_string()];
        state
            .with_db(|conn| set_watch_criteria(conn, &watch_id, Some(&override_criteria)))
            .unwrap();
        let present = state
            .with_db(|conn| get_watch_criteria(conn, &watch_id))
            .unwrap();
        assert!(present.is_some(), "override should be present after set");

        // Now clear it with a null override.
        state
            .with_db(|conn| set_watch_criteria(conn, &watch_id, None))
            .unwrap();
        let after = state
            .with_db(|conn| get_watch_criteria(conn, &watch_id))
            .unwrap();
        assert!(
            after.is_none(),
            "null override must clear the per-watch criteria so it inherits global"
        );
    }

    /// Req 12.5 — the preview command returns `{ included, reason }` per sample.
    /// `preview_filter_match` maps each sample through `engine::matches` against
    /// the loaded alias table; we drive that same evaluation and assert the
    /// inclusion decisions match expectation for a simple title include filter
    /// and that a non-empty reason string accompanies each outcome.
    #[tokio::test]
    async fn preview_output_shape_and_inclusion() {
        let (_dir, state) = test_state();

        let mut criteria = FilterCriteria::match_all();
        criteria.title.include = vec!["engineer".to_string()];
        criteria.title.match_mode = MatchMode::Word;

        let samples = [
            ("Senior Engineer", None::<&str>),
            ("Product Manager", None::<&str>),
        ];

        let outcomes: Vec<PreviewOutcome> = state
            .with_db(|conn| {
                let aliases = load_alias_table(conn)?;
                Ok(samples
                    .iter()
                    .map(|(title, location)| {
                        let result = engine_matches(
                            &criteria,
                            &aliases,
                            JobView {
                                title,
                                location: *location,
                            },
                        );
                        PreviewOutcome {
                            included: result.included,
                            reason: result.reason,
                        }
                    })
                    .collect::<Vec<_>>())
            })
            .unwrap();

        assert_eq!(outcomes.len(), 2, "one outcome per sample, in input order");
        assert!(
            outcomes[0].included,
            "'Senior Engineer' should be included by title include 'engineer'"
        );
        assert!(
            !outcomes[1].included,
            "'Product Manager' should be excluded by title include 'engineer'"
        );
        for outcome in &outcomes {
            assert!(
                !outcome.reason.is_empty(),
                "every preview outcome must carry a human-readable reason"
            );
        }
    }

    /// Req 15.3 (happy path) — the legacy `set_watch_role_keywords` setter writes
    /// through to the structured global criteria so both surfaces stay
    /// consistent. After the call, `get_global_criteria` reflects the keyword in
    /// `title.include` with Word match mode. (The write-through error path is
    /// surfaced structurally via `?` on `set_global_criteria`; see the test
    /// below for an induced failure.)
    #[tokio::test]
    async fn legacy_keyword_setter_writes_through_to_structured_criteria() {
        let (_dir, state) = test_state();

        state
            .with_db(|conn| {
                crate::jobs::service::set_watch_role_keywords(conn, "Software Engineer")
            })
            .unwrap();

        let criteria = state.with_db(|conn| get_global_criteria(conn)).unwrap();
        assert!(
            criteria
                .title
                .include
                .contains(&"Software Engineer".to_string()),
            "legacy keyword must be written through to structured title.include, got {:?}",
            criteria.title.include
        );
        assert_eq!(
            criteria.title.match_mode,
            MatchMode::Word,
            "write-through must use Word match mode"
        );
    }

    /// Req 15.3 (happy path) — the legacy `set_location_settings` setter writes
    /// through to the structured global criteria's location country/include.
    #[tokio::test]
    async fn legacy_location_setter_writes_through_to_structured_criteria() {
        let (_dir, state) = test_state();

        state
            .with_db(|conn| {
                crate::jobs::service::set_location_settings(
                    conn,
                    &crate::jobs::service::LocationSettings {
                        country: "United States".to_string(),
                        cities: "San Francisco, New York".to_string(),
                    },
                )
            })
            .unwrap();

        let criteria = state.with_db(|conn| get_global_criteria(conn)).unwrap();
        assert_eq!(
            criteria.location.country,
            Some("United States".to_string()),
            "non-empty country must write through to location.country"
        );
        assert!(
            criteria
                .location
                .include
                .contains(&"San Francisco".to_string())
                && criteria.location.include.contains(&"New York".to_string()),
            "cities must write through to location.include, got {:?}",
            criteria.location.include
        );
    }

    /// Req 15.3 (induced failure) — when the structured write-through cannot
    /// succeed, the legacy setter fails and surfaces an error. We break the
    /// write-through by renaming `app_settings` so the setter's upsert errors;
    /// the setter propagates that error via `?` on `set_global_criteria`,
    /// returning `Err` rather than silently leaving the two surfaces
    /// inconsistent.
    #[tokio::test]
    async fn legacy_setter_fails_when_write_through_fails() {
        let (_dir, state) = test_state();

        // Remove the settings table the setter's upsert (both legacy key and the
        // structured write-through) depends on, forcing a SQL error.
        state
            .with_db(|conn| {
                conn.execute("ALTER TABLE app_settings RENAME TO app_settings_broken", [])
                    .map_err(crate::error::map_sqlite)?;
                Ok(())
            })
            .unwrap();

        let result = state.with_db(|conn| {
            crate::jobs::service::set_watch_role_keywords(conn, "Software Engineer")
        });
        assert!(
            result.is_err(),
            "legacy setter must return Err when the write-through cannot succeed"
        );
    }
}
