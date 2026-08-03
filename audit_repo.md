# Job Tracker Architecture Audit

**Scope:** React rendering/state, Tauri commands, SQLite access, and asynchronous workflows. This review deliberately excludes visual styling, CSS linting, and boilerplate.

## Discovery map

| Layer | Primary locations | Current responsibility |
| --- | --- | --- |
| Route/UI tree | `desktop/src/App.tsx`, `desktop/src/pages/*` | Each route owns its own fetch lifecycle and page-local state. |
| Interactive state | `desktop/src/components/JobDetailClient.tsx`, `CompaniesClient.tsx`, `GmailClient.tsx`, `CompanyWatchAutomation.tsx` | Form drafts, busy/error state, and mutation follow-up reloads. There is no shared query cache or request coordinator. |
| UI-to-backend boundary | `desktop/src/lib/api.ts` | Thin, direct `invoke` wrappers; no cancellation, invalidation, pagination, or mutation serialization. |
| Tauri command layer | `src-tauri/src/commands/mod.rs` | Commands make synchronous `with_db` calls around a single in-process SQLite connection, while network work is generally separated from the mutex. |
| Data/state layer | `src-tauri/src/jobs/service.rs`, `companies.rs`, `documents.rs`, `ats/sync.rs`, `gmail/poll.rs` | SQLite reads/writes, watch sync, document import/attach, and Gmail classification. |
| Background work | `src-tauri/src/runner.rs`, `src-tauri/src/jobs/csv.rs` | LaunchAgent/manual runner, posting checks, ATS/careers/Gmail processing, and CSV synchronization. |

## Prioritized action plan

### P0 — Repair the documents API contract

**Categories:** Data consistency, UI usability

**Evidence:** `desktop/src/lib/api.ts:152` declares `DocumentListItem[]`; `desktop/src/lib/schema.ts:160-163` requires each item to expose document fields plus `usedBy` and `kinds`. `src-tauri/src/documents.rs:171-197` instead returns `{ document, usageCount }`.

**Root cause:** The serialized backend shape and TypeScript contract drifted. Type assertions at the generic Tauri boundary hide the mismatch, so compilation does not protect the page.

**Observed result:** Documents-page reads such as `doc.id`, `doc.originalFilename`, and `api.openDocument(doc.id)` resolve against the wrong object, producing missing labels/keys and an undefined document id. The advertised attachment metadata can never render because the backend returns only a count.

**Required changes:**

1. Choose one canonical list-item DTO. Recommended: return a flattened `DocumentListItem` from Rust with the document columns plus `kinds: string[]` and `usedBy: string[]` (or change the UI/schema to intentionally use `{ document, usageCount }`).
2. Implement it with a joined/aggregated query rather than per-document lookups; define deterministic ordering for arrays.
3. Add a boundary test that serializes `list_documents` and validates it against the frontend DTO shape, plus a component test covering a document with and without attachments.

**Implementation detail:**

- Add a Rust `DocumentListItem` struct in `src-tauri/src/models.rs`, deriving `Serialize`, with a flattened `Document`, `kinds: Vec<String>`, and `used_by: Vec<String>`. Make `documents::list_documents` return `Vec<DocumentListItem>` rather than `Vec<serde_json::Value>`.
- Replace the loop in `documents::list_documents` with one SQL statement that joins `documents`, `job_documents`, `jobs`, and `companies`, then groups in Rust by document id. This remains portable across the bundled SQLite versions and avoids depending on JSON aggregation. Sort documents by `imported_at DESC`, then attachments by `used_at DESC` so the UI result is stable.
- Return exactly the top-level shape declared in `desktop/src/lib/schema.ts`; do not make the frontend unwrap a backend-specific `document` object. If a count is useful, add `usageCount` to both DTOs deliberately, rather than implicitly replacing `usedBy`/`kinds`.
- Add a Rust unit test using a seeded DB with one unattached document and one document attached to two jobs under different kinds. Add a Vitest mock of `api.listDocuments` that asserts the list uses the returned id in its Open callback.

**Acceptance check:** A populated Documents page renders filename, use context, and an Open action that passes a valid id; empty state still works.

### P0 — Remove synchronous full CSV export from interactive saves

**Categories:** Performance, UI usability, data consistency

**Evidence:** `src-tauri/src/commands/mod.rs:157-177` and `461-470` invoke `schedule_export_jobs_csv` while executing inside `AppState::with_db`; `src-tauri/src/db/mod.rs:41-44` holds the app-wide mutex for that closure. Despite its name, `src-tauri/src/jobs/csv.rs:395-400` exports immediately. Export scans every tracked job (`259-299`), reads/writes synchronization state, writes the full CSV and atomically renames it (`314-386`).

**Root cause:** A full-file synchronization task is included in the latency-critical mutation transaction path and runs under the only UI SQLite mutex.

**User impact:** Save/create latency grows with job count and file-system latency. During an export, every regular UI database read/mutation queues behind the mutex, making the desktop UI feel stuck after a save. Repeated saves generate redundant complete rewrites.

**Required changes:**

1. Commit the job mutation first and return its result immediately.
2. Replace the placeholder `EXPORT_TIMER` with a real, process-wide single-flight debounced export worker. Coalesce dirty writes over a short window; run it on a dedicated SQLite connection after the mutation commits.
3. Use a file lock (or another cross-process coordination protocol) for `jobs.csv` and its sync state, because the LaunchAgent/full runner and UI can both export/import.
4. Expose export failure/last-sync state separately instead of silently logging and reporting a successful job save.
5. Add a benchmark/test showing several rapid saves yield one export and that a list command remains responsive while an export is delayed.

**Implementation detail:**

- Remove `schedule_export_jobs_csv(conn, ...)` from the `with_db` closures in `create_job_with_validator` and `update_job_cmd`. Those closures should only perform the DB transaction and construct the Tauri response.
- Replace `static EXPORT_TIMER: Mutex<Option<()>>` with state that owns a cancelable timer/task plus a `dirty` generation counter. `mark_csv_export_dirty(paths)` should reset a 250–750ms timer, then export from a new connection only after the timer settles. If another mutation arrives during export, retain the newest generation and run one follow-up export.
- Keep this logic in Rust (not React) so every mutation source can use it. Put its coordinator in `AppState` or a dedicated module and have create/update, CSV import, watch approval/dismissal, and any other pipeline-changing command call the same invalidation function after commit.
- Lock both `jobs.csv` and `jobs.csv.sync.json` for the entire read/compare/write sequence. Use the same lock from `export_jobs_csv`, `import_jobs_csv`, and the LaunchAgent flow; preserve the current temp-file plus rename write strategy.
- Persist/report an export status containing `dirty`, `lastSuccessfulAt`, and `lastError`. A future status command or event can power non-blocking UI feedback; a failed export must retry on the next change rather than incorrectly advancing sync state.
- Write tests with an injectable timer/export function: ten quick marks invoke one export, a mark during an export invokes one additional export, and an intentionally blocked export does not delay `list_jobs_cmd`.

**Acceptance check:** Saving a job does not wait for CSV I/O, rapid saves produce one final export, and a failed export is visible without rolling back the successfully saved job.

### P1 — Make multi-record and file/database mutations atomic

**Categories:** Data consistency

**Evidence:** `src-tauri/src/documents.rs:43-94` writes the file before inserting its DB row. `attach_document_to_job` inserts an attachment (`109-139`), and the command adds its event as a separate statement (`src-tauri/src/commands/mod.rs:666-685`). Job creation inserts a company/job/event/watch as separate statements (`src-tauri/src/jobs/service.rs:88-162`, `src-tauri/src/commands/mod.rs:157-176`). Watch sync and careers checks also apply several dependent updates without a transaction (`src-tauri/src/ats/sync.rs:57-253`, `src-tauri/src/ats/careers.rs:35-90`). No transactional APIs are used in the source tree.

**Root cause:** Operations that model one user-visible action are composed from independent writes. A failure or concurrent-process SQLite contention after an earlier write leaves partial state.

**User impact:** Examples include orphaned document files, an attachment without a matching timeline event, a job created without its intended watch, and partially applied watch/careers updates that appear as inconsistent dashboard data.

**Required changes:**

1. Define transactional service methods for `create job + optional watch`, `attach/import document + event`, `watch sync`, `careers snapshot + review`, and Gmail match + job event.
2. Use a SQLite transaction for all database writes in each operation. For document import, write to a uniquely named temporary file, execute the transaction, then atomically move it into place; remove the temporary/final file on DB failure. Treat a failed final move as a recoverable, surfaced consistency error.
3. Add failure-injection tests for the second and final write of each operation, asserting neither partial DB state nor orphaned file remains.

**Implementation detail:**

- Move each workflow into a service-level `*_in_tx` function that accepts `&Transaction`; Tauri commands should acquire the connection, start a transaction, call one workflow, commit, and only then perform follow-up scheduling/events. Do not expose transactions to React.
- In `create_job_with_validator`, validate/fetch outside the DB lock as today, then create/reuse company, create the job, write its `created` event, and insert the optional watch inside one transaction. Add a database uniqueness constraint for normalized `(company_id, provider, board_slug)` so the idempotency guarantee does not depend on a prior read.
- For attachment flows, verify the job and document exist, insert `job_documents`, and append `document_attached` inside the same transaction. Add a unique constraint appropriate to the product rule—usually `(job_id, document_id, kind)`—to prevent accidental duplicate attachment clicks.
- For new document bytes, write to `documents/.tmp/<uuid>` first. Insert the DB record in a transaction, commit, then rename to the content-addressed final name. If rename fails, delete/compensate the DB row in a short recovery transaction and report the failure; startup maintenance should purge old temporary files and flag rows whose file is missing.
- In watch, careers, and Gmail workflows, collect remote data before opening the write transaction. Then perform every dependent insert/update, including success/failure bookkeeping and job events, in one short transaction. This prevents a network await from holding a write lock.
- Add a test-only fault hook around each statement/file move; assert rollback leaves no company/job/watch/event/attachment residue and no final or temporary document file.

**Acceptance check:** Each mutation is all-or-nothing from the user's perspective, including its related history/audit event.

### P1 — Prevent stale async responses and destructive full-page reloads

**Categories:** Data consistency, UI usability, performance

**Evidence:** `JobsPage` loads three requests at once and unconditionally commits their result (`desktop/src/pages/JobsPage.tsx:45-68`); the route changes filters via URL (`141-152`). `JobDetailPage` unconditionally sets `loading` then fetches detail plus the full document library (`18-35`), and each successful save/check/attachment calls that full reload (`63-78`, `114-121`). The same load/unmount pattern appears in Companies, Company Detail, Documents, and Gmail pages. In contrast, `CompanyWatchAutomation` correctly uses a cancellation guard (`55-84`).

**Root cause:** Page-local fetches have no request identity, abort signal, or latest-response guard. The detail page treats every background refresh as an initial load, unmounting its editable form.

**User impact:** A slow response for an old filter/job can overwrite newer screen state. More seriously, a user can type while a job save is in flight; after the save succeeds, `onUpdated` sets parent loading, unmounts `JobDetailClient`, and discards those newer unsaved edits. Full-page spinners also create abrupt layout shifts for routine checks and attachments.

**Required changes:**

1. Introduce a small query/mutation layer (custom hook is sufficient) with monotonic request ids plus `AbortController` where the underlying operation supports it. Commit results/errors/loading only if the request is still current and the route key still matches.
2. Preserve prior data during refresh (`isRefreshing`), rather than replacing the page with a loading paragraph. Reserve full loading UI for first load only.
3. On detail save, use the mutation response to merge the persisted server state into `saved`; do not refetch/unmount the editor. Reload only affected side panels (events/attachments) and reconcile without overwriting a newer dirty draft.
4. Add tests for out-of-order filter requests, route changes while loading, and typing during a delayed save.

**Implementation detail:**

- Implement a reusable `useLatestAsync`/`useTauriQuery` hook under `desktop/src/lib/`. It should assign every request a monotonically increasing sequence, retain the latest key (for example `job:${id}` or the canonicalized filter tuple), and only call `setState` when both still match. Tauri `invoke` itself cannot necessarily be aborted, so the stale-result guard is mandatory even if an abort signal is added for fetch-backed APIs later.
- Make page state explicit: `data`, `initialLoading`, `refreshing`, and `error`. Only render the full loading screen when `data` is absent. During refresh, keep data mounted and show a local `aria-live` status or disabled action control.
- Split `JobDetailPage` loading into `loadDetail` and `loadLibrary`. Have `JobDetailClient.save()` consume `api.updateJob`'s returned detail, update its saved baseline from that response, and notify the parent to merge event/attachment data without toggling page-level initial loading. If a newer local draft exists when the response returns, update only the baseline and leave the draft intact.
- For mutation callbacks, invalidate only the data that changed: attachment actions refresh the library/attachment panel; posting checks merge posting state; job save merges its detail. Avoid the present `onUpdated={() => void load()}` full-page fan-out.
- Use deferred promise tests in Vitest: resolve the newer filter/query before the older one and assert the older response is ignored; type after `save()` begins and assert the editor remains mounted with that text.

**Acceptance check:** The last navigation/filter request wins; a refresh never erases text typed after a save started; routine refreshes retain the visible page shell.

### P1 — Guard URL metadata/detection requests against input changes

**Categories:** Data consistency, UI usability

**Evidence:** On the New Job page, `onAutofill` captures `url` and later writes preview/title/company (`desktop/src/pages/NewJobPage.tsx:43-82`), but the URL input remains editable while the request is pending (`130-152`). Company watch detection has the same mutable-input/request pattern (`desktop/src/components/CompanyWatchAutomation.tsx:86-109`, `201-226`).

**Root cause:** The in-flight flag blocks a second request but does not associate the response with the URL that initiated it.

**User impact:** If the user pastes a replacement URL before the first request returns, metadata and board confirmation from the old URL can be applied to the new URL and then saved.

**Required changes:**

1. Track a request sequence plus the normalized submitted URL; accept the result only when it matches the current URL and is the newest sequence.
2. Abort/supersede the prior request on URL change; clear the corresponding loading state.
3. Optionally disable the URL field while an explicit action runs, but do not make that the sole correctness control.
4. Add a deferred-promise component test proving a stale response cannot repopulate the form.

**Implementation detail:**

- Store `previewRequestId` in a ref and increment it whenever the URL changes or a preview begins. At invocation, capture `{ requestId, normalizedUrl }`; on resolution, require both to equal the current ref/current normalized field value before setting preview, title, company, confirmation, or user feedback.
- Clear board/careers confirmation immediately when the URL changes. If the active request belongs to the old URL, set `isAutofilling`/`busy` false only when that request remains the current one; otherwise it must not affect the newer request's spinner.
- Extract this behavior into a shared `useJobUrlPreview` hook used by `NewJobPage` and `CompanyWatchAutomation`, ensuring the two flows have identical supersession behavior and error formatting.
- Keep manual title/company values unless the product explicitly opts into replacement. The current `applyJobUrlPreview` behavior should be tested for both a blank form and manually edited fields.

**Acceptance check:** Detection state always describes the current URL shown in the field.

### P1 — Serialize scheduled/manual background mutations and make single-flight real

**Categories:** Data consistency, UI usability

**Evidence:** The runner does use a cross-process file lock (`src-tauri/src/runner.rs:50-61`), but manual `sync_watch` (`src-tauri/src/commands/mod.rs:545-558`) and `gmail_poll` (`817-820`) do not participate. The stated in-memory lock in the two runner commands is dropped immediately at the end of its block (`839-875`), so it does not provide the advertised in-process single flight. Gmail’s read-before-insert sequence (`src-tauri/src/gmail/poll.rs:79-92`, `182-237`) can race with a manual poll or runner poll; the unique key then turns a benign duplicate into a failed poll.

**Root cause:** Concurrency ownership is split across an ineffective in-memory guard, a runner-only file lock, and independent commands that mutate the same tables.

**User impact:** Concurrent manual and scheduled work can cause busy errors, duplicate-work failures, inaccurate failure counters, or partial sync application. The UI only reports a generic action failure and offers no indication that the scheduler owns the work.

**Required changes:**

1. Create keyed operation coordinators for `runner`, each `watch:{id}`, and `gmail-poll`; retain the guard for the entire future and have all entry points use the same coordinator/file lock.
2. Make writes idempotent at the database boundary (`INSERT ... ON CONFLICT DO NOTHING` where appropriate) and wrap the Gmail batch in a transaction. On a duplicate message, continue rather than failing the entire poll.
3. Return a typed "already running" result with enough status to let the UI display the active operation instead of a generic error.
4. Add integration tests that launch concurrent runner/manual-poll and manual/runner-watch paths.

**Implementation detail:**

- Replace the scoped `runner_lock.try_lock()` blocks with a guard stored across the awaited call (or, more simply, rely on one shared file-lock helper whose returned file remains alive until the operation completes). The current guards are dropped before `run_jobs_cycle`/`check_all_postings` begins.
- Establish lock identities under the data directory: one cycle lock, one Gmail lock, and one per-watch lock derived from a safe watch-id hash. `run_jobs_cycle`, `check_all_postings`, `sync_watch`, and `gmail_poll` must acquire the relevant lock in a documented, consistent order to avoid deadlocks.
- In `poll_gmail_matches`, use `INSERT ... ON CONFLICT(gmail_message_id) DO NOTHING`, inspect affected-row count, and only create the `job_events` record when the email match insert succeeded. Execute the complete write batch and checkpoint update in one transaction.
- Convert operation conflicts into a structured response/error code such as `{ code: "operation_in_progress", operation: "gmail-poll", startedAt }`. Map that in `api.ts`, and show a disabled button plus current operation text instead of an action-failed alert.
- Test with two independent SQLite connections and two concurrent tasks, including one that crosses the manual/scheduled boundary. Assert one task performs the work and the other receives the typed in-progress result without data loss.

**Acceptance check:** Only one logical sync/poll runs per key; a duplicate Gmail message cannot fail an otherwise valid cycle.

### P1 — Bound background network work and avoid linear cycle duration

**Categories:** Performance, UI usability

**Evidence:** The jobs runner checks every posting sequentially (`src-tauri/src/runner.rs:69-103`), then each watch (`139-171`) and careers page (`173-212`) sequentially. A cycle duration is therefore the sum of every remote request. The Gmail poll then fetches up to 50 messages one at a time (`src-tauri/src/gmail/poll.rs:47-180`). Progress is event-only and the main UI does not refresh when the cycle completes.

**Root cause:** Network I/O is intentionally safe with respect to the DB mutex but has no bounded concurrency or time budget.

**User impact:** A modest number of jobs/watches makes a manual run feel indefinitely busy; overlap with the hourly LaunchAgent becomes more likely. The user sees progress but must manually navigate/refresh to see most results.

**Required changes:**

1. Use bounded concurrency (for example a `buffer_unordered`/semaphore with a conservative configurable limit) for network fetches, while applying each result in a short SQLite transaction.
2. Set per-request and whole-cycle timeouts; carry structured per-item failures into the final summary.
3. Refresh/invalidate affected UI queries when the final runner event arrives, without using a full-page spinner.
4. Add a deterministic test with delayed fetch stubs to prove concurrency is bounded and faster than serial execution.

**Implementation detail:**

- Refactor `runner.rs` into a two-phase pattern for postings, watch syncs, and careers checks: snapshot eligible IDs/URLs, fetch with a shared semaphore, then apply each completed outcome using a short transaction/connection operation. Do not share a mutable `rusqlite::Connection` through concurrent fetch futures.
- Start conservatively (for example 4 concurrent posting/careers fetches, 2 concurrent ATS/Gmail message fetches) and make limits constants/configuration with rate-limit-aware backoff. Preserve result ordering in summaries by attaching each result to its original ID rather than relying on completion order.
- Wrap each remote call in `tokio::time::timeout`; classify timeouts, HTTP errors, and parse errors separately. A failed item must yield a result and progress update, not abort a healthy batch unless the failure is global (for example invalid credentials).
- Emit a final `jobs-runner-progress` event that includes a cycle id, phase, success/failure counts, and `done` state. Pages/listeners should invalidate affected queries only for the matching completed cycle; `RunJobsButton` and `JobsPage` must not treat another operation's events as their own.
- Use injectable async fetch traits/test functions to measure maximum observed parallelism and prove the set limit is never exceeded.

**Acceptance check:** Cycle wall time tracks the slowest bounded batch, failures are isolated to their item, and the UI reflects completed work automatically.

### P2 — Make list endpoints scale: pagination, smaller projections, indexes, and set-based queries

**Categories:** Performance

**Evidence:** Jobs page invokes two unpaginated job-list calls plus the complete companies graph for every filter load (`desktop/src/pages/JobsPage.tsx:45-60`). The jobs list returns all columns, filters `LIKE '%term%'`, and uses a correlated `NOT EXISTS` over `job_events` (`src-tauri/src/jobs/service.rs:232-284`); counts repeat that correlation (`554-593`). Weekly activity reads all qualifying event timestamps into Rust (`596-656`). Companies are assembled by filtering all watches/reviews once per company (`src-tauri/src/companies.rs:110-164`). Documents perform one count query per document (`src-tauri/src/documents.rs:171-197`). The schema provides no supporting indexes beyond URL/source/checksum/message (`src-tauri/src/db/migrate.rs:38-40`, `72`, `96`).

**Root cause:** Dashboard endpoints were designed for small local data and repeatedly ship/compute full collections. Several relations are traversed with N+1 or O(companies × related rows) loops.

**Required changes:**

1. Add cursor/limit pagination to jobs, documents, Gmail pending matches, and the company list; request a small `newFromWatch` preview separately with a limit of five.
2. Split dashboard summary/count/activity from list rows or return them once from one purpose-built dashboard command. Select only fields each view renders.
3. Add migration indexes for queried foreign keys and sort/filter paths, including `job_events(job_id, type)`, `job_events(occurred_at)`, `jobs(company_id, updated_at)`, `jobs(status, updated_at)`, `jobs(is_new_from_watch, updated_at)`, `job_documents(document_id)`, `job_documents(job_id)`, `company_watches(company_id)`, and pending-review/pending-email predicates. Confirm choices with `EXPLAIN QUERY PLAN` against realistic data.
4. Replace per-company/per-document loops with grouped joins/aggregates; let SQLite bucket weekly activity where practical.
5. For substring search, decide whether ordinary indexed prefix search is sufficient or add FTS5. Do not add FTS merely to index `%term%` `LIKE`, which cannot use a normal B-tree index.

**Implementation detail:**

- Extend `JobFilters` with `limit` and a stable cursor such as `(updated_at, id)`. Query with `ORDER BY updated_at DESC, id DESC` and return `{ jobs, nextCursor }`; carry the same request shape through `desktop/src/lib/api.ts` and a "load more" or virtualized list. Keep the watch preview endpoint explicitly capped at five rather than fetching all discoveries and slicing in React.
- Add a dedicated `get_jobs_dashboard` command that returns counts, seven-day activity, and the five newest watch discoveries. `list_jobs_cmd` should return only the current page. On Gmail, request enough job choices for the interaction deliberately (searchable server-side picker for large sets) rather than loading every job.
- Create migrations with `CREATE INDEX IF NOT EXISTS` and verify their order against actual predicates before adding all suggested indexes blindly. Include `EXPLAIN QUERY PLAN` regression assertions/seeds for the list, count, activity, company, documents, and pending-email queries. Remove redundant indexes if a composite one covers the same leading columns.
- Rewrite `list_companies_with_watches` as either three grouped queries indexed by company id or one join folded into a map; rewrite document usage as a grouped query over `job_documents`. Aggregate weekly activity with SQLite date/group expressions where timezone semantics are explicitly specified and tested.
- Choose search semantics first: use an escaped case-insensitive prefix field/index for simple type-ahead, or an FTS5 virtual table with synchronized insert/update/delete triggers for arbitrary token search. Keep query text debounced only after introducing search-on-change; the current submit-only filter does not need debounce.

**Acceptance check:** List rendering remains responsive with thousands of jobs/events/documents, page payloads are bounded, and query-plan tests use the intended indexes.

## Recommended implementation order

1. Fix the documents DTO (P0) and cover it with a contract test.
2. Decouple/debounce CSV export (P0), then introduce transaction boundaries for mutations (P1).
3. Add request coordination and non-destructive detail refreshes; cover stale-request and in-flight edit cases (P1).
4. Unify background-operation locking, then add bounded network concurrency (P1).
5. Add pagination, projections, set-based queries, and indexes after measuring representative data (P2).

## Audit notes

- Existing uncommitted work in `src-tauri/src/jobs/service.rs` and untracked `output/`/`tmp/` was not altered.
- No application code was changed by this audit; this file is the requested approval gate.
