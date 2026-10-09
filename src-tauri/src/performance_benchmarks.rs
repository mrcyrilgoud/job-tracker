//! Report-only performance harnesses, deliberately separated from correctness
//! tests. Run with `cargo test --lib performance_benchmarks -- --ignored --nocapture`.
//! Each fixture is isolated and deterministic; timings are reports, never gates.

use std::time::{Duration, Instant};

fn report_samples(name: &str, workload: &str, mut samples: Vec<Duration>) {
    samples.sort_unstable();
    let p50 = samples[(samples.len() - 1) / 2];
    let p95 = samples[((samples.len() * 95).div_ceil(100) - 1).min(samples.len() - 1)];
    println!(
        "{name}: {workload}; samples={}; p50={:.3}ms; p95={:.3}ms",
        samples.len(),
        p50.as_secs_f64() * 1000.0,
        p95.as_secs_f64() * 1000.0
    );
}

mod jobs_page {
    use super::*;
    use rusqlite::{params, Connection};

    use crate::db::migrate::migrate;
    use crate::jobs::service::{list_jobs_page, JobFilters};

    fn seeded_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let timestamp = "2026-01-01T00:00:00Z";
        conn.execute(
            "INSERT INTO companies (id, name, created_at, updated_at) VALUES ('bench-company', 'Benchmark Co', ?1, ?1)",
            [timestamp],
        ).unwrap();
        let description =
            "Synthetic engineering role with detailed responsibilities and qualifications. "
                .repeat(110);
        let tx = conn.unchecked_transaction().unwrap();
        for index in 0..5_000 {
            let id = format!("bench-job-{index:04}");
            let title = if index == 4_999 {
                "Software Engineer, Sandboxing".to_string()
            } else {
                format!("Software Engineer {index}")
            };
            let url = format!("https://example.test/jobs/{index}");
            tx.execute(
                "INSERT INTO jobs (id, company_id, title, url, canonical_url, status, posting_state,
                                   source, description, is_new_from_watch, missing_from_sync_count,
                                   created_at, updated_at)
                 VALUES (?1, 'bench-company', ?2, ?3, ?3, 'wishlist', 'active', 'manual', ?4, 0, 0, ?5, ?5)",
                params![id, title, url, description, timestamp],
            ).unwrap();
        }
        tx.commit().unwrap();
        conn
    }

    #[test]
    #[ignore = "report-only Jobs page performance measurement"]
    fn search_and_page_5000_jobs() {
        let conn = seeded_connection();
        let filters = JobFilters {
            search: Some("sandbo".into()),
            ..JobFilters::default()
        };
        for _ in 0..3 {
            std::hint::black_box(list_jobs_page(&conn, filters.clone(), None).unwrap());
        }
        let mut samples = Vec::with_capacity(30);
        for _ in 0..30 {
            let started = Instant::now();
            let page = list_jobs_page(&conn, filters.clone(), None).unwrap();
            samples.push(started.elapsed());
            std::hint::black_box(page.jobs.len());
        }
        report_samples(
            "jobs_page.search",
            "5,000 jobs with 8 KB descriptions; one matching role",
            samples,
        );

        let mut samples = Vec::with_capacity(30);
        for _ in 0..30 {
            let started = Instant::now();
            let page = list_jobs_page(&conn, JobFilters::default(), None).unwrap();
            samples.push(started.elapsed());
            std::hint::black_box(page.jobs.len());
        }
        report_samples(
            "jobs_page.first_page",
            "5,000 jobs; 100 compact summary rows",
            samples,
        );
    }
}

mod ats_sync {
    use super::*;
    use rusqlite::Connection;

    use crate::ats::sync::apply_watch_sync;
    use crate::ats::AtsJob;
    use crate::companies::{create_company, insert_watch};
    use crate::db::migrate::migrate;

    #[test]
    #[ignore = "report-only ATS matching measurement"]
    fn sync_matches_1000_existing_external_ids() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let company = create_company(&conn, "Benchmark ATS Co", None).unwrap();
        let watch = insert_watch(&conn, &company.id, "greenhouse", "benchmark-co").unwrap();
        let postings = (0..1_000)
            .map(|index| AtsJob {
                external_id: format!("role-{index}"),
                title: format!("Platform Engineer {index}"),
                url: format!("https://boards.greenhouse.io/benchmark-co/jobs/{index}"),
                location: Some("Remote".into()),
            })
            .collect::<Vec<_>>();
        apply_watch_sync(&conn, &watch.id, Ok(postings.clone())).unwrap();

        let mut samples = Vec::with_capacity(10);
        for _ in 0..10 {
            let started = Instant::now();
            let result = apply_watch_sync(&conn, &watch.id, Ok(postings.clone())).unwrap();
            samples.push(started.elapsed());
            std::hint::black_box(result["created"].clone());
        }
        report_samples(
            "ats_sync.match",
            "1,000 existing postings matched by external ID",
            samples,
        );

        let existing_ids = (0..1_000)
            .map(|index| format!("role-{index}"))
            .collect::<Vec<_>>();
        let remote_ids = existing_ids.iter().rev().collect::<Vec<_>>();
        let mut scan_samples = Vec::with_capacity(30);
        let mut index_samples = Vec::with_capacity(30);
        for _ in 0..30 {
            let started = Instant::now();
            for remote_id in &remote_ids {
                let _ = existing_ids
                    .iter()
                    .find(|existing_id| existing_id.as_str() == remote_id.as_str());
            }
            scan_samples.push(started.elapsed());

            let started = Instant::now();
            let index = existing_ids
                .iter()
                .enumerate()
                .map(|(position, id)| (id.as_str(), position))
                .collect::<std::collections::HashMap<_, _>>();
            for remote_id in &remote_ids {
                let _ = index.get(remote_id.as_str());
            }
            index_samples.push(started.elapsed());
        }
        report_samples(
            "ats_sync.linear_scan",
            "1,000 reverse-ordered remote IDs against 1,000 local IDs",
            scan_samples,
        );
        report_samples(
            "ats_sync.index_build_and_match",
            "same 1,000-ID fixture; includes building the external-ID index",
            index_samples,
        );
    }
}

mod runner_poll {
    use super::*;

    use crate::db::paths::DataPaths;
    use crate::runner::{open_runner_conn, open_runner_status_conn};
    use crate::runs::store;

    #[test]
    #[ignore = "report-only cancellation polling measurement"]
    fn compare_reopening_migrating_and_reusing_status_connection() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_data_dir(dir.path().to_path_buf());
        drop(open_runner_conn(&paths).unwrap());
        let status = open_runner_status_conn(&paths).unwrap();
        let mut reopen_samples = Vec::with_capacity(30);
        for _ in 0..30 {
            let started = Instant::now();
            let conn = open_runner_conn(&paths).unwrap();
            let _ = store::run_status(&conn, "missing-benchmark-run").unwrap();
            reopen_samples.push(started.elapsed());
        }
        let mut reuse_samples = Vec::with_capacity(30);
        for _ in 0..30 {
            let started = Instant::now();
            let _ = store::run_status(&status, "missing-benchmark-run").unwrap();
            reuse_samples.push(started.elapsed());
        }
        report_samples(
            "runner_poll.reopen",
            "30 status reads; each opens a connection and runs migrations",
            reopen_samples,
        );
        report_samples(
            "runner_poll.reuse",
            "30 status reads on one migration-free connection",
            reuse_samples,
        );
    }
}

mod network_client {
    use super::*;
    use reqwest::{redirect::Policy, Client};

    use crate::jobs::safe_fetch::shared_client;

    #[test]
    #[ignore = "report-only HTTP client setup measurement"]
    fn compare_client_construction_and_shared_client_access() {
        let mut build_samples = Vec::with_capacity(100);
        for _ in 0..100 {
            let started = Instant::now();
            let client = Client::builder()
                .redirect(Policy::none())
                .timeout(Duration::from_millis(10_000))
                .user_agent("JobTrackerLocal/1.0")
                .build()
                .unwrap();
            build_samples.push(started.elapsed());
            drop(client);
        }
        let mut shared_samples = Vec::with_capacity(100);
        for _ in 0..100 {
            let started = Instant::now();
            let client = shared_client().unwrap();
            shared_samples.push(started.elapsed());
            drop(client);
        }
        report_samples(
            "network_client.construct",
            "100 builds with safe_fetch client configuration",
            build_samples,
        );
        report_samples(
            "network_client.shared",
            "100 accesses to process-wide configured client",
            shared_samples,
        );
    }
}
