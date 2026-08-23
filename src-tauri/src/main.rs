// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use job_tracker_lib::cli::args::Cli;

fn main() {
    let raw_args: Vec<String> = std::env::args().collect();

    // Check if macOS launched the GUI app (no args or single -psn argument)
    let is_gui_launch = raw_args.len() <= 1
        || (raw_args.len() == 2 && raw_args[1].starts_with("-psn_"));

    if is_gui_launch {
        job_tracker_lib::run();
        return;
    }

    // Attempt to parse CLI arguments
    match Cli::try_parse() {
        Ok(cli) => {
            // If explicit CLI command or --run-jobs flag was provided, run the CLI
            if cli.command.is_some() || cli.run_jobs {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                if let Err(e) = rt.block_on(job_tracker_lib::run_cli(cli)) {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
                return;
            }

            // Otherwise launch the GUI app
            job_tracker_lib::run();
        }
        Err(err) => {
            // Print clap's generated help or error message
            let _ = err.print();
            std::process::exit(if err.use_stderr() { 1 } else { 0 });
        }
    }
}
