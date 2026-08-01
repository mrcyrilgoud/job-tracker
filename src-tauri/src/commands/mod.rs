use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use serde::Deserialize;
use tauri::{AppHandle, State};
use tauri_plugin_shell::ShellExt;

use crate::ats;
use crate::ats::careers::{apply_careers_check, company_careers_url, fetch_careers_hash};
use crate::ats::sync::{apply_watch_sync, fetch_remote_jobs};
use crate::companies;
use crate::db::AppState;
use crate::documents;
use crate::error::AppResult;
use crate::gmail;
use crate::jobs::board_discovery::discover_from_url;
use crate::jobs::check_active::{apply_posting_check, fetch_posting_state, load_job_check_context};
use crate::jobs::csv::{
    export_jobs_csv, get_jobs_csv_status, import_jobs_csv, schedule_export_jobs_csv, ImportMode,
};
use crate::jobs::metadata::{resolve_job_metadata, JobMetadata};
use crate::jobs::service::{
    approve_watch_job, create_job_from_url_with_careers, dismiss_watch_job, get_job_detail,
    get_pipeline_counts, get_weekly_activity, list_jobs, resolve_title_from_url, update_job,
    JobFilters, UpdateJobInput,
};
use crate::runner::{check_all_postings, run_jobs_cycle};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListJobsArgs {
    pub status: Option<String>,
    pub company_id: Option<String>,
    pub posting_state: Option<String>,
    pub search: Option<String>,
    pub new_from_watch: Option<bool>,
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
        new_from_watch: None,
    });
    state.with_db(|conn| {
        let jobs = list_jobs(
            conn,
            JobFilters {
                status: filters.status,
                company_id: filters.company_id,
                posting_state: filters.posting_state,
                search: filters.search,
                new_from_watch: filters.new_from_watch,
            },
        )?;
        let counts = get_pipeline_counts(conn)?;
        let weekly = get_weekly_activity(conn)?;
        Ok(serde_json::json!({
            "jobs": jobs,
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
        Some(careers_url) => Some(discover_from_url(careers_url)?.careers_url.ok_or_else(
            || crate::error::AppError::from("Confirmed careers URL must point to a careers page"),
        )?),
        None => None,
    };
    let title = resolve_title_from_url(&input.url, input.title.as_deref()).await;
    let result = state.with_db(|conn| {
        let (job, company) = create_job_from_url_with_careers(
            conn,
            &input.url,
            &title,
            input.company_name.as_deref(),
            input.status.as_deref(),
            input.applied_at.as_deref(),
            input.notes.as_deref(),
            input.location.as_deref(),
            confirmed_careers_url.as_deref(),
        )?;
        let watch = confirmed_board
            .as_ref()
            .map(|(provider, board_slug)| {
                companies::insert_watch(conn, &company.id, provider, board_slug)
            })
            .transpose()?;
        schedule_export_jobs_csv(conn, &state.paths.jobs_csv_path);
        Ok(serde_json::json!({ "job": job, "company": company, "watch": watch }))
    })?;
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
                    assert_eq!((provider.as_str(), slug.as_str()), ("ashby", "bayesianhealth"));
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
                    assert_eq!((provider.as_str(), slug.as_str()), ("ashby", "bayesianhealth"));
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
                Box::pin(async {
                    Err(crate::error::AppError::from("Ashby board unavailable"))
                })
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
    state.with_db(|conn| {
        let detail = update_job(conn, &id, updates)?;
        schedule_export_jobs_csv(conn, &state.paths.jobs_csv_path);
        Ok(serde_json::json!({ "detail": detail }))
    })
}

#[tauri::command]
pub async fn check_job_posting(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<serde_json::Value> {
    let (url, previous) = state.with_db(|conn| load_job_check_context(conn, &id))?;
    let (posting_state, last_check_result) = fetch_posting_state(&url).await;
    state.with_db(|conn| {
        let result =
            apply_posting_check(conn, &id, &previous, &posting_state, &last_check_result)?;
        Ok(serde_json::json!(result))
    })
}

#[tauri::command]
pub async fn list_companies(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let rows = companies::list_companies_with_watches(conn)?;
        Ok(serde_json::json!({ "companies": rows }))
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
        let company =
            companies::create_company(conn, &input.name, input.careers_url.as_deref())?;
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
        let watch = companies::insert_watch(
            conn,
            &input.company_id,
            &input.provider,
            &input.board_slug,
        )?;
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
    let (provider, board_slug) = state.with_db(|conn| {
        let watch = companies::get_watch(conn, &watch_id)?
            .ok_or_else(|| crate::error::AppError::from("Watch not found"))?;
        Ok((watch.provider, watch.board_slug))
    })?;
    let remote = fetch_remote_jobs(&provider, &board_slug)
        .await
        .map_err(|e| e.to_string());
    state.with_db(|conn| apply_watch_sync(conn, &watch_id, remote))
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
            state.with_db(|conn| apply_careers_check(conn, &company_id, &name, &hash, &text))
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
    state.with_db(|conn| {
        let job = approve_watch_job(conn, &job_id)?;
        Ok(serde_json::json!({ "job": job }))
    })
}

#[tauri::command]
pub async fn dismiss_watch_job_cmd(
    state: State<'_, AppState>,
    job_id: String,
) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let job = dismiss_watch_job(conn, &job_id)?;
        Ok(serde_json::json!({ "job": job }))
    })
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
    state.with_db(|conn| {
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
    state.with_db(|conn| {
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
pub async fn detach_document(
    state: State<'_, AppState>,
    attachment_id: String,
) -> AppResult<()> {
    state.with_db(|conn| documents::detach_document(conn, &attachment_id))
}

#[tauri::command]
pub async fn open_document(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: String,
) -> AppResult<()> {
    let path = state.with_db(|conn| {
        let (_doc, path) =
            documents::get_document_file_path(conn, &state.paths.documents_dir, &document_id)?;
        Ok(path)
    })?;
    app.shell()
        .open(path.to_string_lossy().to_string(), None)
        .map_err(|e| crate::error::AppError::from(e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub async fn csv_status(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let status = get_jobs_csv_status(conn, &state.paths.jobs_csv_path)?;
        Ok(serde_json::json!(status))
    })
}

#[tauri::command]
pub async fn csv_export(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let result = export_jobs_csv(conn, &state.paths.jobs_csv_path, None)?;
        Ok(serde_json::json!(result))
    })
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
    let result = import_jobs_csv(
        &paths.db_path,
        &paths.jobs_csv_path,
        input.content.as_deref(),
        input.dry_run.unwrap_or(false),
        mode,
    )?;
    Ok(serde_json::json!(result))
}

#[tauri::command]
pub async fn gmail_status(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        let config = gmail::get_gmail_config(conn)?;
        let connected = gmail::is_gmail_connected()?;
        let pending = gmail::list_pending_email_matches(conn)?;
        Ok(serde_json::json!({
            "connected": connected,
            "configured": config.get("clientId").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false),
            "redirectUri": config.get("redirectUri"),
            "pending": pending
        }))
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailConfigArgs {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: Option<String>,
}

#[tauri::command]
pub async fn gmail_configure(
    state: State<'_, AppState>,
    input: GmailConfigArgs,
) -> AppResult<()> {
    state.with_db(|conn| {
        gmail::save_gmail_config(
            conn,
            &input.client_id,
            &input.client_secret,
            input.redirect_uri.as_deref(),
        )
    })
}

#[tauri::command]
pub async fn gmail_connect(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<serde_json::Value> {
    let started = state.with_db(|conn| gmail::begin_gmail_oauth(conn))?;
    if let Some(url) = started.get("url").and_then(|v| v.as_str()) {
        app.shell()
            .open(url.to_string(), None)
            .map_err(|e| crate::error::AppError::from(e.to_string()))?;
    }
    let paths = state.paths.clone();
    let result = gmail::complete_gmail_oauth_from_pending(&paths.db_path).await?;
    Ok(result)
}

#[tauri::command]
pub async fn gmail_disconnect() -> AppResult<()> {
    gmail::disconnect_gmail()
}

#[tauri::command]
pub async fn gmail_poll(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    gmail::poll_gmail_matches(&state.paths.db_path).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriageArgs {
    pub match_id: String,
    pub job_id: Option<String>,
}

#[tauri::command]
pub async fn gmail_triage(
    state: State<'_, AppState>,
    input: TriageArgs,
) -> AppResult<serde_json::Value> {
    state.with_db(|conn| {
        gmail::confirm_email_match(conn, &input.match_id, input.job_id.as_deref())
    })
}

#[tauri::command]
pub async fn run_jobs_cycle_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<serde_json::Value> {
    // In-memory single-flight check; drop guard before await (MutexGuard is !Send).
    // File flock inside run_jobs_cycle covers cross-process exclusivity.
    {
        let guard = state.runner_lock.try_lock();
        if guard.is_none() {
            return Err(crate::error::AppError::from(
                "Jobs runner is already in progress",
            ));
        }
    }
    let paths = state.paths.clone();
    run_jobs_cycle(&paths, Some(app)).await
}

#[tauri::command]
pub async fn check_all_postings_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<serde_json::Value> {
    // In-memory single-flight check; drop guard before await (MutexGuard is !Send).
    // File flock inside check_all_postings covers cross-process exclusivity.
    {
        let guard = state.runner_lock.try_lock();
        if guard.is_none() {
            return Err(crate::error::AppError::from(
                "Jobs runner is already in progress",
            ));
        }
    }
    let paths = state.paths.clone();
    check_all_postings(&paths, Some(app)).await
}

#[tauri::command]
pub async fn get_data_dir(state: State<'_, AppState>) -> AppResult<String> {
    Ok(state.paths.data_dir.display().to_string())
}

#[allow(dead_code)]
fn _hashmap_ty(_: HashMap<String, i64>) {}
