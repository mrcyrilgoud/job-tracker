pub mod args;
pub mod handlers;
pub mod output;

use std::io::IsTerminal;

use rusqlite::Connection;

use crate::cli::args::{Cli, Commands};
use crate::db::configure_app_connection;
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
    configure_app_connection(&conn)?;
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
                status: Vec::new(),
                company: None,
                search: None,
                location: None,
                date_field: None,
                date_from: None,
                date_to: None,
                remote: None,
                favorites: false,
                archived: false,
                limit: None,
                min_company_stability: None,
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
        Commands::Delete(args) => {
            let conn = open_cli_connection(&paths)?;
            let stdin_is_terminal = std::io::stdin().is_terminal();
            handlers::handle_delete(&conn, &paths, args, json, quiet, stdin_is_terminal)?;
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
                assert_eq!(args.status, vec!["applied".to_string()]);
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
                assert_eq!(args.application_ease, None);
                assert!(!args.clear_application_ease);
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
    fn parses_application_ease_bounds() {
        let cli = Cli::try_parse_from([
            "job-tracker",
            "update",
            "job_123",
            "--application-ease",
            "5",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Update(args)) => {
                assert_eq!(args.application_ease, Some(5));
                assert!(!args.clear_application_ease);
                assert_eq!(args.appeal, None);
            }
            _ => panic!("Expected Update command"),
        }

        let cleared = Cli::try_parse_from([
            "job-tracker",
            "update",
            "job_123",
            "--clear-application-ease",
        ])
        .unwrap();
        match cleared.command {
            Some(Commands::Update(args)) => {
                assert_eq!(args.application_ease, None);
                assert!(args.clear_application_ease);
            }
            _ => panic!("Expected Update command"),
        }

        assert!(Cli::try_parse_from([
            "job-tracker",
            "update",
            "job_123",
            "--application-ease",
            "0"
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "job-tracker",
            "update",
            "job_123",
            "--application-ease",
            "6"
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "job-tracker",
            "update",
            "job_123",
            "--application-ease",
            "1",
            "--clear-application-ease"
        ])
        .is_err());
    }

    #[test]
    fn parses_delete_args() {
        let cli = Cli::try_parse_from([
            "job-tracker",
            "delete",
            "job_abc",
            "job_def",
            "--yes",
            "--json",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Delete(args)) => {
                assert_eq!(
                    args.targets,
                    vec!["job_abc".to_string(), "job_def".to_string()]
                );
                assert!(args.yes);
                assert!(!args.dry_run);
            }
            _ => panic!("Expected Delete command"),
        }
        assert!(cli.json);

        let dry_run =
            Cli::try_parse_from(["job-tracker", "delete", "job_abc", "--dry-run"]).unwrap();
        match dry_run.command {
            Some(Commands::Delete(args)) => {
                assert!(!args.yes);
                assert!(args.dry_run);
            }
            _ => panic!("Expected Delete command"),
        }

        assert!(Cli::try_parse_from(["job-tracker", "delete"]).is_err());
    }

    #[test]
    fn delete_help_includes_examples() {
        use clap::CommandFactory;
        let mut cmd = Cli::command();
        let help = cmd
            .find_subcommand_mut("delete")
            .expect("delete subcommand")
            .render_long_help()
            .to_string();
        assert!(help.contains("jt delete <job_id> --yes"));
        assert!(help.contains("--dry-run"));
        assert!(help.contains("closed"));
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
            application_ease: None,
            clear_application_ease: false,
            company_stability: None,
            clear_company_stability: false,
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
            status: vec!["interviewing".to_string()],
            company: None,
            search: Some("Quantum".to_string()),
            location: None,
            date_field: None,
            date_from: None,
            date_to: None,
            remote: None,
            favorites: false,
            archived: false,
            limit: None,
            min_company_stability: None,
        };
        let search_results = crate::jobs::service::list_jobs(
            &conn,
            crate::jobs::service::JobFilters {
                statuses: Some(list_args.status.clone()),
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
                ..Default::default()
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
            application_ease: None,
            clear_application_ease: false,
            company_stability: None,
            clear_company_stability: false,
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
        assert_eq!(
            parsed[0].last().map(String::as_str),
            Some("company_stability")
        );
        assert_eq!(parsed[1][appeal_idx], "4");
        let ease_idx = parsed[0]
            .iter()
            .position(|header| header == "application_ease")
            .unwrap();
        assert_eq!(parsed[1][ease_idx], "");

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
            application_ease: None,
            clear_application_ease: false,
            company_stability: None,
            clear_company_stability: false,
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

        // 11. Set and clear application ease without touching favorite or appeal.
        let set_ease = UpdateArgs {
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
            clear_appeal: false,
            application_ease: Some(2),
            clear_application_ease: false,
            company_stability: None,
            clear_company_stability: false,
        };
        handlers::handle_update(&conn, &paths, set_ease, true, true).unwrap();
        let eased = crate::jobs::service::get_job_detail(&conn, &job_id)
            .unwrap()
            .unwrap();
        assert_eq!(eased.job.application_ease, Some(2));
        assert_eq!(eased.job.appeal, None);
        assert!(eased.job.is_favorite);
        let eased_json = serde_json::to_value(&eased).unwrap();
        assert_eq!(eased_json["job"]["applicationEase"], 2);
        let listed = crate::jobs::service::list_jobs(
            &conn,
            crate::jobs::service::JobFilters {
                status: Some("interviewing".to_string()),
                ..crate::jobs::service::JobFilters::default()
            },
        )
        .unwrap();
        let listed_json = serde_json::to_value(&listed).unwrap();
        assert_eq!(listed_json[0]["job"]["applicationEase"], 2);
        let eased_csv = std::fs::read_to_string(&paths.jobs_csv_path).unwrap();
        let eased_rows = crate::jobs::csv::parse_csv(&eased_csv);
        assert_eq!(eased_rows[1][ease_idx], "2");

        let clear_ease = UpdateArgs {
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
            clear_appeal: false,
            application_ease: None,
            clear_application_ease: true,
            company_stability: None,
            clear_company_stability: false,
        };
        handlers::handle_update(&conn, &paths, clear_ease, true, true).unwrap();
        let ease_cleared = crate::jobs::service::get_job_detail(&conn, &job_id)
            .unwrap()
            .unwrap();
        assert_eq!(ease_cleared.job.application_ease, None);
        assert!(ease_cleared.job.is_favorite);
        let ease_cleared_rows =
            crate::jobs::csv::parse_csv(&std::fs::read_to_string(&paths.jobs_csv_path).unwrap());
        assert_eq!(ease_cleared_rows[1][ease_idx], "");
    }

    fn insert_job(conn: &Connection, url: &str, title: &str) -> String {
        crate::jobs::service::create_job_from_url(
            conn,
            url,
            title,
            Some("Acme"),
            Some("wishlist"),
            None,
            None,
            None,
        )
        .unwrap()
        .0
        .id
    }

    fn rename_job_id(conn: &Connection, from: &str, to: &str) {
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        conn.execute(
            "UPDATE job_events SET job_id = ?1 WHERE job_id = ?2",
            rusqlite::params![to, from],
        )
        .unwrap();
        conn.execute(
            "UPDATE jobs SET id = ?1 WHERE id = ?2",
            rusqlite::params![to, from],
        )
        .unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
    }

    fn csv_ids(csv: &str) -> Vec<String> {
        crate::jobs::csv::parse_csv(csv)
            .into_iter()
            .skip(1)
            .filter_map(|row| row.first().cloned())
            .collect()
    }

    fn delete_args(targets: &[&str], yes: bool, dry_run: bool) -> DeleteArgs {
        DeleteArgs {
            targets: targets.iter().map(|target| (*target).to_string()).collect(),
            yes,
            dry_run,
        }
    }

    #[test]
    fn delete_command_removes_jobs_from_db_and_csv() {
        let temp_dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_data_dir(temp_dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let conn = Connection::open(&paths.db_path).unwrap();
        migrate::migrate(&conn).unwrap();

        let keep_id = insert_job(&conn, "https://example.com/jobs/keep", "Keep role");
        let prefix_id = insert_job(&conn, "https://example.com/jobs/prefix", "Prefix role");
        rename_job_id(&conn, &prefix_id, "pref2222-1111-1111-1111-111111111111");
        let prefix_id = "pref2222-1111-1111-1111-111111111111".to_string();
        let url_id = insert_job(&conn, "https://example.com/jobs/delete-by-url", "URL role");
        let first_shared = insert_job(&conn, "https://example.com/jobs/shared-a", "Shared A");
        let second_shared = insert_job(&conn, "https://example.com/jobs/shared-b", "Shared B");
        rename_job_id(&conn, &first_shared, "abc11111-1111-1111-1111-111111111111");
        rename_job_id(
            &conn,
            &second_shared,
            "abc11111-2222-2222-2222-222222222222",
        );
        let first_shared = "abc11111-1111-1111-1111-111111111111".to_string();
        let second_shared = "abc11111-2222-2222-2222-222222222222".to_string();

        crate::jobs::csv::export_jobs_csv(&conn, &paths.jobs_csv_path, None).unwrap();
        let before = csv_ids(&std::fs::read_to_string(&paths.jobs_csv_path).unwrap());
        assert!(before.contains(&prefix_id));
        assert!(before.contains(&keep_id));

        let missing = handlers::handle_delete(
            &conn,
            &paths,
            delete_args(&["missing-job"], true, false),
            true,
            true,
            false,
        );
        assert!(missing.unwrap_err().to_string().contains("No job found"));
        assert!(crate::jobs::service::get_job_by_id(&conn, &keep_id)
            .unwrap()
            .is_some());

        let ambiguous = handlers::handle_delete(
            &conn,
            &paths,
            delete_args(&["abc11111"], true, false),
            true,
            true,
            false,
        );
        assert!(ambiguous.unwrap_err().to_string().contains("Ambiguous"));
        assert!(crate::jobs::service::get_job_by_id(&conn, &first_shared)
            .unwrap()
            .is_some());
        assert!(crate::jobs::service::get_job_by_id(&conn, &second_shared)
            .unwrap()
            .is_some());

        let needs_yes = handlers::handle_delete(
            &conn,
            &paths,
            delete_args(&[&prefix_id], false, false),
            true,
            true,
            true,
        );
        assert!(needs_yes.unwrap_err().to_string().contains("without --yes"));
        assert!(crate::jobs::service::get_job_by_id(&conn, &prefix_id)
            .unwrap()
            .is_some());

        handlers::handle_delete(
            &conn,
            &paths,
            delete_args(&[&prefix_id], false, true),
            true,
            true,
            false,
        )
        .unwrap();
        assert!(crate::jobs::service::get_job_by_id(&conn, &prefix_id)
            .unwrap()
            .is_some());

        handlers::handle_delete(
            &conn,
            &paths,
            delete_args(&["pref2222", "pref2222"], true, false),
            true,
            true,
            false,
        )
        .unwrap();
        assert!(crate::jobs::service::get_job_by_id(&conn, &prefix_id)
            .unwrap()
            .is_none());
        assert!(crate::jobs::service::get_job_by_id(&conn, &keep_id)
            .unwrap()
            .is_some());

        handlers::handle_delete(
            &conn,
            &paths,
            delete_args(&["https://example.com/jobs/delete-by-url"], true, false),
            true,
            true,
            false,
        )
        .unwrap();
        assert!(crate::jobs::service::get_job_by_id(&conn, &url_id)
            .unwrap()
            .is_none());

        let partial = handlers::handle_delete(
            &conn,
            &paths,
            delete_args(&[&keep_id, "still-missing"], true, false),
            true,
            true,
            false,
        );
        assert!(partial.is_err());
        assert!(crate::jobs::service::get_job_by_id(&conn, &keep_id)
            .unwrap()
            .is_some());

        handlers::handle_delete(
            &conn,
            &paths,
            delete_args(&[&first_shared, &second_shared], true, false),
            true,
            true,
            false,
        )
        .unwrap();
        assert!(crate::jobs::service::get_job_by_id(&conn, &first_shared)
            .unwrap()
            .is_none());
        assert!(crate::jobs::service::get_job_by_id(&conn, &second_shared)
            .unwrap()
            .is_none());

        let csv = std::fs::read_to_string(&paths.jobs_csv_path).unwrap();
        let remaining = csv_ids(&csv);
        assert_eq!(remaining, vec![keep_id.clone()]);
        assert!(!csv.contains(&prefix_id));
        assert!(!csv.contains(&url_id));
        assert!(!csv.contains(&first_shared));

        let (_imported, exported) =
            crate::jobs::csv::sync_jobs_csv_with_disk(&paths.db_path, &paths.jobs_csv_path)
                .unwrap();
        assert_eq!(exported.row_count, 1);
        let after_sync = csv_ids(&std::fs::read_to_string(&paths.jobs_csv_path).unwrap());
        assert_eq!(after_sync, vec![keep_id.clone()]);
        assert!(crate::jobs::service::get_job_by_id(&conn, &prefix_id)
            .unwrap()
            .is_none());
        assert!(crate::jobs::service::get_job_by_id(&conn, &keep_id)
            .unwrap()
            .is_some());

        crate::jobs::service::delete_job(&conn, &keep_id).unwrap();
        assert!(
            csv_ids(&std::fs::read_to_string(&paths.jobs_csv_path).unwrap()).contains(&keep_id)
        );
        let (_imported, exported) =
            crate::jobs::csv::sync_jobs_csv_with_disk(&paths.db_path, &paths.jobs_csv_path)
                .unwrap();
        assert_eq!(exported.row_count, 0);
        assert!(csv_ids(&std::fs::read_to_string(&paths.jobs_csv_path).unwrap()).is_empty());
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn closed_status_keeps_the_job() {
        let temp_dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_data_dir(temp_dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let conn = Connection::open(&paths.db_path).unwrap();
        migrate::migrate(&conn).unwrap();
        let job_id = insert_job(&conn, "https://example.com/jobs/close-me", "Close role");

        handlers::handle_update(
            &conn,
            &paths,
            UpdateArgs {
                target: job_id.clone(),
                status: Some("closed".to_string()),
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
                clear_appeal: false,
                application_ease: None,
                clear_application_ease: false,
                company_stability: None,
                clear_company_stability: false,
            },
            true,
            true,
        )
        .unwrap();

        let job = crate::jobs::service::get_job_by_id(&conn, &job_id)
            .unwrap()
            .unwrap();
        assert_eq!(job.status, "closed");
        let rows =
            crate::jobs::csv::parse_csv(&std::fs::read_to_string(&paths.jobs_csv_path).unwrap());
        let status_idx = rows[0]
            .iter()
            .position(|header| header == "status")
            .unwrap();
        let row = rows
            .iter()
            .find(|row| row.first().map(String::as_str) == Some(job_id.as_str()))
            .unwrap();
        assert_eq!(row[status_idx], "closed");
    }

    /// Parse `jt <argv...>` and return the subcommand, so tests build args the
    /// same way the binary does.
    fn parse_command(argv: &[&str]) -> Commands {
        let mut full = vec!["job-tracker"];
        full.extend_from_slice(argv);
        Cli::try_parse_from(full).unwrap().command.unwrap()
    }

    fn update_args(argv: &[&str]) -> UpdateArgs {
        let mut full = vec!["update"];
        full.extend_from_slice(argv);
        match parse_command(&full) {
            Commands::Update(args) => args,
            _ => panic!("Expected Update command"),
        }
    }

    fn list_args(argv: &[&str]) -> ListArgs {
        let mut full = vec!["list"];
        full.extend_from_slice(argv);
        match parse_command(&full) {
            Commands::List(args) => args,
            _ => panic!("Expected List command"),
        }
    }

    fn csv_cell(paths: &DataPaths, job_id: &str, column: &str) -> String {
        let rows =
            crate::jobs::csv::parse_csv(&std::fs::read_to_string(&paths.jobs_csv_path).unwrap());
        let idx = rows[0].iter().position(|h| h == column).unwrap();
        rows.iter()
            .find(|row| row.first().map(String::as_str) == Some(job_id))
            .unwrap()[idx]
            .clone()
    }

    #[test]
    fn company_stability_flag_errors_and_help() {
        use clap::CommandFactory;
        let message = crate::models::COMPANY_STABILITY_RANGE_MESSAGE;
        for argv in [
            ["update", "job_1", "--company-stability", "0"],
            ["update", "job_1", "--company-stability", "3.5"],
            ["list", "--min-company-stability", "6", "--json"],
            ["list", "--min-company-stability", "x", "--json"],
        ] {
            let mut full = vec!["job-tracker"];
            full.extend_from_slice(&argv);
            let error = Cli::try_parse_from(full).unwrap_err().to_string();
            assert!(error.contains(message), "{argv:?}: {error}");
        }
        assert!(Cli::try_parse_from(["job-tracker", "list", "--min-company-stability"]).is_err());

        // Req 3.5: both flags together is a conflict naming both.
        let conflict = Cli::try_parse_from([
            "job-tracker",
            "update",
            "job_1",
            "--company-stability",
            "3",
            "--clear-company-stability",
        ])
        .unwrap_err();
        assert_eq!(conflict.kind(), clap::error::ErrorKind::ArgumentConflict);
        let conflict = conflict.to_string();
        assert!(conflict.contains("--company-stability"));
        assert!(conflict.contains("--clear-company-stability"));

        let mut cmd = Cli::command();
        let update_help = cmd
            .find_subcommand_mut("update")
            .unwrap()
            .render_long_help()
            .to_string();
        for needle in [
            "--company-stability",
            "--clear-company-stability",
            "1 (least stable) to 5 (most stable)",
            "leave the job unscored",
        ] {
            assert!(update_help.contains(needle), "update help missing {needle:?}");
        }
        let list_help = cmd
            .find_subcommand_mut("list")
            .unwrap()
            .render_long_help()
            .to_string();
        for needle in [
            "--min-company-stability",
            ">= N",
            "1-5",
            "5 = most stable",
            "unscored jobs are excluded",
        ] {
            assert!(list_help.contains(needle), "list help missing {needle:?}");
        }
    }

    #[test]
    fn company_stability_update_set_keep_clear_and_csv_mirror() {
        let temp_dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_data_dir(temp_dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let conn = Connection::open(&paths.db_path).unwrap();
        migrate::migrate(&conn).unwrap();
        let job_id = insert_job(&conn, "https://example.com/jobs/stable", "Stable role");
        let other_id = insert_job(&conn, "https://example.com/jobs/other", "Other role");
        handlers::handle_update(
            &conn,
            &paths,
            update_args(&[&job_id, "--appeal", "2", "--application-ease", "5"]),
            true,
            true,
        )
        .unwrap();
        let get = |id: &str| {
            crate::jobs::service::get_job_by_id(&conn, id)
                .unwrap()
                .unwrap()
        };

        // Set (Req 3.1, 3.12): only Company_Stability changes; CSV shows it.
        handlers::handle_update(
            &conn,
            &paths,
            update_args(&[&job_id, "--company-stability", "4"]),
            true,
            true,
        )
        .unwrap();
        let job = get(&job_id);
        assert_eq!(job.company_stability, Some(4));
        assert_eq!((job.appeal, job.application_ease), (Some(2), Some(5)));
        assert_eq!(job.status, "wishlist");
        assert!(!job.is_favorite);
        assert_eq!(csv_cell(&paths, &job_id, "company_stability"), "4");

        // Keep (Req 3.3): an update without either flag leaves the score.
        handlers::handle_update(
            &conn,
            &paths,
            update_args(&[&job_id, "--status", "applied"]),
            true,
            true,
        )
        .unwrap();
        assert_eq!(get(&job_id).company_stability, Some(4));
        assert_eq!(csv_cell(&paths, &job_id, "company_stability"), "4");

        // Missing id (Req 3.6): fails, nothing changes.
        let missing = handlers::handle_update(
            &conn,
            &paths,
            update_args(&["missing-job", "--company-stability", "1"]),
            true,
            true,
        );
        assert!(missing.unwrap_err().to_string().contains("No job found"));
        assert_eq!(get(&job_id).company_stability, Some(4));
        assert_eq!(get(&other_id).company_stability, None);

        // Table and detail output print to stdout, which libtest captures and
        // the tests cannot read; run them as smoke checks only (Req 3.8, 3.9).
        let listed = crate::jobs::service::list_jobs(
            &conn,
            crate::jobs::service::JobFilters::default(),
        )
        .unwrap();
        super::output::format_jobs_table(&listed);
        let detail = crate::jobs::service::get_job_detail(&conn, &job_id)
            .unwrap()
            .unwrap();
        super::output::format_job_detail(&detail);
        handlers::handle_update(&conn, &paths, update_args(&[&job_id]), false, false).unwrap();

        // Clear (Req 3.2): twice, the second time on an already-unscored job.
        for _ in 0..2 {
            handlers::handle_update(
                &conn,
                &paths,
                update_args(&[&job_id, "--clear-company-stability"]),
                true,
                true,
            )
            .unwrap();
            let job = get(&job_id);
            assert_eq!(job.company_stability, None);
            assert_eq!((job.appeal, job.application_ease), (Some(2), Some(5)));
            assert_eq!(csv_cell(&paths, &job_id, "company_stability"), "");
        }
    }

    #[test]
    fn list_min_company_stability_filter() {
        let temp_dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_data_dir(temp_dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let conn = Connection::open(&paths.db_path).unwrap();
        migrate::migrate(&conn).unwrap();
        let scored = insert_job(&conn, "https://example.com/jobs/scored", "Scored");
        let unscored = insert_job(&conn, "https://example.com/jobs/unscored", "Unscored");
        handlers::handle_update(
            &conn,
            &paths,
            update_args(&[&scored, "--company-stability", "3"]),
            true,
            true,
        )
        .unwrap();

        // Without the flag (Req 8.2) the parsed args carry no minimum, and the
        // same filters handle_list builds include unscored jobs.
        let plain = list_args(&["--json"]);
        assert_eq!(plain.min_company_stability, None);
        handlers::handle_list(&conn, &paths, plain, true).unwrap();
        let ids = |min: Option<i64>| -> Vec<String> {
            crate::jobs::service::list_jobs(
                &conn,
                crate::jobs::service::JobFilters {
                    is_archived: Some(false),
                    min_company_stability: min,
                    ..Default::default()
                },
            )
            .unwrap()
            .into_iter()
            .map(|item| item.job.id)
            .collect()
        };
        let all = ids(None);
        assert!(all.contains(&scored) && all.contains(&unscored));
        assert_eq!(ids(Some(3)), vec![scored.clone()]);

        // No matches with --json (Req 8.3): succeeds and the payload is `[]`.
        let strict = list_args(&["--min-company-stability", "5", "--json"]);
        assert_eq!(strict.min_company_stability, Some(5));
        handlers::handle_list(&conn, &paths, strict, true).unwrap();
        let empty = ids(Some(5));
        assert!(empty.is_empty());
        let none: Vec<crate::models::JobListItem> = Vec::new();
        assert_eq!(serde_json::to_string_pretty(&none).unwrap(), "[]");
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
