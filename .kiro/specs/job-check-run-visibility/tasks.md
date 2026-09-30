# Implementation Plan: Job Check Run Visibility

## Overview

This plan implements the design in Rust (`src-tauri/`) and TypeScript/React (`desktop/`), following the design's module layout. It builds inward-out. Pure Rust types and state machines come first, then the progress contract, persistence, fetch hardening, and the evidence-based classifier. After that come the `RunCoordinator`, the Jobs_Cycle stages, and the legacy, CLI, and Tauri adapters. The frontend contract, reducer, UI, and page refresh follow, and the plan ends with surface and integration verification.

Constraints for every task:
- Tauri 2 only. No Node server, HTTP API, socket, MCP, or LLM.
- Keep the command names `run_jobs_cycle_cmd`, `check_all_postings_cmd`, `check_job_posting`, `jt sync`, and `--run-jobs`, and keep their JSON fields.
- Keep `operation_in_progress:runner` and the `jobs-runner.lock` flock.
- Keep the data-dir precedence in `db/paths.rs` and the current CSV mirror behavior.
- Do not modify `.config.kiro`, `requirements.md`, or `design.md`.

The five behavior changes listed at the end of the design are accepted and built in:
1. Blocked, consent-walled, and client-rendered pages become Unknown ("Couldn't confirm").
2. A posting absent from a provider listing closes after one check. If the page is still live but unlisted, the result is Unknown (conflict).
3. A failed stage lets the remaining stages run, and the run ends `completed_with_errors`.
4. A desktop posting check that changes state marks the CSV mirror dirty.
5. Posting checks still cover all jobs, including archived ones.

Test commands: `npm test` (runs `cargo test --lib`), `npm run test:desktop` (runs `vitest run`), and `npm run desktop:build` (runs `tsc -b && vite build`).

## Tasks

- [x] 1. Pure Rust run model, lifecycle, and ledger
  - [x] 1.1 Scaffold the `runs` module and domain types
    - Create `src-tauri/src/runs/mod.rs`. Declare `model`, `lifecycle`, `ledger`, `progress`, `store`, `coordinator`, `legacy`, `stages` (with `stages/mod.rs` declaring `postings`, `watches`, `careers`, `csv`), and `#[cfg(test)] pub(crate) mod test_support`. Each gets a doc-comment-only stub file so later tasks only edit their own file.
    - Add `mod runs;` to `src-tauri/src/lib.rs`.
    - In `src-tauri/src/runs/model.rs`, implement `RunId` (uuid v4), `RunType`, `RunStatus` with `is_terminal`, `PostingStatus`, `PostingState` (`Inactive` persisted as `"inactive"`), `StageName`, `StageOutcome`, `PostingCounts`, `JobIdentity`, and `Trigger` (`desktop|legacy_command|cli|launchd|retry`). Use the serde casing from the design.
    - Add `"test-util"` to the dev-dependency `tokio` features in `src-tauri/Cargo.toml`.
    - _Requirements: 1.1, 1.4, 3.10, 11.1_

  - [x] 1.2 Implement the pure lifecycle transition function
    - In `src-tauri/src/runs/lifecycle.rs`, implement `RunEvent` and `next_status(current, event) -> Result<RunStatus, LifecycleError>` exactly per the run state diagram.
    - `AllUnitsSettled` resolves to Canceled from Canceling, to CompletedWithErrors when `any_error` is set, and to Completed otherwise. Terminal statuses reject every event.
    - _Requirements: 1.3, 1.4, 1.5, 1.6, 1.7, 5.6_

  - [ ]* 1.3 Write property test for lifecycle transitions
    - **Property 1: Lifecycle transition soundness**
    - **Validates: Requirements 1.4, 1.5, 1.6, 5.6**

  - [x] 1.4 Implement the pure `RunLedger`
    - In `src-tauri/src/runs/ledger.rs`, implement `RunLedger::new` (rejects duplicate job ids), `transition`, `counts`, `queued_ids`, `next_queued`, `has_queued`, and `snapshot`.
    - Allowed transitions are `Queued→Active|Canceled|Error` and `Active→Completed|Error`. Any other request returns `IllegalTransition` and leaves the ledger unchanged.
    - Expose the pure predicates `is_retry_eligible(status, state)` (Unknown, Error, or Canceled) and `needs_attention(status, state)` (Unknown or Error). The frontend corpus and retry validation both use them.
    - _Requirements: 3.5, 3.10, 3.11, 5.8, 8.3, 8.4, 8.6, 9.7_

  - [ ]* 1.5 Write property test for ledger accounting
    - **Property 2: Ledger accounting invariant**
    - **Validates: Requirements 3.5, 3.11, 8.3, 8.4, 8.6**

- [x] 2. Progress contract v1 (Rust side)
  - [x] 2.1 Implement contract types, bounds, and event sinks
    - In `src-tauri/src/runs/progress.rs`, define:
      - `PROGRESS_CONTRACT_VERSION = 1` and `EVENT_NAME = "jobs-runner-progress"`.
      - The `MAX_*_BYTES` constants.
      - `bounded()`, which truncates on a UTF-8 boundary and appends `…`, and `BoundedCount`, which saturates at 2^53−1.
      - `RunProgressEvent`, `PostingProgress`, `StageProgress`, `RunSummary`, `RunSnapshot`, `RunAccepted`, and `EvidenceView`. Use camelCase fields and `skip_serializing_if` so optionals are absent rather than `null`.
    - Keep the legacy fields `stage`, `message`, `current`, `total`, and `done` with their current meanings. The final Jobs_Cycle event uses `stage: "cycle"`.
    - Implement the `RunEventSink` trait with `TauriSink` (`app.emit(EVENT_NAME, …)`), `LogSink` (the current `log::info!` behavior), and `RecordingSink` (for tests).
    - _Requirements: 1.9, 11.1, 11.2, 11.3, 11.4, 11.6, 11.7_

  - [x] 2.2 Implement the Run_Summary builder
    - In `src-tauri/src/runs/progress.rs`, add `build_run_summary(ledger, stages, state_at_start, timing)`. It emits all five outcome keys (including zeros), lists each Job_Identity once, and computes `stateChanges` against `state_at_start`.
    - _Requirements: 9.1, 9.2, 9.3, 9.8_

  - [ ]* 2.3 Write property test for contract bounds (Rust)
    - **Property 21: Contract values are always within bounds**
    - Arbitrary Unicode strings and `u64` counts. Assert that byte bounds hold, cuts land on character boundaries, and `current ≤ total ≤ 2^53−1`.
    - **Validates: Requirements 11.2, 11.3**

  - [ ]* 2.4 Write property test for contract serde round trip (Rust)
    - **Property 22: Progress contract round-trips across the Rust–TypeScript boundary** (Rust half)
    - Serialize and deserialize yields an equal value, and absent optionals never appear as `null`.
    - **Validates: Requirements 11.4, 11.5, 13.8**

  - [ ]* 2.5 Write property test for run summary consistency
    - **Property 19: Run summary is complete and consistent**
    - **Validates: Requirements 9.1, 9.2, 9.8**

- [x] 3. SQLite schema and RunStore
  - [x] 3.1 Add migrations for `runs`, `run_postings`, and `posting_check_evidence`
    - In `src-tauri/src/db/migrate.rs`, add the three tables and the indexes from the design: `runs_single_lock_owner_uidx`, `runs_status_started_idx`, `run_postings_run_status_idx`, `pce_one_authoritative_uidx`, and `pce_job_attempted_idx`. The migration must be additive and idempotent.
    - In `src-tauri/src/jobs/service.rs`, make `delete_job` also delete the job's `posting_check_evidence` rows. Keep its `run_postings` rows as history.
    - _Requirements: 4.8, 7.1, 7.7, 9.2_

  - [ ]* 3.2 Extend the migration tests
    - Extend `migrate_is_idempotent_for_additive_columns` and the legacy-schema test to assert that the new tables and indexes exist exactly once after repeated `migrate`.
    - Assert that `delete_job` removes evidence rows but keeps `run_postings` rows.
    - _Requirements: 4.8, 7.7_

  - [x] 3.3 Implement `runs::store`
    - In `src-tauri/src/runs/store.rs`, implement these functions exactly as specified in the design, using conditional SQL for every transition: `insert_accepted_run`, `cas_run_status`, `try_mark_posting_active` (a CAS that also requires the run to be `queued|active`), `finalize_posting`, `mark_posting_error`, `cancel_remaining_queued`, `request_cancel`, `finalize_run` (sets `duration_ms` and clears `owns_runner_lock`), `load_snapshot`, `load_current` (a non-terminal run first, else the latest undismissed terminal run), `recover_orphaned_runs` (`runner_interrupted`), `insert_supplementary_evidence`, `dismiss_run`, and `prune_history` (keep 50 runs, keep 20 authoritative rows per job plus the latest conclusive row, and delete supplementary rows older than 30 days).
    - _Requirements: 1.2, 1.7, 1.8, 2.5, 4.8, 5.4, 5.5, 5.11, 7.7, 8.8_

  - [ ]* 3.4 Write unit tests for the store
    - Cases:
      - `try_mark_posting_active` fails after `request_cancel`.
      - The partial unique index rejects a second lock owner.
      - `request_cancel` on a terminal run affects zero rows.
      - `prune_history` keeps the latest conclusive row.
      - `load_current` prefers a non-terminal run.
      - Orphan recovery closes out queued and active postings.
    - _Requirements: 4.8, 5.4, 5.11, 7.7_

- [x] 4. safe_fetch hardening
  - [x] 4.1 Add redirect chain, error kinds, signal headers, and async DNS
    - In `src-tauri/src/jobs/safe_fetch.rs`, add the fields `requested_url`, `redirect_statuses`, `error_kind: Option<FetchErrorKind>`, and `signal_headers` (allowlist: `cf-mitigated`) to `SafeFetchResult`. Add the `FetchErrorKind` enum.
    - Move `dns_lookup::lookup_host` into `tokio::task::spawn_blocking` inside a 5 s `tokio::time::timeout`, so DNS stops blocking a worker and the 30 s bound can be enforced.
    - Keep the private-address checks, the scheme allowlist, and the existing field meanings, so the `ats`, `metadata`, and `careers` callers are unaffected.
    - _Requirements: 6.12, 7.2, 7.5, 8.1_

  - [x]* 4.2 Write unit tests for safe_fetch changes
    - Cases:
      - The redirect statuses are recorded in order.
      - A DNS timeout maps to `DnsTimeout`.
      - Private and loopback destinations are still blocked.
      - Only allowlisted headers are retained.
    - _Requirements: 6.12, 7.2, 7.5, 7.9_

- [ ] 5. Evidence, signals, provider resolution, and classification
  - [x] 5.1 Implement `CheckEvidence` and URL sanitization
    - Create `src-tauri/src/jobs/posting_check/mod.rs`. It declares `evidence`, `signals`, `provider`, `fetch`, `classify`, `persist`, and `#[cfg(test)] mod fixture_tests`, each with a stub file. Register `posting_check` in `src-tauri/src/jobs/mod.rs`.
    - In `evidence.rs`, implement:
      - `CheckEvidence` (`evidence_version = 1`), `ProviderSignal`, `Provider`, `ProviderSignalKind`, `ContentSignal` (ordered, stored in a `BTreeSet`), and `FailureCategory`.
      - `CheckEvidence::timeout(...)` and an idempotent `normalized()`.
      - `sanitize_url`, which strips userinfo, fragments, and `;jsessionid`, and redacts secret-like query keys while keeping `gh_jid`.
      - An exhaustive `FetchErrorKind → FailureCategory` mapping and an `is_transient` predicate.
    - _Requirements: 6.12, 6.15, 7.1, 7.2, 7.3, 7.4, 7.5, 7.9_

  - [ ]* 5.2 Write unit tests for sanitization and failure mapping
    - An exhaustive `FetchErrorKind → FailureCategory` table.
    - URL sanitization examples for userinfo, fragment, `jsessionid`, token, session, and `gh_jid` retention.
    - _Requirements: 7.5, 7.9_

  - [x] 5.3 Implement page signal extraction
    - In `src-tauri/src/jobs/posting_check/signals.rs`, implement `norm()` and `extract_signals(html, identity, requested, final)` with `scraper`, following the signal table in the design.
    - Identity fields are `<title>`, `og:title`, the first `<h1>`, and JSON-LD. Detect enabled and disabled apply controls. Treat closure copy as matched only when the title matches or the final path equals the requested path.
    - Detect consent, auth, anti-bot (including the `cf-mitigated` header), access-denied, and generic-careers pages.
    - Discard body text right after extraction.
    - _Requirements: 6.2, 6.5, 6.13, 6.14, 7.4, 13.3_

  - [ ]* 5.4 Write property test for signal extraction
    - **Property 12: Page signal extraction soundness and normalization invariance**
    - Use template-generated HTML: a posting page, a careers index, a consent wall, a login page, a challenge page, an access-denied page, and a closure page. Vary case and whitespace.
    - **Validates: Requirements 6.2, 6.5, 6.13, 6.14, 13.3**

  - [x] 5.5 Implement provider target resolution and listing lookup
    - In `src-tauri/src/jobs/posting_check/provider.rs`, implement pure `resolve_provider_target`. The priority order is `source` + `source_external_id` first, then `discover_from_url`, then a single `company_watches` row.
    - Implement `ProviderListingCache` (one fetch per board per run through `OnceCell`).
    - Implement `lookup_listing(listing, posting_id) -> ProviderSignalKind`. Use `ats::parse_ats_jobs_from_json`. A parse or fetch failure yields `ListingUnavailable`.
    - _Requirements: 6.1, 6.4, 7.3_

  - [ ]* 5.6 Write property test for provider listing lookup
    - **Property 11: Provider listing lookup**
    - **Validates: Requirements 6.1, 6.4**

  - [x] 5.7 Implement the pure classifier
    - In `src-tauri/src/jobs/posting_check/classify.rs`, implement `classify(&CheckEvidence) -> Classification { state, reason_code, reason }` using the ordered decision procedure: blockers, then conflict, then positive, then closed, then Unknown.
    - Build a deterministic reason of at most 500 bytes that names every decisive category, and every blocker or transient category.
    - Add `format_last_check_result(state, reason)`, which produces `"{persisted_state}: {detail}"` so `checkResultNote` and CSV consumers keep working.
    - _Requirements: 6.3, 6.6, 6.7, 6.8, 6.9, 6.10, 6.11, 6.13, 6.14, 6.16, 10.5_

  - [ ]* 5.8 Write property test for the classification decision table
    - **Property 10: Classification decision table**
    - **Validates: Requirements 6.3, 6.6, 6.7, 6.8, 6.9, 6.10, 6.11, 6.14, 6.16**

  - [x] 5.9 Implement `PostingFetcher` and `evaluate`
    - In `src-tauri/src/jobs/posting_check/fetch.rs`, define the `PostingFetcher` trait and `HttpPostingFetcher`, which uses `safe_fetch` for pages and ATS listing endpoints.
    - In `posting_check/mod.rs`, implement `evaluate(identity, fetcher, cache, attempted_at) -> CheckEvidence` following the evaluation flowchart:
      - `ListedOpen` skips the page fetch.
      - `Absent` or `ListedClosed` fetches the page for confirmation, so a live but unlisted page becomes a conflict (Unknown).
      - A listing failure falls back to the page.
      - The final URL and final response decide, and the redirect evidence is retained.
    - _Requirements: 6.1, 6.2, 6.4, 6.8, 6.12, 7.2, 7.3_

  - [ ]* 5.10 Write property test for redirect evidence
    - **Property 14: Redirect evidence is retained and the final response decides**
    - Use a fake fetcher that produces redirect chains of length 0 to 5.
    - **Validates: Requirements 6.12, 7.2**

  - [x] 5.11 Build the classification fixture corpus and fixture tests
    - Add fixtures under `src-tauri/src/jobs/posting_check/fixtures/`:
      - Greenhouse, Lever, and Ashby listings (open, absent, malformed, empty).
      - Generic HTML: a JSON-LD posting with an apply button, closure copy, a Greenhouse `?error=true` redirect target, a Workday-style JS shell, a Cloudflare challenge, a consent wall, an SSO login, and access denied.
      - Status-only cases: 404, 410, 401, 403, 429, 500, 503, and timeout.
    - In `fixture_tests.rs`, add a table-driven test that runs each fixture through `evaluate` with a fake fetcher plus `classify`, and asserts the expected Posting_State and `reason_code`.
    - This task is required, not optional, because the fixture tests are the acceptance evidence for Requirements 13.1 and 13.2.
    - _Requirements: 6.1–6.14, 7.3, 13.1, 13.2_

  - [x] 5.12 Implement result persistence and the `check_job_posting` shim
    - In `src-tauri/src/jobs/posting_check/persist.rs`, implement `apply_classified_check(tx, run_id: Option<&str>, job_id, evidence, classification)`. Inside the caller's transaction it:
      - Updates only `posting_state`, `last_checked_at` (set to the attempt time), `last_check_result`, and `updated_at`.
      - Returns `job_missing` when no row is affected.
      - Inserts exactly one `posting_state_changed` `job_events` row when the state changes, with the previous state, new state, and reason.
      - Inserts the authoritative evidence row.
    - Add `preceding_conclusive_state(conn, job_id)`.
    - Reduce `src-tauri/src/jobs/check_active.rs` to a shim. `check_job_posting` keeps its signature and return fields, and now runs `evaluate` + `classify` + `apply_classified_check` with `run_id = NULL`, taking no runner lock.
    - _Requirements: 6.17, 7.1, 7.7, 7.8, 8.7, 10.5_

  - [ ]* 5.13 Write property test for result application and history
    - **Property 16: Result application changes only availability fields and records history exactly**
    - **Validates: Requirements 6.17, 7.7, 7.8, 8.7**

  - [ ]* 5.14 Write property test for evidence normalization
    - **Property 13: Evidence normalization is idempotent and round-trips**
    - Cover idempotence, classify invariance, the serde round trip, and the persist-then-load round trip.
    - **Validates: Requirements 6.15, 7.1**

- [x] 6. Checkpoint: pure layers, store, and classifier
  - Run `npm test`. Ensure all tests pass, ask the user if questions arise.

- [ ] 7. RunCoordinator
  - [x] 7.1 Implement accept with lock ownership and structured errors
    - In `src-tauri/src/error.rs`, add `AppError::Coded { code, category, message }`. It displays as `code:category`, so `operation_in_progress:runner` stays byte-identical. Add `AppError::code_parts()`, which also parses legacy `a:b` strings.
    - In `src-tauri/src/runs/mod.rs`, add `RunRegistry`, which maps each in-process run id to its cancel `watch::Sender`.
    - In `src-tauri/src/runs/coordinator.rs`, implement:
      - `RunConfig` (concurrency 4, timeout 30 s, grace 15 s, cancel poll 250 ms) and a `Clock` trait.
      - `RunLockGuard` (in-process `OwnedMutexGuard` + flock file), with a `Drop` that unlocks.
      - `RunRequest`, `RunRejection`, and `RunCoordinator::accept`, following the six-step accept algorithm:
        - Acquire both locks before any DB work.
        - Recover orphaned runs.
        - Build the posting set over all jobs, including archived ones, in stable order.
        - For retries, validate against a terminal source run using `is_retry_eligible`, dedupe the selection, and reject empty or ineligible selections with no writes.
        - Commit a single accept transaction with `owns_runner_lock = 1` and `last_seq = 1`.
        - Publish the `seq=1` Queued event with the full posting list.
    - _Requirements: 1.1, 1.2, 3.1, 3.9, 3.10, 4.3, 4.4, 4.8, 5.9, 5.10, 5.12, 10.13, 10.14, 10.15_

  - [x] 7.2 Implement the dispatch loop, workers, and finalization
    - In `src-tauri/src/runs/coordinator.rs`, implement `execute`. It is a single-owner `tokio::select!` loop with a 4-permit semaphore.
      - Each dispatch does a CAS through `try_mark_posting_active`. A lost CAS means cancel.
      - The run moves Queued→Active on the first start.
      - Workers send results over `mpsc`, and each result is attributed by `job_id`.
    - In `src-tauri/src/runs/stages/postings.rs`, implement the DB-free worker, which runs `evaluate` under the 30 s timeout. `JoinError` maps to `internal`.
    - Finalize each posting in one `BEGIN IMMEDIATE` transaction: `apply_classified_check` + `store::finalize_posting`. On failure, roll back, then `mark_posting_error(persistence)`. If that also fails, the run fails with `persistence_unavailable`.
    - Finalize the run in this order:
      - Derive the terminal status through `lifecycle::next_status`.
      - Call `finalize_run` with duration and summary.
      - Drop the lock guard.
      - Publish the `done=true` event last.
      - Run `prune_history` best-effort.
    - Every event gets `seq` + 1, `previousRunStatus` on status changes, and Job_Identity for posting deltas.
    - _Requirements: 1.3, 1.5, 1.6, 1.8, 1.9, 3.2, 3.3, 3.4, 3.5, 3.11, 3.12, 4.1, 4.2, 4.6, 4.7, 4.9, 7.1, 7.10, 8.1, 8.3, 8.4, 8.5, 8.6_

  - [x] 7.3 Implement cancellation (in-process and cross-process)
    - In `src-tauri/src/runs/coordinator.rs`, implement `RunCoordinator::cancel`. It calls `store::request_cancel`, returns `cancel_not_allowed:<status>` or `run_not_found` with no modification, and signals the registry `watch`.
    - In the loop, handle `cancel_rx.changed()`, plus a 250 ms poll of the run status so a Canceling state committed by another process is also picked up.
    - Once canceling, start nothing new, let in-flight postings reach Completed or Error, bulk-cancel the remaining queued postings, and move the run to Canceled.
    - _Requirements: 3.6, 5.3, 5.4, 5.5, 5.6, 5.7, 5.11_

  - [x] 7.4 Implement timeouts and supplementary late evidence
    - In `src-tauri/src/runs/stages/postings.rs` and `coordinator.rs`, add timeout handling:
      - At 30 s, send an authoritative `CheckEvidence::timeout` (Completed/Unknown, category `timeout`).
      - Keep the network task for up to 15 s of grace, holding its permit.
      - Store a late result through `insert_supplementary_evidence` only. The posting row, state, and counters never change.
      - Abort late tasks on cancel or when the grace deadline passes.
    - _Requirements: 8.1, 8.2, 8.8, 8.9_

  - [x] 7.5 Implement run-level failure and orphan handling
    - In `src-tauri/src/runs/coordinator.rs`, when the run cannot continue (the connection is unusable or the ledger and DB disagree):
      - Mark queued postings Canceled and in-flight postings Error (`run_aborted`).
      - Leave finalized rows untouched.
      - Set the run to Error with a non-empty `error_reason`.
      - Persist best-effort and always publish the terminal event.
    - Guarantee lock release on every path through `RunLockGuard`, including panics.
    - _Requirements: 1.7, 3.12, 4.9, 8.5_

  - [ ]* 7.6 Build the coordinator test harness
    - In `src-tauri/src/runs/test_support.rs`, add:
      - A scripted `FakePostingFetcher` with per-job latency and evidence, a max-concurrency probe, and a per-job overlap detector.
      - A controllable `Clock`.
      - A tempdir `DataPaths` builder with seeded jobs and companies.
      - A `RecordingSink`-based harness.
      - A table-digest helper for `runs`, `run_postings`, `posting_check_evidence`, `jobs`, and `job_events`.
    - Use `tokio::time::pause()`.
    - _Requirements: 13.4, 13.5, 13.6, 13.7_

  - [ ]* 7.7 Write property test for the event stream
    - **Property 3: Published event stream is well-formed**
    - **Validates: Requirements 1.1, 1.3, 1.9, 3.1, 3.2, 3.3, 3.4, 3.9, 3.10**

  - [ ]* 7.8 Write property test for bounded concurrency
    - **Property 4: Bounded, non-duplicating concurrency**
    - **Validates: Requirements 4.1, 4.2**

  - [ ]* 7.9 Write property test for completion-order independence
    - **Property 5: Completion-order independence**
    - **Validates: Requirements 4.6, 4.7, 13.4**

  - [ ]* 7.10 Write property test for cancellation at any boundary
    - **Property 6: Cancellation at any completion boundary**
    - **Validates: Requirements 3.6, 5.4, 5.5, 5.6, 5.7, 13.5**

  - [ ]* 7.11 Write integration test for cancel latency
    - With paused time, an in-process cancel emits Canceling immediately. A DB-only cross-process cancel is observed within the 250 ms poll, which is under the 1 s bound.
    - _Requirements: 5.3_

  - [ ]* 7.12 Write property test for terminal accounting and lock release
    - **Property 7: Terminal accounting, timing, and lock release on every path**
    - **Validates: Requirements 1.2, 1.7, 1.8, 3.12, 4.9, 8.5**

  - [ ]* 7.13 Write property test for side-effect-free rejections
    - **Property 8: Rejected requests are side-effect free**
    - Use k in 2–6 concurrent accepts against a real flock. Include invalid cancels and invalid retry selections. Compare table digests before and after.
    - **Validates: Requirements 4.3, 4.4, 4.8, 5.11, 5.12, 10.14, 10.15, 13.7**

  - [ ]* 7.14 Write property test for retry selection
    - **Property 9: Retry selects exactly the unique eligible entries**
    - **Validates: Requirements 5.9, 5.10, 13.6**

  - [ ]* 7.15 Write property test for finalize atomicity
    - **Property 17: Finalize is atomic under persistence failure**
    - Inject a failure at each statement of the finalize transaction.
    - **Validates: Requirements 3.4, 7.10**

  - [ ]* 7.16 Write property test for timeouts and supplementary evidence
    - **Property 18: Timeouts finalize at 30 s and late results stay supplementary**
    - **Validates: Requirements 8.1, 8.2, 8.8, 8.9**

  - [ ]* 7.17 Write property test for secret and body exclusion
    - **Property 15: Secrets and bodies never leave the fetch layer**
    - Put canary strings in URLs, bodies, and non-allowlisted headers. Assert that the serialized evidence, persisted row, `last_check_result`, and recorded events never contain them.
    - **Validates: Requirements 7.4, 7.9**

- [x] 8. Jobs_Cycle stages
  - [x] 8.1 Move the watches, careers, and CSV stages out of `runner.rs`
    - Move the existing logic, unchanged, into `src-tauri/src/runs/stages/watches.rs` (concurrency 2, 30 s), `careers.rs` (concurrency 4, 30 s), and `csv.rs` (`sync_jobs_csv_with_disk`).
    - Each stage returns item-level results plus a stage-level `Err` only for apply or DB failures, and reports `StageProgress`.
    - `src-tauri/src/runner.rs` keeps `try_lock_runner`, `open_runner_conn`, and `run_jobs_cli` with their existing signatures.
    - _Requirements: 2.2, 10.9, 10.10_

  - [x] 8.2 Wire stage sequencing and CSV behavior into the coordinator
    - In `src-tauri/src/runs/coordinator.rs`, run the stages in the fixed order `postings → watches → careers → csv`.
      - A stage starts only if the run is not canceling.
      - A stage failure is recorded as `failed`, and the next stage still runs, so the run ends Completed_With_Errors.
      - At the terminal state, every unstarted stage becomes `skipped`.
      - `any_error` is true when any posting has Error status or any stage failed.
    - CSV rules:
      - A canceled cycle in the GUI process calls `csv_export.mark_dirty()`.
      - A Posting_Check_Run in the GUI process marks the mirror dirty when any posting state changed.
      - CLI posting checks write no CSV.
    - _Requirements: 1.6, 2.2, 9.3, 9.4, 10.10_

  - [ ]* 8.3 Write property test for stage outcomes
    - **Property 20: Stage outcomes are consistent at terminal**
    - **Validates: Requirements 2.2, 9.3, 9.4**

  - [ ]* 8.4 Write unit tests for CSV mirror behavior
    - Cases:
      - A GUI posting check that changes state marks the mirror dirty.
      - A GUI posting check with no change leaves it clean.
      - A CLI posting check does not write the CSV.
      - A successful Jobs_Cycle exports the CSV.
      - A canceled Jobs_Cycle marks it dirty.
    - _Requirements: 10.10_

- [ ] 9. Legacy command and CLI adapters
  - [x] 9.1 Implement the legacy projections
    - In `src-tauri/src/runs/legacy.rs`, implement `jobs_cycle_summary(&RunSnapshot)` and `postings_summary(&RunSnapshot)`. They produce the existing fields (`postings` = completed posting results, `watches`, `careers`, `csv {imported, exported}`) plus the additive `runId` and `runStatus`.
    - Terminal mapping: Canceled becomes `Err("run_canceled:<runId>")`, and Error becomes `Err("run_failed:<reason_code>")`. Completed_With_Errors becomes `Ok`.
    - _Requirements: 10.2, 10.3, 10.4_

  - [x] 9.2 Delegate `runner.rs` and the legacy Tauri commands to the coordinator
    - `runner::run_jobs_cycle`, `runner::check_all_postings`, and `run_jobs_cli` keep their signatures and call `accept` + `execute`, then project through `legacy`. `--run-jobs` uses trigger `Launchd` and `LogSink`.
    - In `src-tauri/src/commands/mod.rs`, make `run_jobs_cycle_cmd` and `check_all_postings_cmd` use trigger `LegacyCommand` and `TauriSink`, so they also publish v1 events. `check_job_posting` uses the 5.12 shim and keeps its existing `mark_dirty` call.
    - `commands::sync_watch` and the CSV commands keep calling `try_lock_runner` unchanged.
    - _Requirements: 4.3, 10.2, 10.3, 10.4, 10.5, 10.6, 10.9, 10.10, 11.6_

  - [x] 9.3 Route CLI sync through the coordinator and add structured JSON errors
    - In `src-tauri/src/cli/handlers.rs`, make `jt sync` and `--run-jobs` call the coordinator with trigger `Cli` and `LogSink` after `DataPaths` is resolved. Data-dir resolution is untouched.
    - In `src-tauri/src/cli/output.rs`, add a writer abstraction for `print_json` and error output.
    - In `src-tauri/src/main.rs`, when `--json` is set and `run_cli` fails, print exactly one `{"ok":false,"error":{"code","category","message"}}` object to stdout and exit 1. The stderr `Error:` line is printed only when `!quiet`. Successful output is unchanged.
    - _Requirements: 10.6, 10.7, 10.8, 10.9, 10.13, 10.14_

  - [x] 9.4 Write golden JSON compatibility tests for the legacy surfaces
    - In `src-tauri/src/runs/legacy.rs` tests, add golden key-set and value-meaning tests for the `run_jobs_cycle`, `check_all_postings`, and `check_job_posting` projections, and for the legacy event fields `stage/message/current/total/done`, including the final `stage: "cycle"` event.
    - Add an error-mapping test for Canceled, Error, and `operation_in_progress:runner`.
    - This task is required, not optional, because these tests guard the compatibility constraint and are the acceptance evidence for Requirement 13.9.
    - _Requirements: 10.3, 10.4, 10.5, 11.6, 11.7, 13.9_

  - [ ]* 9.5 Write unit tests for CLI JSON output
    - Assert exactly one parseable JSON document on success and on error, no extra stdout under `--quiet`, and the `code` and `category` fields for `operation_in_progress:runner`.
    - _Requirements: 10.7, 10.8, 10.13_

  - [ ]* 9.6 Write regression tests for data-dir precedence
    - In `src-tauri/src/db/paths.rs`, test that `--data-dir` beats `JOB_TRACKER_DATA_DIR`, which beats the debug repo `data/`, which beats Application Support.
    - _Requirements: 10.9_

  - [ ]* 9.7 Write headless sync integration test
    - Run `handle_sync` against an empty tempdir data directory. Assert that it completes without the UI, writes the CSV mirror, and prints the legacy payload.
    - _Requirements: 10.6, 10.10_

- [x] 10. New Tauri run commands
  - [x] 10.1 Implement and register `commands::runs`
    - Create `src-tauri/src/commands/runs.rs` with `start_run_cmd`, `retry_run_cmd`, `cancel_run_cmd`, `get_run_cmd`, `get_current_run_cmd`, and `dismiss_run_cmd`, following the command table in the design. Add `pub mod runs;` to `commands/mod.rs`.
      - `start_run_cmd` and `retry_run_cmd` run `accept`, register the cancel handle, spawn `execute` on `tauri::async_runtime`, and return `RunAccepted` immediately.
      - `get_current_run_cmd` opportunistically recovers orphans when both locks can be acquired and released immediately.
      - Snapshots set `live` according to whether the run is registered in this process.
    - Add `RunRegistry` to `AppState`, and register the commands in `generate_handler!` in `src-tauri/src/lib.rs` alongside the unchanged legacy commands.
    - _Requirements: 1.1, 2.3, 2.5, 2.7, 4.5, 5.3, 5.9, 5.11, 5.12, 10.1, 10.12, 11.11_

  - [x] 10.2 Write the golden contract corpus generator
    - Add a test in `src-tauri/src/runs/progress.rs` that generates a fixed-seed sample of events, snapshots, `is_retry_eligible`/`needs_attention` cases, and the `MAX_*_BYTES` constants into `desktop/src/lib/__fixtures__/run-contract-corpus.json`.
    - The test rewrites the file under `UPDATE_CONTRACT_CORPUS=1`. Otherwise it fails when the checked-in file differs from what it would generate.
    - _Requirements: 11.4, 11.5, 13.8_

  - [ ]* 10.3 Write unit tests for the run commands
    - Cases:
      - `dismiss_run_cmd` rejects a non-terminal run.
      - `cancel_run_cmd` on a terminal run returns `cancel_not_allowed`.
      - `get_current_run_cmd` returns a non-terminal run before an undismissed terminal one.
      - `retry_run_cmd` returns `retry_ineligible`.
    - _Requirements: 2.3, 5.11, 5.12_

- [x] 11. Checkpoint: backend complete
  - Run `npm test` and `cargo clippy --manifest-path src-tauri/Cargo.toml`. Confirm that `jt sync --json` against a temp `--data-dir` still returns the legacy payload through the automated tests. Ensure all tests pass, ask the user if questions arise.

- [x] 12. Frontend contract, API, and RunMonitor state
  - [x] 12.1 Implement the TS contract, validators, errors, and API wrappers
    - Create `desktop/src/lib/run-contract.ts`. It contains the contract v1 types, `PROGRESS_CONTRACT_VERSION`, bound constants mirrored from Rust, and hand-written `parseRunEvent` and `parseRunSnapshot`. The parsers reject malformed input, unsupported versions, over-bound values, and `null` optionals.
    - Create `desktop/src/lib/run-errors.ts` with `parseAppError("code:category[:detail]")`.
    - In `desktop/src/lib/api.ts`, add `startRun`, `retryRun`, `cancelRun`, `getRun`, `getCurrentRun`, `dismissRun`, and `listenRunProgress`, all through `invoke`/`listen`. Fix the `JobsRunnerProgress` drift from `phase` to `stage`.
    - Add `fast-check` and `jsdom` as devDependencies pinned to exact versions in `desktop/package.json`.
    - _Requirements: 10.1, 10.12, 11.1, 11.2, 11.3, 11.4, 11.5, 11.7, 11.8, 11.9_

  - [ ]* 12.2 Write property test for TS bound validation
    - **Property 21: Contract values are always within bounds** (TypeScript validator half)
    - Place it in `desktop/src/lib/run-contract.test.ts`.
    - **Validates: Requirements 11.2, 11.3**

  - [x] 12.3 Implement the pure RunMonitor reducer and selectors
    - Create `desktop/src/lib/run-state.ts` with `RunViewState` and `reduceRun`, handling `accepted`, `event`, `snapshotLoaded`, `snapshotFailed`, `invalidEvent`, `unsupportedVersion`, `dismissed`, `rejectedInProgress`, and `clearNotice`. Reducer rules:
      - Ignore events for a different run id or type, except a `seq=1` Queued event for a newer run.
      - Treat `seq ≤ lastSeq` as a duplicate.
      - Treat a sequence gap as `needsReconcile`.
      - Keep `lastValid` on invalid input.
      - Never create a displayed run from `rejectedInProgress`.
    - Add these selectors: `canCancel`, `isRetryEligible`, `needsAttention`, `runViewModel`, status labels for all Run_Status and Posting_Status values, and `buildAnnouncements` (polite throttled at 10% or 5 s, assertive once per `runId:jobId` or `runId:run`).
    - _Requirements: 2.1, 2.3, 2.6, 2.7, 2.8, 2.9, 3.7, 3.8, 4.5, 5.1, 5.2, 5.8, 9.7, 11.8, 11.9, 11.10, 11.11, 11.12, 12.3, 12.4, 12.6_

  - [ ]* 12.4 Write property test for unrelated events
    - **Property 23: Reducer ignores unrelated events**
    - Place it in `desktop/src/lib/run-state.test.ts`.
    - **Validates: Requirements 2.6, 11.10**

  - [ ]* 12.5 Write property test for event folding and reconciliation
    - **Property 24: Folding events reproduces the backend snapshot, with reconciliation after gaps**
    - **Validates: Requirements 2.7, 3.7, 11.11**

  - [ ]* 12.6 Write property test for invalid-input resilience
    - **Property 25: Invalid inputs never corrupt the last valid state**
    - **Validates: Requirements 2.8, 11.8, 11.9, 11.12**

  - [ ]* 12.7 Write property test for summary persistence until dismissed
    - **Property 26: Terminal summary stays until dismissed or superseded**
    - **Validates: Requirements 2.3, 2.9**

  - [ ]* 12.8 Write property test for presentation text
    - **Property 28: Presentation always carries required text**
    - **Validates: Requirements 2.1, 3.8, 12.6**

  - [ ]* 12.9 Write property test for announcements
    - **Property 29: Announcements are polite for progress and assertive once per error**
    - **Validates: Requirements 12.3, 12.4**

  - [ ]* 12.10 Write unit examples for reducer selectors
    - Cases:
      - `canCancel` across all 7 run statuses.
      - `rejectedInProgress` sets the "Another run is in progress" notice without a displayed run.
      - A `clearNotice` expiry.
    - _Requirements: 4.5, 5.1, 5.2_

  - [ ]* 12.11 Write property test for the Rust–TS round trip against the golden corpus
    - **Property 22: Progress contract round-trips across the Rust–TypeScript boundary** (TypeScript half)
    - Add a fast-check round trip, and parse and canonically re-serialize every entry of `__fixtures__/run-contract-corpus.json`. Assert that the bound constants equal the corpus constants.
    - **Validates: Requirements 11.1, 11.4, 11.5, 13.8**

  - [ ]* 12.12 Write property test for retry eligibility and the attention filter
    - **Property 27: Retry eligibility and attention filter are exact**
    - Include agreement with the Rust predicate cases in the golden corpus.
    - **Validates: Requirements 5.8, 9.7**

- [ ] 13. RunMonitor provider, UI components, accessibility, and page refresh
  - [x] 13.1 Implement `RunMonitorProvider`
    - Create `desktop/src/lib/RunMonitorContext.tsx`. It includes:
      - A single `listenRunProgress` subscription that validates through `parseRunEvent`.
      - A reconciler that calls `getCurrentRun` on mount and `getRun` on `needsReconcile`, debounced at 250 ms with backoff from 250 ms to 4 s.
      - Polling every 1 s while an external (`live=false`) run is non-terminal, and every 5 s for discovery when no run is displayed.
      - The actions `start`, `cancel`, `retry`, and `dismiss`, which map `operation_in_progress:runner` to `rejectedInProgress`.
      - `onRunSettled(cb)`, which fires once per run on its terminal transition.
    - In `desktop/src/App.tsx`, wrap `<BrowserRouter>` with the provider.
    - _Requirements: 2.4, 2.5, 2.7, 4.5, 9.5, 9.6, 10.1, 11.10, 11.11, 11.12, 12.1_

  - [x] 13.2 Implement the posting list and evidence disclosure
    - Create `desktop/src/components/runs/RunPostingList.tsx`. It has:
      - Rows keyed by `jobId` showing title, company, a status text pill, and the Posting_State label plus reason when completed.
      - A filter for all rows or rows needing attention.
      - Per-row retry checkboxes limited to eligible rows, and a "Select all needing attention" control.
    - Create `desktop/src/components/runs/EvidenceDisclosure.tsx`: a native `<button aria-expanded aria-controls>` that reveals the reason, HTTP status, requested and final URL, redirect statuses, provider signal, content categories, and failure category.
    - _Requirements: 3.7, 3.8, 5.8, 7.6, 9.7, 12.5, 12.6, 12.8_

  - [x] 13.3 Implement live regions and reduced motion
    - Create `desktop/src/components/runs/RunLiveRegions.tsx`: a visually hidden polite `role="status"` region and an assertive region, both fed by `buildAnnouncements`. It never calls `focus()`.
    - In `desktop/src/index.css`, add a `prefers-reduced-motion: reduce` rule that disables spinner and progress animation. The static `n / total` text and the bar always render.
    - _Requirements: 12.3, 12.4, 12.7, 12.8_

  - [x] 13.4 Implement run controls, panel, and summary, and wire them into Layout
    - Create `desktop/src/components/runs/RunControls.tsx`. It has the Run jobs and Check postings native buttons, which are `disabled` while the displayed run is non-terminal, plus the in-progress notice.
    - Create `RunPanel.tsx`. It shows the run id, type, status text, current stage, completed and total counts, elapsed time, and a stage list for Jobs_Cycle. It also has the Cancel button (enabled only for queued and active runs), the retry action, the transient notices, `RunPostingList`, and `RunLiveRegions`.
    - Create `RunSummaryCard.tsx`. It shows the outcome counts including zeros, the state changes, the stage outcomes, and a Dismiss button.
    - In `desktop/src/components/Layout.tsx`, render `RunControls` in the header and `RunPanel` above `<Outlet/>`. Replace the internals and listener of `desktop/src/components/RunJobsButton.tsx` with `RunControls`, or remove it, and remove its tooltip progress.
    - _Requirements: 2.1, 2.2, 2.3, 4.5, 5.1, 5.2, 9.1, 9.3, 9.4, 12.1, 12.2, 12.5, 12.6_

  - [x] 13.5 Refresh pages on settle and update posting-state presentation
    - `desktop/src/pages/JobsPage.tsx`: remove the private `jobs-runner-progress` listener and `checkProgress` state, and use `useRunMonitor().start("postingCheck")`.
    - `JobsPage`, `JobDetailPage` (via `JobDetailClient`), and `CompaniesPage`: subscribe to `onRunSettled` and call their existing `load({ quiet: true })`. When the refresh fails, keep the previous data and show an inline refresh error.
    - `desktop/src/lib/ui.ts`: add a `lastCheckedAt` argument to `postingStatePresentation`, so a checked `unknown` posting reads "Couldn't confirm". Update the callers in `JobsPage.tsx`, `JobsBoardView.tsx`, and `JobDetailClient.tsx`.
    - _Requirements: 3.8, 9.5, 9.6, 9.9, 12.1, 12.6_

  - [x] 13.6 Write jsdom component tests
    - Use `// @vitest-environment jsdom` in files under `desktop/src/components/runs/*.test.tsx`. Cases:
      - `EvidenceDisclosure` toggles `aria-expanded` and reveals the reason.
      - The start buttons expose `disabled` during a run.
      - Focus stays on a focused row button across event updates.
      - A route change keeps the provider state.
      - Stage presentation renders every outcome as text.
    - _Requirements: 2.2, 2.4, 7.6, 12.1, 12.2, 12.5, 12.8_

  - [ ]* 13.7 Write RunMonitor integration tests
    - In `desktop/src/lib/RunMonitorContext.test.tsx`, mock `invoke`/`listen` and use fake timers. Cases:
      - Restore on mount applies a non-terminal snapshot within 2 s.
      - A gap triggers reconciliation within 2 s.
      - `onRunSettled` fires once, and page loaders run within 5 s for Completed, Completed_With_Errors, and Canceled.
      - A refresh failure keeps the previous data.
    - _Requirements: 2.5, 2.7, 9.5, 9.6, 9.9_

  - [x] 13.8 Write unit tests for posting-state presentation
    - Cases: `unknown` without `lastCheckedAt` reads "Not checked yet", `unknown` with it reads "Couldn't confirm", and `active` and `inactive` keep their labels.
    - _Requirements: 3.8, 12.6_

- [ ] 14. Surface verification
  - [x] 14.1 Write surface smoke checks
    - Add `src-tauri/src/surface_tests.rs`, declared under `#[cfg(test)]` in `lib.rs`. It asserts:
      - Every command invoked in `desktop/src/lib/api.ts` is registered in `generate_handler!`, including the three legacy names.
      - `desktop/src` has no `fetch(` or `WebSocket` usage against localhost.
      - `src-tauri` adds no HTTP listener.
    - _Requirements: 10.1, 10.2, 10.11, 10.12_

- [~] 15. Final checkpoint: full verification
  - Run `npm test`, `cargo clippy --manifest-path src-tauri/Cargo.toml`, `npm run test:desktop`, `npm run desktop:build`, and the desktop oxlint config. Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for a faster MVP. Skipping them leaves the matching correctness properties unverified.
- Tasks 5.11 (fixture corpus) and 9.4 (legacy JSON compatibility) are test tasks left unmarked on purpose. They are the acceptance evidence for Requirements 13.1, 13.2, and 13.9, and they guard the command and JSON compatibility constraints.
- Each property test runs at least 100 cases (`ProptestConfig::with_cases(100)` or `fc.assert(..., { numRuns: 100 })`) and carries the tag comment `Feature: job-check-run-visibility, Property N: <title>`.
- Tasks 12.11 and 12.12 come last in group 12 because they need the golden corpus from 10.2.
- Rust test modules stay in the file named by each task, as the design's test placement specifies. The dependency graph serializes tasks that share a file.
- Manual VoiceOver, keyboard, and reduced-motion review is outside the coding tasks. Full WCAG validation still requires manual assistive-technology testing.

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "4.1"] },
    { "id": 1, "tasks": ["1.2", "1.4", "3.1", "4.2", "5.1"] },
    { "id": 2, "tasks": ["1.3", "1.5", "2.1", "3.2", "5.2", "5.3", "5.5"] },
    { "id": 3, "tasks": ["2.2", "3.3", "5.4", "5.6", "5.7", "12.1"] },
    { "id": 4, "tasks": ["2.3", "3.4", "5.8", "5.9", "5.12", "12.2", "12.3"] },
    { "id": 5, "tasks": ["2.4", "5.10", "5.13", "12.4"] },
    { "id": 6, "tasks": ["2.5", "5.11", "5.14", "7.1", "12.5"] },
    { "id": 7, "tasks": ["7.2", "12.6"] },
    { "id": 8, "tasks": ["7.3", "7.6", "12.7"] },
    { "id": 9, "tasks": ["7.4", "12.8"] },
    { "id": 10, "tasks": ["7.5", "12.9"] },
    { "id": 11, "tasks": ["7.7", "8.1", "12.10"] },
    { "id": 12, "tasks": ["8.2", "9.1"] },
    { "id": 13, "tasks": ["7.8", "9.2"] },
    { "id": 14, "tasks": ["7.9", "9.3", "9.4", "10.1"] },
    { "id": 15, "tasks": ["7.10", "9.5", "9.6", "10.2", "10.3", "13.1"] },
    { "id": 16, "tasks": ["7.11", "9.7", "12.11", "12.12", "13.2"] },
    { "id": 17, "tasks": ["7.12", "13.3"] },
    { "id": 18, "tasks": ["7.13", "13.4"] },
    { "id": 19, "tasks": ["7.14", "13.5"] },
    { "id": 20, "tasks": ["7.15", "13.6", "13.8"] },
    { "id": 21, "tasks": ["7.16", "13.7"] },
    { "id": 22, "tasks": ["7.17"] },
    { "id": 23, "tasks": ["8.3"] },
    { "id": 24, "tasks": ["8.4", "14.1"] }
  ]
}
```
