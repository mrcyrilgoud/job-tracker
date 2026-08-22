mod ats;
mod commands;
mod companies;
mod db;
mod documents;
mod error;
mod gmail;
mod jobs;
mod models;
mod runner;
mod util;

use std::path::PathBuf;

use tauri::Manager;

use crate::db::{migrate_legacy_data, resolve_data_dir, AppState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let app_data = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            let paths = resolve_data_dir(Some(app_data.clone()));

            // Shared-path mode: in debug we use repo data/. Only copy legacy → app_data
            // when explicitly leaving shared mode (release without JOB_TRACKER_DATA_DIR).
            if !cfg!(debug_assertions) && std::env::var_os("JOB_TRACKER_DATA_DIR").is_none() {
                let legacy = std::env::current_dir()
                    .ok()
                    .map(|cwd| cwd.join("data"))
                    .unwrap_or_else(|| PathBuf::from("data"));
                if let Err(e) = migrate_legacy_data(&legacy, &paths) {
                    log::warn!("legacy data migration skipped/failed: {e}");
                }
            }

            let state = AppState::open(paths)?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_jobs_cmd,
            commands::preview_job_url,
            commands::create_job,
            commands::get_job,
            commands::update_job_cmd,
            commands::toggle_job_favorite_cmd,
            commands::set_job_favorite_cmd,
            commands::check_job_posting,
            commands::list_companies,
            commands::list_open_watch_positions_cmd,
            commands::create_company,
            commands::create_watch,
            commands::delete_watch,
            commands::sync_watch,
            commands::check_careers,
            commands::dismiss_review,
            commands::approve_watch_job_cmd,
            commands::dismiss_watch_job_cmd,
            commands::save_open_watch_job_cmd,
            commands::reset_dismissed_watch_job_cmd,
            commands::list_documents,
            commands::import_document,
            commands::attach_document,
            commands::detach_document,
            commands::open_document,
            commands::csv_status,
            commands::csv_export,
            commands::csv_import,
            commands::csv_config,
            commands::csv_path_status,
            commands::csv_configure,
            commands::csv_reset_config,
            commands::gmail_status,
            commands::gmail_configure,
            commands::gmail_connect,
            commands::gmail_disconnect,
            commands::gmail_poll,
            commands::gmail_triage,
            commands::run_jobs_cycle_cmd,
            commands::check_all_postings_cmd,
            commands::get_data_dir,
            commands::get_watch_role_keywords,
            commands::set_watch_role_keywords,
            commands::get_location_settings_cmd,
            commands::set_location_settings_cmd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

pub use runner::run_jobs_cli;
