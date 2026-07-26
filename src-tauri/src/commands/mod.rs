use std::collections::HashMap;

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
use crate::jobs::check_active::{apply_posting_check, fetch_posting_state, load_job_check_context};
use crate::jobs::csv::{
    export_jobs_csv, get_jobs_csv_status, import_jobs_csv, schedule_export_jobs_csv, ImportMode,
};
use crate::jobs::service::{
    get_job_detail, get_pipeline_counts, get_weekly_activity, list_jobs, resolve_title_from_url,
    update_job, create_job_from_url, JobFilters, UpdateJobInput,
};
use crate::runner::run_jobs_cycle;

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
}

#[tauri::command]
pub async fn create_job(
    state: State<'_, AppState>,
    input: CreateJobArgs,
) -> AppResult<serde_json::Value> {
    let title = resolve_title_from_url(&input.url, input.title.as_deref()).await;
    let result = state.with_db(|conn| {
        let (job, company) = create_job_from_url(
            conn,
            &input.url,
            &title,
            input.company_name.as_deref(),
            input.status.as_deref(),
            input.applied_at.as_deref(),
            input.notes.as_deref(),
            input.location.as_deref(),
        )?;
        schedule_export_jobs_csv(conn, &state.paths.jobs_csv_path);
        Ok(serde_json::json!({ "job": job, "company": company }))
    })?;
    Ok(result)
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
pub async fn get_data_dir(state: State<'_, AppState>) -> AppResult<String> {
    Ok(state.paths.data_dir.display().to_string())
}

#[allow(dead_code)]
fn _hashmap_ty(_: HashMap<String, i64>) {}
