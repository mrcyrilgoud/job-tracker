pub mod args;
pub mod handlers;
pub mod output;

use rusqlite::Connection;

use crate::cli::args::{Cli, Commands};
use crate::db::migrate;
use crate::db::paths::{resolve_data_dir, DataPaths};
use crate::error::AppResult;

fn open_cli_connection(paths: &DataPaths) -> AppResult<Connection> {
    paths.ensure_dirs()?;
    let conn = Connection::open(&paths.db_path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000i32)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate::migrate(&conn)?;
    Ok(conn)
}

pub async fn run_cli(cli: Cli) -> AppResult<()> {
    let paths = if let Some(dir) = cli.data_dir {
        DataPaths::from_data_dir(dir)
    } else {
        resolve_data_dir(None)
    };

    let json = cli.json;
    let quiet = cli.quiet;

    if cli.run_jobs {
        return handlers::handle_sync(&paths, json, quiet).await;
    }

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            // Default to list if no subcommand is given
            Commands::List(args::ListArgs {
                status: None,
                company: None,
                search: None,
                location: None,
                favorites: false,
                archived: false,
                limit: None,
            })
        }
    };

    match command {
        Commands::List(args) => {
            let conn = open_cli_connection(&paths)?;
            handlers::handle_list(&conn, &paths, args, json)?;
        }
        Commands::Get(args) => {
            let conn = open_cli_connection(&paths)?;
            handlers::handle_get(&conn, args, json)?;
        }
        Commands::Add(args) => {
            let mut conn = open_cli_connection(&paths)?;
            handlers::handle_add(&mut conn, &paths, args, json, quiet).await?;
        }
        Commands::Update(args) => {
            let conn = open_cli_connection(&paths)?;
            handlers::handle_update(&conn, &paths, args, json, quiet)?;
        }
        Commands::Note(args) => {
            let conn = open_cli_connection(&paths)?;
            handlers::handle_note(&conn, &paths, args, json, quiet)?;
        }
        Commands::Stats(_) => {
            let conn = open_cli_connection(&paths)?;
            handlers::handle_stats(&conn, json)?;
        }
        Commands::Watches(args) => {
            let mut conn = open_cli_connection(&paths)?;
            handlers::handle_watches(&mut conn, &paths, args.command, json, quiet).await?;
        }
        Commands::Sync(_) => {
            handlers::handle_sync(&paths, json, quiet).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use rusqlite::Connection;
    use crate::db::migrate;
    use crate::db::paths::DataPaths;
    use super::args::*;
    use super::handlers;

    #[test]
    fn parses_list_args() {
        let cli = Cli::try_parse_from(["job-tracker", "list", "--status", "applied", "-f"]).unwrap();
        match cli.command {
            Some(Commands::List(args)) => {
                assert_eq!(args.status.as_deref(), Some("applied"));
                assert!(args.favorites);
            }
            _ => panic!("Expected List command"),
        }
    }

    #[test]
    fn parses_add_args() {
        let cli = Cli::try_parse_from([
            "job-tracker",
            "add",
            "https://example.com/job/123",
            "--title",
            "Staff Engineer",
            "--company",
            "Stripe",
            "--notes",
            "Referred by Alex",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Add(args)) => {
                assert_eq!(args.url, "https://example.com/job/123");
                assert_eq!(args.title.as_deref(), Some("Staff Engineer"));
                assert_eq!(args.company.as_deref(), Some("Stripe"));
                assert_eq!(args.notes.as_deref(), Some("Referred by Alex"));
            }
            _ => panic!("Expected Add command"),
        }
    }

    #[test]
    fn parses_update_args() {
        let cli = Cli::try_parse_from([
            "job-tracker",
            "update",
            "job_123",
            "--status",
            "interviewing",
            "-f",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Update(args)) => {
                assert_eq!(args.target, "job_123");
                assert_eq!(args.status.as_deref(), Some("interviewing"));
                assert!(args.favorite);
                assert!(!args.unfavorite);
            }
            _ => panic!("Expected Update command"),
        }
    }

    #[tokio::test]
    async fn test_cli_crud_workflow() {
        let temp_dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_data_dir(temp_dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let mut conn = Connection::open(&paths.db_path).unwrap();
        migrate::migrate(&conn).unwrap();

        // 1. Add job
        let add_args = AddArgs {
            url: "https://example.com/jobs/staff-eng".to_string(),
            title: Some("Staff Engineer".to_string()),
            company: Some("Acme Corp".to_string()),
            status: "wishlist".to_string(),
            applied_at: None,
            notes: Some("Initial note".to_string()),
            location: Some("Remote".to_string()),
            favorite: true,
        };
        handlers::handle_add(&mut conn, &paths, add_args, true, true).await.unwrap();

        // 2. Resolve job ID by prefix / URL
        let job_id = handlers::resolve_target_job_id(&conn, "https://example.com/jobs/staff-eng").unwrap();
        assert!(!job_id.is_empty());

        let prefix = &job_id[..6];
        let resolved_by_prefix = handlers::resolve_target_job_id(&conn, prefix).unwrap();
        assert_eq!(resolved_by_prefix, job_id);

        // 3. Update job
        let update_args = UpdateArgs {
            target: job_id.clone(),
            status: Some("interviewing".to_string()),
            applied_at: Some("today".to_string()),
            notes: None,
            append_note: Some("Passed initial screen".to_string()),
            title: None,
            company: None,
            location: None,
            favorite: false,
            unfavorite: false,
            archive: false,
            unarchive: false,
        };
        handlers::handle_update(&conn, &paths, update_args, true, true).unwrap();

        // 4. Add Note
        let note_args = NoteArgs {
            target: job_id.clone(),
            note: "Technical interview on Friday".to_string(),
        };
        handlers::handle_note(&conn, &paths, note_args, true, true).unwrap();

        // 5. Get detail & verify notes + events
        let detail = crate::jobs::service::get_job_detail(&conn, &job_id).unwrap().unwrap();
        assert_eq!(detail.job.status, "interviewing");
        assert!(detail.job.is_favorite);
        assert!(detail.job.notes.as_ref().unwrap().contains("Passed initial screen"));
        assert!(detail.job.notes.as_ref().unwrap().contains("Technical interview on Friday"));
        assert_eq!(detail.events.len(), 4); // created, favorited, status_changed, note_added
        assert!(detail.events.iter().any(|e| e.event_type == "note_added"));

        // 6. List jobs
        let list_args = ListArgs {
            status: Some("interviewing".to_string()),
            company: None,
            search: None,
            location: None,
            favorites: false,
            archived: false,
            limit: None,
        };
        handlers::handle_list(&conn, &paths, list_args, true).unwrap();

        // 7. Stats
        handlers::handle_stats(&conn, true).unwrap();
    }
}
