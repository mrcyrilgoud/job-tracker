use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use std::io::{Read, Seek, SeekFrom, Write};

use crate::cli::args::{
    AddArgs, DeleteArgs, DescriptionArgs, GetArgs, ListArgs, NoteArgs, UpdateArgs, WatchCommands,
    WatchListArgs,
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
    add_job_event, archive_job, create_job_from_url_with_careers, delete_jobs, dismiss_watch_job,
    get_job_by_id, get_job_detail, get_pipeline_counts, get_weekly_activity, job_cols, list_jobs,
    map_job, reset_dismissed_watch_job, resolve_title_from_url,
    retain_watch_positions_matching_criteria, save_open_watch_job, set_job_favorite, unarchive_job,
    update_job, JobFilters, UpdateJobInput, JOB_COL_COUNT,
};
use crate::models::{is_job_status, JobListItem};
use crate::runner::run_jobs_cycle_trigger;
use crate::runs::model::Trigger;
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
    let explicit_status = args.status.is_some();
    let filters = JobFilters {
        status: args.status,
        company_id: args.company,
        posting_state: None,
        search: args.search,
        salary_min: None,
        salary_max: None,
        location: args.location,
        new_from_watch: None,
        is_favorite: if args.favorites { Some(true) } else { None },
        is_archived: if args.archived {
            Some(true)
        } else if explicit_status {
            // An explicit --status (e.g. closed, rejected) must not be hidden
            // by the default active-pipeline filter.
            None
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
    } else {
        args.description.map(Some)
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
        appeal: if args.clear_appeal {
            Some(None)
        } else {
            args.appeal.map(Some)
        },
        salary_min: None,
        salary_max: None,
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
        println!(
            "  Appeal:   {}",
            match updated.job.appeal {
                Some(score) => format!("{score} (1-5, 5 = most appealing)"),
                None => "— (1-5, 5 = most appealing)".to_string(),
            }
        );
    }
    Ok(())
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DeletedJob {
    id: String,
    title: String,
    company_name: String,
    status: String,
    url: String,
}

fn snapshot_job(conn: &Connection, job_id: &str) -> AppResult<DeletedJob> {
    let job = get_job_by_id(conn, job_id)?
        .ok_or_else(|| AppError::from(format!("No job found matching '{job_id}'")))?;
    let company_name: String = conn
        .query_row(
            "SELECT name FROM companies WHERE id = ?1",
            params![job.company_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite)?
        .unwrap_or_else(|| "Unknown".to_string());
    Ok(DeletedJob {
        id: job.id,
        title: job.title,
        company_name,
        status: job.status,
        url: job.url,
    })
}

fn render_deleted_jobs(jobs: &[DeletedJob]) -> String {
    jobs.iter()
        .map(|job| {
            format!(
                "  {}\n    {} — {} ({})\n    {}",
                job.id, job.title, job.company_name, job.status, job.url
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn confirms_delete_answer(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn confirm_permanent_delete(
    yes: bool,
    count: usize,
    preview: &str,
    interactive: bool,
) -> AppResult<()> {
    if yes {
        return Ok(());
    }
    if interactive {
        eprintln!("{preview}");
        eprint!("Permanently delete {count} job(s)? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if confirms_delete_answer(&answer) {
            return Ok(());
        }
        return Err(AppError::from("Delete cancelled"));
    }
    Err(AppError::from(format!(
        "Refusing to permanently delete {count} job(s) without --yes.\n{preview}\n  jt delete <id> [<id>...] --yes"
    )))
}

pub fn handle_delete(
    conn: &Connection,
    paths: &DataPaths,
    args: DeleteArgs,
    json: bool,
    quiet: bool,
    stdin_is_terminal: bool,
) -> AppResult<()> {
    if args.targets.is_empty() {
        return Err(AppError::from(
            "At least one job id is required.\n  jt delete <id> [<id>...] --yes",
        ));
    }

    let mut ids = Vec::new();
    for target in &args.targets {
        let id = resolve_target_job_id(conn, target)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }

    let jobs: Vec<DeletedJob> = ids
        .iter()
        .map(|id| snapshot_job(conn, id))
        .collect::<AppResult<_>>()?;
    let preview = render_deleted_jobs(&jobs);
    let deleted_count = jobs.len();

    if args.dry_run {
        if json {
            print_raw_json(&json!({
                "dryRun": true,
                "deletedCount": deleted_count,
                "deleted": jobs,
            }));
        } else if !quiet {
            println!("Would permanently delete {deleted_count} job(s):");
            println!("{preview}");
        }
        return Ok(());
    }

    // `--json` is the agent path. Never block it on a terminal prompt.
    let interactive = stdin_is_terminal && !json;
    confirm_permanent_delete(args.yes, deleted_count, &preview, interactive)?;

    conn.execute_batch("BEGIN IMMEDIATE").map_err(map_sqlite)?;
    if let Err(err) = delete_jobs(conn, &ids) {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(err);
    }
    if let Err(err) = conn.execute_batch("COMMIT") {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(map_sqlite(err));
    }

    sync_csv_after_mutation(conn, paths);

    if json {
        print_raw_json(&json!({
            "dryRun": false,
            "deletedCount": deleted_count,
            "deleted": jobs,
        }));
    } else if !quiet {
        println!("✓ Permanently deleted {deleted_count} job(s):");
        println!("{preview}");
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
            appeal: None,
            salary_min: None,
            salary_max: None,
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
    let mut sql = format!(
        "SELECT {}, c.name
         FROM jobs j INNER JOIN companies c ON j.company_id = c.id
         WHERE j.posting_state = 'active'
           AND j.source IN ('greenhouse', 'lever', 'ashby')",
        job_cols(Some("j"))
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

    if let Some(search) = &args.search {
        sql.push_str(
            " AND (j.title LIKE ? OR c.name LIKE ? OR j.url LIKE ? OR j.notes LIKE ? OR \
             j.description LIKE ? OR j.location LIKE ? OR c.careers_url LIKE ? OR \
             EXISTS (SELECT 1 FROM job_events e WHERE e.job_id = j.id AND e.note LIKE ?))",
        );
        let pattern = format!("%{search}%");
        for _ in 0..8 {
            values.push(Box::new(pattern.clone()));
        }
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
            company_name: row.get(JOB_COL_COUNT)?,
        })
    })?;

    let mut positions = rows.collect::<Result<Vec<_>, _>>()?;

    // Apply the same criteria filter as the desktop watch listing, then apply
    // the caller's limit so truncation happens post-filter (Req 8.3).
    retain_watch_positions_matching_criteria(conn, &mut positions)?;

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
                search: None,
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
            let summary = run_jobs_cycle_trigger(paths, None, Trigger::Cli).await?;
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
    let summary = run_jobs_cycle_trigger(paths, None, Trigger::Cli).await?;
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
    fn delete_confirmation_accepts_yes_and_refuses_noninteractive() {
        assert!(confirms_delete_answer("y"));
        assert!(confirms_delete_answer(" YES "));
        assert!(!confirms_delete_answer("n"));
        assert!(!confirms_delete_answer(""));

        assert!(confirm_permanent_delete(true, 1, "preview", false).is_ok());
        let refused =
            confirm_permanent_delete(false, 2, "  job-1\n    Role — Acme (wishlist)", false)
                .unwrap_err();
        let message = refused.to_string();
        assert!(message.contains("without --yes"));
        assert!(message.contains("jt delete <id> [<id>...] --yes"));
        assert!(message.contains("job-1"));
    }

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
                search: None,
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

    #[test]
    fn list_and_watch_search_cover_location_careers_url_and_job_history() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let company_id = create_id();
        let job_id = create_id();
        let timestamp = now_iso();

        conn.execute(
            "INSERT INTO companies (id, name, careers_url, created_at, updated_at) VALUES (?1, 'Acme', 'https://careers.acme.example/jobs', ?2, ?2)",
            params![company_id, timestamp],
        )
        .unwrap();
        conn.execute(
            r#"INSERT INTO jobs (
                id, company_id, title, url, canonical_url, status, posting_state, source,
                description, location, is_new_from_watch, watch_disposition,
                missing_from_sync_count, is_favorite, created_at, updated_at
            ) VALUES (
                ?1, ?2, 'Platform Engineer', 'https://example.com/jobs/1',
                'https://example.com/jobs/1', 'wishlist', 'active', 'greenhouse',
                'Build distributed systems', 'Remote - North America', 1, 'new', 0, 0, ?3, ?3
            )"#,
            params![job_id, company_id, timestamp],
        )
        .unwrap();
        for event_id in [create_id(), create_id()] {
            conn.execute(
                "INSERT INTO job_events (id, job_id, type, note, occurred_at) VALUES (?1, ?2, 'note_added', 'Recruiter mentioned special historical milestone', ?3)",
                params![event_id, job_id, timestamp],
            )
            .unwrap();
        }

        for search_term in ["North America", "careers.acme", "historical milestone"] {
            let tracked = crate::jobs::service::list_jobs(
                &conn,
                crate::jobs::service::JobFilters {
                    search: Some(search_term.to_string()),
                    new_from_watch: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(tracked.len(), 1, "tracked search term: {search_term}");
            assert_eq!(tracked[0].job.id, job_id);

            let watched = load_watch_positions(
                &conn,
                WatchListArgs {
                    new_only: true,
                    dismissed: false,
                    all: false,
                    provider: Some("greenhouse".to_string()),
                    company: Some("Acme".to_string()),
                    search: Some(search_term.to_string()),
                    limit: Some(1),
                },
            )
            .unwrap();
            assert_eq!(watched.len(), 1, "watch search term: {search_term}");
            assert_eq!(watched[0].job.id, job_id);
        }

        let empty = load_watch_positions(
            &conn,
            WatchListArgs {
                new_only: true,
                dismissed: false,
                all: false,
                provider: None,
                company: None,
                search: Some("no matching posting".to_string()),
                limit: None,
            },
        )
        .unwrap();
        assert!(empty.is_empty());
    }
}
