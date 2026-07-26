// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let mut args = std::env::args().skip(1);
    let mut run_jobs = false;
    let mut data_dir = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--run-jobs" => run_jobs = true,
            "--data-dir" => data_dir = args.next().map(std::path::PathBuf::from),
            other if other.starts_with("--data-dir=") => {
                data_dir = Some(std::path::PathBuf::from(
                    other.trim_start_matches("--data-dir="),
                ));
            }
            _ => {}
        }
    }

    if run_jobs {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        if let Err(e) = rt.block_on(job_tracker_lib::run_jobs_cli(data_dir)) {
            eprintln!("jobs runner failed: {e}");
            std::process::exit(1);
        }
        return;
    }

    job_tracker_lib::run();
}
