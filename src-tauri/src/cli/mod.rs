pub mod args;
pub mod handlers;
pub mod output;

use rusqlite::Connection;

use crate::cli::args::{Cli, Commands};
use crate::db::migrate;
use crate::db::paths::{resolve_data_dir, DataPaths};
use crate::error::AppResult;

/// Returns the nearest ancestor (inclusive) of `dir` containing a `.git` entry.
pub(crate) fn enclosing_git_repo(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let start = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    start
        .ancestors()
        .find(|p| p.join(".git").exists())
        .map(|p| p.to_path_buf())
}

fn canon(p: &std::path::Path) -> std::path::PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Returns the git repo that makes `data_dir` "dev-only" data, if any.
///
/// A git root that is the user's home directory (or an ancestor of it, e.g. `/`)
/// is never treated as a dev repo: people accidentally `git init` their home
/// folder. The standard app data dir (~/Library/Application Support/
/// com.jobtracker.local) is always allowed.
pub(crate) fn dev_data_repo(
    data_dir: &std::path::Path,
    home: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let data = canon(data_dir);
    if let Some(home) = home {
        let home = canon(home);
        if data == canon(&home.join("Library/Application Support/com.jobtracker.local")) {
            return None;
        }
        let repo = enclosing_git_repo(&data)?;
        if home.starts_with(&repo) {
            return None;
        }
        return Some(repo);
    }
    enclosing_git_repo(&data).filter(|r| r.parent().is_some())
}

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
        if !cfg!(debug_assertions) && !cli.allow_dev_data {
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
            if let Some(repo) = dev_data_repo(&paths.data_dir, home.as_deref()) {
                return Err(crate::error::AppError::Message(format!(
                    "refusing --run-jobs: data dir {} is inside git repo {} (dev-only data). \
                     Use ~/Library/Application Support/com.jobtracker.local, or pass --allow-dev-data.",
                    paths.data_dir.display(),
                    repo.display()
                )));
            }
        }
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
        Commands::Description(args) => {
            let conn = open_cli_connection(&paths)?;
            handlers::handle_description(&conn, &paths, args, json, quiet)?;
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
    use super::args::*;
    use super::handlers;
    use crate::db::migrate;
    use crate::db::paths::DataPaths;
    use clap::Parser;
    use rusqlite::Connection;

    #[test]
    fn parses_list_args() {
        let cli =
            Cli::try_parse_from(["job-tracker", "list", "--status", "applied", "-f"]).unwrap();
        match cli.command {
            Some(Commands::List(args)) => {
                assert_eq!(args.status.as_deref(), Some("applied"));
                assert!(args.favorites);
            }
            _ => panic!("Expected List command"),
        }
    }

    #[test]
    fn parses_watch_list_search_arg() {
        let cli = Cli::try_parse_from([
            "job-tracker",
            "watches",
            "list",
            "--search",
            "distributed systems",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Watches(WatchesArgs {
                command: Some(WatchCommands::List(args)),
            })) => assert_eq!(args.search.as_deref(), Some("distributed systems")),
            _ => panic!("Expected watches list command"),
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
            "--description",
            "Design distributed ledgers",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Add(args)) => {
                assert_eq!(args.url, "https://example.com/job/123");
                assert_eq!(args.title.as_deref(), Some("Staff Engineer"));
                assert_eq!(args.company.as_deref(), Some("Stripe"));
                assert_eq!(args.notes.as_deref(), Some("Referred by Alex"));
                assert_eq!(
                    args.description.as_deref(),
                    Some("Design distributed ledgers")
                );
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
            "--description",
            "Updated role scope",
            "-f",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Update(args)) => {
                assert_eq!(args.target, "job_123");
                assert_eq!(args.status.as_deref(), Some("interviewing"));
                assert_eq!(args.description.as_deref(), Some("Updated role scope"));
                assert!(args.favorite);
                assert!(!args.unfavorite);
                assert_eq!(args.appeal, None);
                assert!(!args.clear_appeal);
            }
            _ => panic!("Expected Update command"),
        }
    }

    #[test]
    fn parses_appeal_bounds() {
        let cli =
            Cli::try_parse_from(["job-tracker", "update", "job_123", "--appeal", "5"]).unwrap();
        match cli.command {
            Some(Commands::Update(args)) => {
                assert_eq!(args.appeal, Some(5));
                assert!(!args.clear_appeal);
            }
            _ => panic!("Expected Update command"),
        }

        let cleared =
            Cli::try_parse_from(["job-tracker", "update", "job_123", "--clear-appeal"]).unwrap();
        match cleared.command {
            Some(Commands::Update(args)) => {
                assert_eq!(args.appeal, None);
                assert!(args.clear_appeal);
            }
            _ => panic!("Expected Update command"),
        }

        assert!(
            Cli::try_parse_from(["job-tracker", "update", "job_123", "--appeal", "0"]).is_err()
        );
        assert!(
            Cli::try_parse_from(["job-tracker", "update", "job_123", "--appeal", "6"]).is_err()
        );
        assert!(Cli::try_parse_from([
            "job-tracker",
            "update",
            "job_123",
            "--appeal",
            "5",
            "--clear-appeal"
        ])
        .is_err());
    }

    #[test]
    fn parses_description_args() {
        let cli =
            Cli::try_parse_from(["job-tracker", "desc", "job_123", "Direct description text"])
                .unwrap();
        match cli.command {
            Some(Commands::Description(args)) => {
                assert_eq!(args.target, "job_123");
                assert_eq!(args.description.as_deref(), Some("Direct description text"));
                assert!(!args.show);
                assert!(!args.clear);
            }
            _ => panic!("Expected Description command"),
        }

        let cli_show =
            Cli::try_parse_from(["job-tracker", "description", "job_123", "--show"]).unwrap();
        match cli_show.command {
            Some(Commands::Description(args)) => {
                assert_eq!(args.target, "job_123");
                assert!(args.show);
            }
            _ => panic!("Expected Description command"),
        }
    }

    #[tokio::test]
    async fn test_cli_crud_workflow() {
        let temp_dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_data_dir(temp_dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let mut conn = Connection::open(&paths.db_path).unwrap();
        migrate::migrate(&conn).unwrap();

        // 1. Add job with description and notes
        let add_args = AddArgs {
            url: "https://example.com/jobs/staff-eng".to_string(),
            title: Some("Staff Engineer".to_string()),
            company: Some("Acme Corp".to_string()),
            status: "wishlist".to_string(),
            applied_at: None,
            notes: Some("Initial note".to_string()),
            description: Some("Original job requirements and responsibilities".to_string()),
            location: Some("Remote".to_string()),
            favorite: true,
        };
        handlers::handle_add(&mut conn, &paths, add_args, true, true)
            .await
            .unwrap();

        // 2. Resolve job ID by prefix / URL
        let job_id =
            handlers::resolve_target_job_id(&conn, "https://example.com/jobs/staff-eng").unwrap();
        assert!(!job_id.is_empty());

        let prefix = &job_id[..6];
        let resolved_by_prefix = handlers::resolve_target_job_id(&conn, prefix).unwrap();
        assert_eq!(resolved_by_prefix, job_id);

        // 3. Update job (including description)
        let update_args = UpdateArgs {
            target: job_id.clone(),
            status: Some("interviewing".to_string()),
            applied_at: Some("today".to_string()),
            notes: None,
            append_note: Some("Passed initial screen".to_string()),
            description: Some("Updated responsibilities".to_string()),
            clear_description: false,
            title: None,
            company: None,
            location: None,
            favorite: false,
            unfavorite: false,
            archive: false,
            unarchive: false,
            appeal: None,
            clear_appeal: false,
        };
        handlers::handle_update(&conn, &paths, update_args, true, true).unwrap();

        // 4. Add Note
        let note_args = NoteArgs {
            target: job_id.clone(),
            note: "Technical interview on Friday".to_string(),
        };
        handlers::handle_note(&conn, &paths, note_args, true, true).unwrap();

        // 5. Test CLI description subcommand directly
        let desc_args = DescriptionArgs {
            target: job_id.clone(),
            description: Some("Final edited job description via CLI".to_string()),
            file: None,
            stdin: false,
            show: false,
            clear: false,
        };
        handlers::handle_description(&conn, &paths, desc_args, true, true).unwrap();

        // 6. Get detail & verify description + notes + events
        let detail = crate::jobs::service::get_job_detail(&conn, &job_id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.job.status, "interviewing");
        assert!(detail.job.is_favorite);
        assert_eq!(
            detail.job.description.as_deref(),
            Some("Final edited job description via CLI")
        );
        assert!(detail
            .job
            .notes
            .as_ref()
            .unwrap()
            .contains("Passed initial screen"));
        assert!(detail
            .job
            .notes
            .as_ref()
            .unwrap()
            .contains("Technical interview on Friday"));
        assert_eq!(detail.events.len(), 4); // created, favorited, status_changed, note_added
        assert!(detail.events.iter().any(|e| e.event_type == "note_added"));

        // 7. Clear description via CLI
        let clear_desc_args = DescriptionArgs {
            target: job_id.clone(),
            description: None,
            file: None,
            stdin: false,
            show: false,
            clear: true,
        };
        handlers::handle_description(&conn, &paths, clear_desc_args, true, true).unwrap();
        let cleared_detail = crate::jobs::service::get_job_detail(&conn, &job_id)
            .unwrap()
            .unwrap();
        assert_eq!(cleared_detail.job.description, None);

        // 8. List jobs with search across description
        let set_desc_again = DescriptionArgs {
            target: job_id.clone(),
            description: Some("Quantum cryptography experience required".to_string()),
            file: None,
            stdin: false,
            show: false,
            clear: false,
        };
        handlers::handle_description(&conn, &paths, set_desc_again, true, true).unwrap();

        let list_args = ListArgs {
            status: Some("interviewing".to_string()),
            company: None,
            search: Some("Quantum".to_string()),
            location: None,
            favorites: false,
            archived: false,
            limit: None,
        };
        let search_results = crate::jobs::service::list_jobs(
            &conn,
            crate::jobs::service::JobFilters {
                status: list_args.status.clone(),
                company_id: list_args.company.clone(),
                posting_state: None,
                search: list_args.search.clone(),
                salary_min: None,
                salary_max: None,
                location: list_args.location.clone(),
                new_from_watch: None,
                is_favorite: None,
                is_archived: Some(false),
                limit: None,
            },
        )
        .unwrap();
        assert_eq!(search_results.len(), 1);
        assert_eq!(search_results[0].job.id, job_id);

        // 9. Stats
        handlers::handle_stats(&conn, true).unwrap();

        // 10. Set, export, and clear overall appeal without touching favorite.
        let set_appeal = UpdateArgs {
            target: job_id.clone(),
            status: None,
            applied_at: None,
            notes: None,
            append_note: None,
            description: None,
            clear_description: false,
            title: None,
            company: None,
            location: None,
            favorite: false,
            unfavorite: false,
            archive: false,
            unarchive: false,
            appeal: Some(4),
            clear_appeal: false,
        };
        handlers::handle_update(&conn, &paths, set_appeal, true, true).unwrap();
        let scored = crate::jobs::service::get_job_detail(&conn, &job_id)
            .unwrap()
            .unwrap();
        assert_eq!(scored.job.appeal, Some(4));
        assert!(scored.job.is_favorite);
        let csv = std::fs::read_to_string(&paths.jobs_csv_path).unwrap();
        let parsed = crate::jobs::csv::parse_csv(&csv);
        let appeal_idx = parsed[0]
            .iter()
            .position(|header| header == "appeal")
            .unwrap();
        assert_eq!(parsed[0].last().map(String::as_str), Some("appeal"));
        assert_eq!(parsed[1][appeal_idx], "4");

        let clear_appeal = UpdateArgs {
            target: job_id.clone(),
            status: None,
            applied_at: None,
            notes: None,
            append_note: None,
            description: None,
            clear_description: false,
            title: None,
            company: None,
            location: None,
            favorite: false,
            unfavorite: false,
            archive: false,
            unarchive: false,
            appeal: None,
            clear_appeal: true,
        };
        handlers::handle_update(&conn, &paths, clear_appeal, true, true).unwrap();
        let cleared = crate::jobs::service::get_job_detail(&conn, &job_id)
            .unwrap()
            .unwrap();
        assert_eq!(cleared.job.appeal, None);
        assert!(cleared.job.is_favorite);
        let cleared_csv = std::fs::read_to_string(&paths.jobs_csv_path).unwrap();
        let cleared_rows = crate::jobs::csv::parse_csv(&cleared_csv);
        assert_eq!(cleared_rows[1][appeal_idx], "");
    }
}

#[cfg(test)]
mod dev_data_guard_tests {
    use super::{dev_data_repo, enclosing_git_repo};
    use crate::cli::args::Cli;
    use clap::Parser;

    #[test]
    fn home_dir_git_root_is_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(home.join(".git")).unwrap();
        let other = home.join("some/data");
        std::fs::create_dir_all(&other).unwrap();
        assert!(dev_data_repo(&other, Some(&home)).is_none());
        // git root above home is also ignored
        let home2 = tmp.path().join("home/nested-user");
        std::fs::create_dir_all(home2.join("x")).unwrap();
        assert!(dev_data_repo(&home2.join("x"), Some(&home2)).is_none());
    }

    #[test]
    fn application_support_is_allowed_even_under_home_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(home.join(".git")).unwrap();
        let app = home.join("Library/Application Support/com.jobtracker.local");
        std::fs::create_dir_all(&app).unwrap();
        assert!(dev_data_repo(&app, Some(&home)).is_none());
    }

    #[test]
    fn project_repo_data_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(home.join(".git")).unwrap();
        let repo = home.join("repos/Job-Tracker");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("data")).unwrap();
        let found = dev_data_repo(&repo.join("data"), Some(&home)).unwrap();
        assert_eq!(found, repo.canonicalize().unwrap());
    }

    #[test]
    fn allow_dev_data_flag_bypasses() {
        let cli = Cli::try_parse_from(["job-tracker", "--run-jobs", "--allow-dev-data"]).unwrap();
        assert!(cli.allow_dev_data);
    }

    #[test]
    fn detects_data_dir_inside_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("data")).unwrap();
        let found = enclosing_git_repo(&repo.join("data")).unwrap();
        assert_eq!(found, repo.canonicalize().unwrap());
    }

    #[test]
    fn ignores_data_dir_outside_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Application Support/com.jobtracker.local");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(enclosing_git_repo(&dir).is_none());
    }
}
