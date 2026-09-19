use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use std::io::{Read, Seek, SeekFrom, Write};

use crate::cli::args::{
    AddArgs, DescriptionArgs, GetArgs, ListArgs, NoteArgs, UpdateArgs, WatchCommands, WatchListArgs,
};
use crate::cli::output::{
    format_job_detail, format_jobs_table, format_stats, format_watch_positions, print_json,
    print_raw_json,
};
use crate::db::paths::DataPaths;
use crate::error::{map_sqlite, AppError, AppResult};
use crate::jobs::csv::export_jobs_csv;
use crate::jobs::csv_config::active_csv_path;
use crate::jobs::metadata::resolve_job_metadata;
use crate::jobs::service::{
    add_job_event, archive_job, create_job_from_url_with_careers, dismiss_watch_job, get_job_by_id,
    get_job_detail, get_pipeline_counts, get_weekly_activity, list_jobs, map_job,
    reset_dismissed_watch_job, resolve_title_from_url, save_open_watch_job, set_job_favorite,
    unarchive_job, update_job, JobFilters, UpdateJobInput,
};
use crate::models::{is_job_status, JobListItem};
use crate::runner::run_jobs_cycle;
use crate::util::normalize_canonical_url;

/// Resolve a user-supplied target (UUID, prefix, or URL) to a job ID.
pub fn resolve_target_job_id(conn: &Connection, target: &str) -> AppResult<String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Err(AppError::from("Job target identifier cannot be empty"));
    }

    // 1. Exact ID match
    if let Some(id) = conn
        .query_row("SELECT id FROM jobs WHERE id = ?1", params![trimmed], |r| {
            r.get::<_, String>(0)
        })
        .optional()
        .map_err(map_sqlite)?
    {
        return Ok(id);
    }

    // 2. Canonical URL exact match
    if let Ok(canonical) = normalize_canonical_url(trimmed) {
        if let Some(id) = conn
            .query_row(
                "SELECT id FROM jobs WHERE canonical_url = ?1",
                params![canonical],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(map_sqlite)?
        {
            return Ok(id);
        }
    }

    // 3. ID prefix match
    let matches: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM jobs WHERE id LIKE ?1 LIMIT 5")?;
        let pattern = format!("{trimmed}%");
        let rows = stmt.query_map(params![pattern], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    if matches.len() == 1 {
        return Ok(matches[0].clone());
    } else if matches.len() > 1 {
        return Err(AppError::from(format!(
            "Ambiguous job prefix '{trimmed}' matches multiple jobs: {}",
            matches.join(", ")
        )));
    }

    // 4. URL substring match
    let url_matches: Vec<(String, String, String)> = {
        let mut stmt = conn.prepare("SELECT j.id, c.name, j.title FROM jobs j JOIN companies c ON j.company_id = c.id WHERE j.url LIKE ?1 LIMIT 5")?;
        let pattern = format!("%{trimmed}%");
        let rows = stmt.query_map(params![pattern], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    if url_matches.len() == 1 {
        return Ok(url_matches[0].0.clone());
    } else if url_matches.len() > 1 {
        let list: Vec<String> = url_matches
            .iter()
            .map(|(id, comp, title)| format!("{} ({} - {})", &id[..8.min(id.len())], comp, title))
            .collect();
        return Err(AppError::from(format!(
            "Ambiguous search '{trimmed}' matches multiple jobs:\n  {}",
            list.join("\n  ")
        )));
    }

    Err(AppError::from(format!("No job found matching '{trimmed}'")))
}

fn sync_csv_after_mutation(conn: &Connection, paths: &DataPaths) {
    if let Ok(csv_path) = active_csv_path(conn, &paths.jobs_csv_path) {
        if let Err(e) = export_jobs_csv(conn, &csv_path, None) {
            log::warn!("CSV export after CLI mutation failed: {e}");
        }
    }
}

pub fn handle_list(
    conn: &Connection,
    _paths: &DataPaths,
    args: ListArgs,
    json: bool,
) -> AppResult<()> {
    let filters = JobFilters {
        status: args.status,
        company_id: args.company,
        posting_state: None,
        search: args.search,
        location: args.location,
        new_from_watch: None,
        is_favorite: if args.favorites { Some(true) } else { None },
        is_archived: if args.archived {
            Some(true)
        } else {
            Some(false)
        },
        limit: args.limit,
    };

    let mut jobs = list_jobs(conn, filters)?;
    if let Some(limit) = args.limit {
        jobs.truncate(limit);
    }

    if json {
        print_json(&jobs);
    } else {
        format_jobs_table(&jobs);
    }
    Ok(())
}

pub fn handle_get(conn: &Connection, args: GetArgs, json: bool) -> AppResult<()> {
    let job_id = resolve_target_job_id(conn, &args.target)?;
    let detail = get_job_detail(conn, &job_id)?
        .ok_or_else(|| AppError::from(format!("Job with ID {job_id} not found")))?;

    if json {
        print_json(&detail);
    } else {
        format_job_detail(&detail);
    }
    Ok(())
}

pub async fn handle_add(
    conn: &mut Connection,
    paths: &DataPaths,
    args: AddArgs,
    json: bool,
    quiet: bool,
) -> AppResult<()> {
    let url = args.url.trim();
    if url.is_empty() {
        return Err(AppError::from("URL cannot be empty"));
    }

    // Resolve metadata via scraper
    let (resolved_title, resolved_company, resolved_description, resolved_location) = {
        let meta = resolve_job_metadata(url).await.ok();
        let title = if let Some(t) = args.title.filter(|s| !s.trim().is_empty()) {
            t
        } else {
            resolve_title_from_url(url, None).await
        };
        let company = if let Some(c) = args.company.filter(|s| !s.trim().is_empty()) {
            Some(c)
        } else {
            meta.as_ref().and_then(|m| m.company_name.clone())
        };
        let description = if let Some(d) = args.description.filter(|s| !s.trim().is_empty()) {
            Some(d)
        } else {
            meta.as_ref().and_then(|m| m.description.clone())
        };
        let location = args.location.filter(|s| !s.trim().is_empty());
        (title, company, description, location)
    };

    let applied_date = match args.applied_at.as_deref() {
        Some("today") | Some("now") => Some(chrono::Local::now().format("%Y-%m-%d").to_string()),
        Some(d) if !d.trim().is_empty() => Some(d.trim().to_string()),
        _ => None,
    };

    let status = if applied_date.is_some() && args.status == "wishlist" {
        "applied"
    } else {
        args.status.as_str()
    };

    if !is_job_status(status) {
        return Err(AppError::from(format!(
            "Invalid status '{status}'. Valid statuses: wishlist, applied, interviewing, offer, rejected, withdrawn, closed"
        )));
    }

    let (mut job, company) = create_job_from_url_with_careers(
        conn,
        url,
        &resolved_title,
        resolved_company.as_deref(),
        Some(status),
        applied_date.as_deref(),
        args.notes.as_deref(),
        resolved_description.as_deref(),
        resolved_location.as_deref(),
        None,
    )?;

    if args.favorite {
        let _ = set_job_favorite(conn, &job.id, true);
        job.is_favorite = true;
    }

    sync_csv_after_mutation(conn, paths);

    if json {
        let out = json!({
            "job": job,
            "company": company
        });
        print_raw_json(&out);
    } else if !quiet {
        println!("✓ Added job: {} — {}", job.title, company.name);
        println!("  ID:     {}", job.id);
        println!("  Status: {}", job.status);
        println!("  URL:    {}", job.url);
    }
    Ok(())
}

pub fn handle_update(
    conn: &Connection,
    paths: &DataPaths,
    args: UpdateArgs,
    json: bool,
    quiet: bool,
) -> AppResult<()> {
    let job_id = resolve_target_job_id(conn, &args.target)?;

    if args.archive {
        let detail = archive_job(conn, &job_id)?;
        sync_csv_after_mutation(conn, paths);
        if json {
            print_json(&detail);
        } else if !quiet {
            println!("✓ Archived job: {}", detail.job.title);
        }
        return Ok(());
    }

    if args.unarchive {
        let detail = unarchive_job(conn, &job_id, None)?;
        sync_csv_after_mutation(conn, paths);
        if json {
            print_json(&detail);
        } else if !quiet {
            println!("✓ Unarchived job: {}", detail.job.title);
        }
        return Ok(());
    }

    let applied_date = match args.applied_at.as_deref() {
        Some("today") | Some("now") => {
            Some(Some(chrono::Local::now().format("%Y-%m-%d").to_string()))
        }
        Some(d) if !d.trim().is_empty() => Some(Some(d.trim().to_string())),
        _ => None,
    };

    if let Some(st) = &args.status {
        if !is_job_status(st) {
            return Err(AppError::from(format!(
                "Invalid status '{st}'. Valid statuses: wishlist, applied, interviewing, offer, rejected, withdrawn, closed"
            )));
        }
    }

    let existing = get_job_by_id(conn, &job_id)?.ok_or_else(|| AppError::from("Job not found"))?;

    let notes = if let Some(n) = args.notes {
        Some(Some(n))
    } else if let Some(app) = args.append_note {
        let existing_notes = existing.notes.unwrap_or_default();
        let combined = if existing_notes.trim().is_empty() {
            app
        } else {
            format!("{}\n{}", existing_notes, app)
        };
        Some(Some(combined))
    } else {
        None
    };

    let description = if args.clear_description {
        Some(None)
    } else if let Some(d) = args.description {
        Some(Some(d))
    } else {
        None
    };

    let location = args.location.map(Some);

    let input = UpdateJobInput {
        title: args.title,
        company_name: args.company,
        status: args.status,
        applied_at: applied_date,
        notes,
        description,
        location,
        url: None,
        is_new_from_watch: None,
        is_favorite: if args.favorite {
            Some(true)
        } else if args.unfavorite {
            Some(false)
        } else {
            None
        },
    };

    let updated = update_job(conn, &job_id, input)?;
    sync_csv_after_mutation(conn, paths);

    if json {
        print_json(&updated);
    } else if !quiet {
        println!("✓ Updated job: {}", updated.job.title);
        println!("  Status:   {}", updated.job.status);
        println!(
            "  Favorite: {}",
            if updated.job.is_favorite {
                "Yes ★"
            } else {
                "No"
            }
        );
    }
    Ok(())
}

pub fn handle_note(
    conn: &Connection,
    paths: &DataPaths,
    args: NoteArgs,
    json: bool,
    quiet: bool,
) -> AppResult<()> {
    let job_id = resolve_target_job_id(conn, &args.target)?;
    let note_text = args.note.trim();
    if note_text.is_empty() {
        return Err(AppError::from("Note cannot be empty"));
    }

    let event = add_job_event(conn, &job_id, "note_added", Some(note_text))?;

    // Also append to job.notes
    let existing = get_job_by_id(conn, &job_id)?.ok_or_else(|| AppError::from("Job not found"))?;
    let existing_notes = existing.notes.unwrap_or_default();
    let combined = if existing_notes.trim().is_empty() {
        note_text.to_string()
    } else {
        format!("{}\n{}", existing_notes, note_text)
    };

    let _ = update_job(
        conn,
        &job_id,
        UpdateJobInput {
            title: None,
            company_name: None,
            status: None,
            applied_at: None,
            notes: Some(Some(combined)),
            description: None,
            location: None,
            url: None,
            is_new_from_watch: None,
            is_favorite: None,
        },
    )?;

    sync_csv_after_mutation(conn, paths);

    if json {
        print_json(&event);
    } else if !quiet {
        println!(
            "✓ Added note to job {}: \"{}\"",
            &job_id[..8.min(job_id.len())],
            note_text
        );
    }
    Ok(())
}

fn open_interactive_editor(initial_text: &str) -> AppResult<String> {
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "nano".to_string()
            }
        });

    let mut temp_file = tempfile::Builder::new()
        .prefix("job-tracker-desc-")
        .suffix(".txt")
        .tempfile()
        .map_err(|e| AppError::from(format!("Failed to create temporary file: {e}")))?;

    if !initial_text.is_empty() {
        temp_file.write_all(initial_text.as_bytes()).map_err(|e| {
            AppError::from(format!("Failed to write initial text to temp file: {e}"))
        })?;
        temp_file
            .flush()
            .map_err(|e| AppError::from(format!("Failed to flush temp file: {e}")))?;
    }

    let status = std::process::Command::new(&editor)
        .arg(temp_file.path())
        .status()
        .map_err(|e| AppError::from(format!("Failed to launch editor '{editor}': {e}")))?;

    if !status.success() {
        return Err(AppError::from(format!(
            "Editor '{editor}' exited with non-zero status"
        )));
    }

    temp_file
        .seek(SeekFrom::Start(0))
        .map_err(|e| AppError::from(format!("Failed to seek temp file: {e}")))?;
    let mut updated = String::new();
    temp_file
        .read_to_string(&mut updated)
        .map_err(|e| AppError::from(format!("Failed to read edited temp file: {e}")))?;

    Ok(updated)
}

pub fn handle_description(
    conn: &Connection,
    paths: &DataPaths,
    args: DescriptionArgs,
    json: bool,
    quiet: bool,
) -> AppResult<()> {
    let job_id = resolve_target_job_id(conn, &args.target)?;
    let existing = get_job_by_id(conn, &job_id)?.ok_or_else(|| AppError::from("Job not found"))?;

    // 1. Show mode
    if args.show {
        if json {
            let out = json!({
                "id": job_id,
                "title": existing.title,
                "description": existing.description
            });
            print_raw_json(&out);
        } else if let Some(desc) = &existing.description {
            if !desc.trim().is_empty() {
                println!("{desc}");
            } else if !quiet {
                println!("(Description is empty)");
            }
        } else if !quiet {
            println!(
                "No description set for job {} ({})",
                existing.title,
                &job_id[..8.min(job_id.len())]
            );
        }
        return Ok(());
    }

    // 2. Clear mode
    if args.clear {
        let updated = update_job(
            conn,
            &job_id,
            UpdateJobInput {
                description: Some(None),
                ..Default::default()
            },
        )?;
        sync_csv_after_mutation(conn, paths);

        if json {
            print_json(&updated);
        } else if !quiet {
            println!("✓ Cleared description for job: {}", updated.job.title);
        }
        return Ok(());
    }

    // 3. Determine new description text from arguments, file, stdin, or interactive editor
    let new_description = if let Some(text) = args.description {
        text
    } else if let Some(file_path) = args.file {
        std::fs::read_to_string(&file_path).map_err(|e| {
            AppError::from(format!("Failed to read file {}: {e}", file_path.display()))
        })?
    } else if args.stdin {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|e| AppError::from(format!("Failed to read from stdin: {e}")))?;
        buffer
    } else {
        // Open interactive editor
        let initial_text = existing.description.as_deref().unwrap_or("");
        let edited = open_interactive_editor(initial_text)?;
        if edited == initial_text {
            if !quiet && !json {
                println!("No changes made to description.");
            }
            return Ok(());
        }
        edited
    };

    let desc_to_save = if new_description.trim().is_empty() {
        None
    } else {
        Some(new_description)
    };

    let updated = update_job(
        conn,
        &job_id,
        UpdateJobInput {
            description: Some(desc_to_save),
            ..Default::default()
        },
    )?;

    sync_csv_after_mutation(conn, paths);

    if json {
        print_json(&updated);
    } else if !quiet {
        println!("✓ Updated description for job: {}", updated.job.title);
    }
    Ok(())
}

pub fn handle_stats(conn: &Connection, json: bool) -> AppResult<()> {
    let counts = get_pipeline_counts(conn)?;
    let weekly = get_weekly_activity(conn)?;

    if json {
        let out = json!({
            "counts": counts,
            "weeklyActivity": weekly
        });
        print_raw_json(&out);
    } else {
        format_stats(&counts, &weekly);
    }
    Ok(())
}

fn load_watch_positions(conn: &Connection, args: WatchListArgs) -> AppResult<Vec<JobListItem>> {
    let mut sql = String::from(
        "SELECT j.id, j.company_id, j.title, j.url, j.canonical_url, j.source_external_id, j.status, j.applied_at, j.posting_state, j.last_checked_at, j.last_check_result, j.source, j.notes, j.description, j.location, j.is_new_from_watch, j.watch_disposition, j.missing_from_sync_count, j.is_favorite, j.created_at, j.updated_at, c.name
         FROM jobs j INNER JOIN companies c ON j.company_id = c.id
         WHERE j.posting_state = 'active'
           AND j.source IN ('greenhouse', 'lever', 'ashby')"
    );
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(company_filter) = &args.company {
        sql.push_str(" AND (j.company_id = ? OR c.name LIKE ?)");
        values.push(Box::new(company_filter.clone()));
        values.push(Box::new(format!("%{company_filter}%")));
    }

    if let Some(prov) = &args.provider {
        sql.push_str(" AND j.source = ?");
        values.push(Box::new(prov.to_lowercase()));
    }

    if args.dismissed {
        sql.push_str(" AND j.watch_disposition = 'dismissed'");
    } else if args.new_only {
        sql.push_str(" AND (j.is_new_from_watch = 1 OR j.watch_disposition = 'new')");
    } else if !args.all {
        // Default: only show active discoveries that are new or in review
        sql.push_str(" AND (j.watch_disposition IS NULL OR j.watch_disposition != 'dismissed')");
    }

    sql.push_str(" ORDER BY j.updated_at DESC");

    let mut stmt = conn.prepare(&sql).map_err(map_sqlite)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
    let rows = stmt.query_map(params_ref.as_slice(), |row| {
        Ok(JobListItem {
            job: map_job(row)?,
            company_name: row.get(21)?,
        })
    })?;

    let mut positions = rows.collect::<Result<Vec<_>, _>>()?;

    if let Some(limit) = args.limit {
        positions.truncate(limit);
    }

    Ok(positions)
}

fn handle_watch_list(conn: &Connection, args: WatchListArgs, json: bool) -> AppResult<()> {
    let positions = load_watch_positions(conn, args)?;

    if json {
        print_json(&positions);
    } else {
        format_watch_positions(&positions);
    }
    Ok(())
}

pub async fn handle_watches(
    conn: &mut Connection,
    paths: &DataPaths,
    cmd: Option<WatchCommands>,
    json: bool,
    quiet: bool,
) -> AppResult<()> {
    match cmd {
        None => {
            let default_args = WatchListArgs {
                new_only: false,
                dismissed: false,
                all: false,
                provider: None,
                company: None,
                limit: None,
            };
            handle_watch_list(conn, default_args, json)?;
        }
        Some(WatchCommands::List(args)) => {
            handle_watch_list(conn, args, json)?;
        }
        Some(WatchCommands::Save(args)) => {
            let job_id = resolve_target_job_id(conn, &args.target)?;
            let saved = save_open_watch_job(conn, &job_id)?;
            sync_csv_after_mutation(conn, paths);
            if json {
                print_json(&saved);
            } else if !quiet {
                println!("✓ Saved watch position to tracked jobs: {}", saved.title);
            }
        }
        Some(WatchCommands::Dismiss(args)) => {
            let job_id = resolve_target_job_id(conn, &args.target)?;
            let dismissed = dismiss_watch_job(conn, &job_id)?;
            sync_csv_after_mutation(conn, paths);
            if json {
                print_json(&dismissed);
            } else if !quiet {
                println!("✓ Dismissed watch position: {}", dismissed.title);
            }
        }
        Some(WatchCommands::Reset(args)) => {
            let job_id = resolve_target_job_id(conn, &args.target)?;
            let reset = reset_dismissed_watch_job(conn, &job_id)?;
            sync_csv_after_mutation(conn, paths);
            if json {
                print_json(&reset);
            } else if !quiet {
                println!("✓ Reset watch position back to review: {}", reset.title);
            }
        }
        Some(WatchCommands::Sync) => {
            if !quiet && !json {
                println!("Starting ATS & background jobs sync...");
            }
            let summary = run_jobs_cycle(paths, None).await?;
            if json {
                print_raw_json(&summary);
            } else if !quiet {
                println!("✓ Sync cycle completed successfully.");
            }
        }
    }
    Ok(())
}

pub async fn handle_sync(paths: &DataPaths, json: bool, quiet: bool) -> AppResult<()> {
    if !quiet && !json {
        println!("Starting full jobs cycle...");
    }
    let summary = run_jobs_cycle(paths, None).await?;
    if json {
        print_raw_json(&summary);
    } else if !quiet {
        println!("✓ Jobs sync cycle completed successfully.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;
    use crate::util::{create_id, now_iso};

    #[test]
    fn watch_list_uses_current_job_column_layout() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let company_id = create_id();
        let job_id = create_id();
        let timestamp = now_iso();

        conn.execute(
            "INSERT INTO companies (id, name, created_at, updated_at) VALUES (?1, 'Acme', ?2, ?2)",
            params![company_id, timestamp],
        )
        .unwrap();
        conn.execute(
            r#"INSERT INTO jobs (
                id, company_id, title, url, canonical_url, source_external_id, status,
                posting_state, source, description, location, is_new_from_watch,
                watch_disposition, missing_from_sync_count, is_favorite, created_at, updated_at
            ) VALUES (
                ?1, ?2, 'Platform Engineer', 'https://example.com/jobs/1',
                'https://example.com/jobs/1', 'remote-1', 'wishlist', 'active',
                'greenhouse', 'Build distributed systems', 'Remote', 1, 'new', 0, 0, ?3, ?3
            )"#,
            params![job_id, company_id, timestamp],
        )
        .unwrap();

        let positions = load_watch_positions(
            &conn,
            WatchListArgs {
                new_only: false,
                dismissed: false,
                all: true,
                provider: None,
                company: None,
                limit: None,
            },
        )
        .unwrap();

        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].company_name, "Acme");
        assert_eq!(
            positions[0].job.description.as_deref(),
            Some("Build distributed systems")
        );
        assert_eq!(positions[0].job.location.as_deref(), Some("Remote"));
    }
}
