# Requirements Document

## Introduction

Job Tracker provides a full jobs cycle through **Run jobs** and a posting-only cycle through **Check postings**. The current desktop interface reports aggregate progress but does not identify the posting under evaluation, distinguish every lifecycle state, retain enough failure evidence, or make classification confidence clear. The current classifier can also treat a successful HTTP response as proof that a posting is active even when the response is a generic careers page, redirect target, consent page, or other non-posting content.

This feature introduces a consistent run model for the Tauri 2 desktop application and Rust backend. The model makes run and per-posting state visible, supports safe cancellation and targeted retry, applies conservative evidence-based posting classification, and preserves the existing Tauri commands, CLI automation, data-directory behavior, runner exclusivity, and CSV synchronization behavior.

## Glossary

- **Job_Tracker**: The Tauri 2 desktop application comprising the React/Vite user interface in `desktop/` and the Rust backend in `src-tauri/`.
- **Run**: One accepted execution request tracked from creation to a Terminal_State.
- **Run_Identifier**: An opaque value that uniquely identifies one Run.
- **Run_Type**: Either Jobs_Cycle or Posting_Check_Run.
- **Jobs_Cycle**: A Run that performs posting checks, ATS watch synchronization, careers-page checks, and CSV synchronization.
- **Posting_Check_Run**: A Run that performs posting checks without the later Jobs_Cycle stages.
- **Run_Status**: Exactly one of Queued, Active, Canceling, Canceled, Completed, Completed_With_Errors, or Error.
- **Terminal_State**: One of Canceled, Completed, Completed_With_Errors, or Error.
- **Run_Coordinator**: The Rust backend component that owns Run lifecycle, runner exclusivity, work scheduling, cancellation, retry selection, result persistence, and progress publication.
- **Run_Monitor**: The React/Vite desktop interface that starts a Run and presents Run progress, results, cancellation, and retry controls.
- **Progress_Contract**: The typed Tauri event and command-response data shared by the Run_Coordinator and Run_Monitor.
- **Run_Summary**: The terminal record containing Run_Type, timestamps, duration, aggregate counts, stage outcomes, and links to per-posting results.
- **Posting_Check**: One attempt within a Run to determine the availability of one Posting.
- **Posting**: A tracked job opportunity identified by a Job_Identifier and Posting_URL.
- **Job_Identifier**: The existing stable identifier of a tracked job.
- **Posting_URL**: The configured URL checked for a Posting.
- **Job_Identity**: The Job_Identifier, job title, company name, and Posting_URL used to identify a Posting in progress and results.
- **Posting_Status**: Exactly one of Queued, Active, Completed, Error, or Canceled for a Posting_Check.
- **Posting_State**: Exactly one of Active, Closed, or Unknown as presented to a user; Closed corresponds to the existing persisted value `inactive`, while Active and Unknown correspond to `active` and `unknown`.
- **Classification_Engine**: The Rust backend logic that maps Check_Evidence to a Posting_State and Classification_Reason.
- **Check_Evidence**: Structured facts collected by a Posting_Check, including attempt time, requested URL, final URL, HTTP status when available, provider signal when available, matched content signal when available, and failure category when applicable.
- **Authoritative_Result**: The finalized Posting_Check result that determines the Posting_State, Posting_Status, and counters.
- **Supplementary_Check_Evidence**: Non-authoritative Check_Evidence retained for audit after a Posting_Check has finalized an Authoritative_Result; Supplementary_Check_Evidence cannot change the Authoritative_Result, Posting_State, Posting_Status, or counters.
- **Classification_Reason**: A human-readable explanation that identifies the evidence and rule responsible for a Posting_State.
- **Positive_Active_Evidence**: A successful provider response that lists the stable posting identifier as open, or a successful posting page that contains matching job identity plus an enabled application affordance.
- **Conclusive_Closed_Evidence**: An HTTP 404 or 410 response from the posting destination, an explicit provider response identifying the posting as closed or absent from a successfully retrieved authoritative listing, or recognized closure content on the posting page.
- **Application_Affordance**: A page control or link that explicitly starts an application for the Posting and is not disabled.
- **Transient_Failure**: A timeout, connection failure, HTTP 429 response, HTTP 5xx response, or temporary provider failure that does not establish Posting availability.
- **Retry_Run**: A new Posting_Check_Run containing selected Posting_Check results from an earlier Run.
- **Cancellation_Request**: A user request to stop a non-terminal Run from starting additional work.
- **Existing_Command_Surface**: The existing `run_jobs_cycle_cmd`, `check_all_postings_cmd`, `check_job_posting`, `jt sync`, and `--run-jobs` entry points and their established automation behavior.
- **CSV_Mirror**: The existing jobs CSV file synchronized by Job Tracker.
- **Data_Directory**: The existing database and file location resolved by explicit override, debug repository rules, or packaged application rules.

## Requirements

### Requirement 1: Canonical Run Lifecycle

**User Story:** As a job seeker, I want each execution to expose an unambiguous lifecycle, so that I can tell whether work is waiting, running, finished, partially failed, canceled, or unable to run.

#### Acceptance Criteria

1. WHEN Job_Tracker accepts a Run request, THE Run_Coordinator SHALL assign a non-empty Run_Identifier that is unique among recorded Runs and assign the Run_Status Queued.
2. WHEN a Run is accepted, THE Run_Coordinator SHALL record the Run start time before starting the first unit of work.
3. WHEN the first unit of work starts, THE Run_Coordinator SHALL change the Run_Status from Queued to Active.
4. THE Run_Coordinator SHALL treat Canceled, Completed, Completed_With_Errors, and Error as terminal Run_Status values and Queued, Active, and Canceling as non-terminal Run_Status values.
5. WHEN every scheduled unit finishes successfully and no accepted Cancellation_Request is pending, THE Run_Coordinator SHALL assign the Run_Status Completed.
6. WHEN every scheduled unit reaches a terminal status, at least one unit has an Error status, and no accepted Cancellation_Request is pending, THE Run_Coordinator SHALL assign the Run_Status Completed_With_Errors.
7. IF a Run cannot begin or continue because of a run-level failure, THEN THE Run_Coordinator SHALL assign the Run_Status Error, record a non-empty run-level reason, and preserve every unit result completed before the failure.
8. WHEN a Run reaches a Terminal_State, THE Run_Coordinator SHALL record the finish time and calculate the duration from the recorded start time to the finish time.
9. WHEN the Run_Status changes, THE Run_Coordinator SHALL publish the previous Run_Status, new Run_Status, Run_Identifier, and Run_Type through the Progress_Contract.

### Requirement 2: Visible Run State and Stage Progress

**User Story:** As a job seeker, I want persistent and detailed run feedback, so that I understand what Job Tracker is doing without relying on a temporary message.

#### Acceptance Criteria

1. WHILE a Run is non-terminal, THE Run_Monitor SHALL display the Run_Identifier, Run_Type, Run_Status, current stage, completed count, total count, and elapsed duration.
2. WHERE the Run_Type is Jobs_Cycle, THE Run_Monitor SHALL display a stage outcome of not-started, in-progress, succeeded, failed, or skipped for posting checks, ATS watch synchronization, careers-page checks, and CSV synchronization.
3. WHEN a Run enters a Terminal_State, THE Run_Monitor SHALL display the Run_Summary until the user dismisses the Run_Summary or a newer accepted Run becomes the displayed Run.
4. WHEN the user navigates between desktop views during a non-terminal Run, THE Run_Monitor SHALL preserve the displayed Run_Identifier and latest valid progress.
5. WHEN the desktop application starts while a Run is non-terminal, THE Run_Monitor SHALL restore the current Run_Status and latest recorded progress within 2 seconds after the initial Tauri connection is available.
6. WHEN the Run_Monitor receives a progress update for a Run_Identifier or Run_Type other than the displayed Run, THE Run_Monitor SHALL retain the displayed Run state without applying the unrelated update.
7. IF progress updates are missed, THEN THE Run_Monitor SHALL reconcile the displayed state from the Run_Coordinator within 2 seconds after the Tauri connection becomes available.
8. IF progress data is unavailable or invalid, THEN THE Run_Monitor SHALL retain the last valid displayed Run state and display a transient, non-blocking retrieval or validation notice beside the retained state without requiring acknowledgment.
9. WHEN Job_Tracker accepts a newer Run, THE Run_Monitor SHALL allow the newer Run to become the displayed Run.

### Requirement 3: Per-Posting Progress and Identity

**User Story:** As a job seeker, I want to see which postings are queued, being checked, and finished, so that I can understand the scope and movement of a posting-check run.

#### Acceptance Criteria

1. WHEN a Posting_Check is added to a Run, THE Run_Coordinator SHALL publish the Job_Identity with a Posting_Status of Queued in the first Progress_Contract update that includes the Posting_Check.
2. WHEN a Posting_Check starts network evaluation, THE Run_Coordinator SHALL publish the Job_Identity with a Posting_Status of Active in the first Progress_Contract update after the transition.
3. WHEN a Posting_Check finishes with a Posting_State, THE Run_Coordinator SHALL publish the Job_Identity with a Posting_Status of Completed in the first Progress_Contract update after the transition.
4. IF a Posting_Check cannot produce and persist a Posting_State, THEN THE Run_Coordinator SHALL publish the Job_Identity with a Posting_Status of Error and a non-empty failure reason within the Progress_Contract message bound.
5. WHEN a Posting_Check publishes a Posting_Status of Error, THE Run_Coordinator SHALL omit a Completed transition for the same Posting_Check attempt.
6. WHEN a queued Posting_Check is omitted because of cancellation, THE Run_Coordinator SHALL publish the Job_Identity with a Posting_Status of Canceled in the first Progress_Contract update after the transition.
7. WHILE posting checks are in progress, THE Run_Monitor SHALL list every Job_Identity in the Run with the latest Posting_Status and counters for Queued, Active, Completed, Error, and Canceled.
8. WHEN a Posting_Check reaches Completed, THE Run_Monitor SHALL display the resulting Posting_State and non-empty Classification_Reason for the Job_Identity.
9. THE Run_Coordinator SHALL calculate the total posting count before the first Posting_Check enters Active status.
10. THE Run_Coordinator SHALL associate each Job_Identifier with one stable Job_Identity within a Run.
11. THE Run_Coordinator SHALL maintain Posting_Status counters whose sum equals the total posting count in every progress snapshot.
12. WHEN a Run reaches a Terminal_State, THE Run_Coordinator SHALL report zero Queued and zero Active Posting_Check entries.

### Requirement 4: Concurrency and Runner Exclusivity

**User Story:** As an operator, I want posting checks to run concurrently without overlapping runner cycles or checking the same posting twice, so that execution remains efficient and deterministic.

#### Acceptance Criteria

1. WHILE a Run performs posting checks, THE Run_Coordinator SHALL execute no more than 4 Posting_Check network evaluations concurrently.
2. WHILE a Posting_Check for a Job_Identifier is Active within a Run, THE Run_Coordinator SHALL keep every other Posting_Check for the same Job_Identifier non-active within that Run.
3. WHILE a Jobs_Cycle or Posting_Check_Run owns the existing runner lock, THE Run_Coordinator SHALL reject every overlapping Jobs_Cycle or Posting_Check_Run with the existing `operation_in_progress:runner` error.
4. IF a Run request is rejected because another process owns the existing runner lock, THEN THE Run_Coordinator SHALL start no work and mutate no Run or posting data for the rejected request.
5. IF a Run request is rejected because another process owns the existing runner lock, THEN THE Run_Monitor SHALL display that another run is in progress without representing the rejected request as an accepted Run.
6. WHEN concurrent Posting_Check results finish in any order, THE Run_Coordinator SHALL associate each result with the correct Job_Identifier.
7. WHEN concurrent Posting_Check results finish in any order, THE Run_Coordinator SHALL produce the same per-posting persisted outcomes as a sequential execution over the same Check_Evidence.
8. WHILE the existing runner lock is held, THE Run_Coordinator SHALL recognize exactly one Run as the lock owner.
9. WHEN the lock-owning Run reaches any Terminal_State, THE Run_Coordinator SHALL release the existing runner lock.

### Requirement 5: Cancellation and Retry

**User Story:** As a job seeker, I want to stop a long run and retry only inconclusive or failed checks, so that I can recover without repeating successful work.

#### Acceptance Criteria

1. WHERE a Run has a Run_Status of Queued or Active, THE Run_Monitor SHALL offer an enabled cancellation control.
2. WHERE a Run has a Run_Status other than Queued or Active, THE Run_Monitor SHALL present the cancellation control as disabled.
3. WHEN the prior Run_Status is Queued or Active and the Run_Coordinator accepts a Cancellation_Request, THE Run_Coordinator SHALL change the Run_Status to Canceling within 1 second.
4. WHILE a Run has a Run_Status of Canceling, THE Run_Coordinator SHALL start zero additional queued units of work.
5. WHEN every unit active at the time of an accepted Cancellation_Request reaches Completed or Error, THE Run_Coordinator SHALL assign Canceled to every remaining queued Posting_Check.
6. WHEN no Active or Queued Posting_Check remains after an accepted Cancellation_Request, THE Run_Coordinator SHALL have a current Run_Status of Canceled.
7. WHEN a Run reaches Canceled, THE Run_Coordinator SHALL preserve every result completed before cancellation.
8. WHERE a Run contains a Posting_Check with Posting_State Unknown, Posting_Status Error, or Posting_Status Canceled, THE Run_Monitor SHALL offer that Posting_Check for retry selection.
9. WHEN the Run_Coordinator accepts a retry request, THE Run_Coordinator SHALL create a Retry_Run with a distinct Run_Identifier containing each selected eligible Job_Identifier exactly once.
10. WHEN a Retry_Run starts, THE Run_Coordinator SHALL retain the earlier Run and Check_Evidence without modification.
11. IF a Cancellation_Request targets a Run that is not Queued or Active, THEN THE Run_Coordinator SHALL retain the Run state without modification and return a non-empty cancellation error.
12. IF a retry request contains an ineligible or unknown Posting_Check, THEN THE Run_Coordinator SHALL create no Retry_Run, retain prior Run data without modification, and return a non-empty retry error.

### Requirement 6: Conservative Posting Classification

**User Story:** As a job seeker, I want posting availability to be classified from conclusive evidence, so that generic or blocked pages are not reported as active jobs and temporary failures are not reported as closures.

#### Acceptance Criteria

1. WHEN a successful source response identifies the Posting by the stable posting identifier, THE Classification_Engine SHALL recognize Positive_Active_Evidence.
2. WHEN a successful posting page contains a normalized company-name match, a normalized job-title match, and an enabled Application_Affordance associated with the Posting, THE Classification_Engine SHALL recognize Positive_Active_Evidence.
3. WHEN the posting destination returns HTTP 404 or 410, THE Classification_Engine SHALL recognize Conclusive_Closed_Evidence.
4. WHEN a successfully retrieved authoritative provider listing identifies the Posting as closed or omits the stable posting identifier from open postings, THE Classification_Engine SHALL recognize Conclusive_Closed_Evidence.
5. WHEN a posting page contains recognized closure content matched to the Posting, THE Classification_Engine SHALL recognize Conclusive_Closed_Evidence.
6. WHEN Check_Evidence contains Positive_Active_Evidence, contains no Conclusive_Closed_Evidence, and contains no transient, blocked-access, consent, authentication, anti-bot, or access-denied signal, THE Classification_Engine SHALL classify the Posting_State as Active.
7. WHEN Check_Evidence contains Conclusive_Closed_Evidence, contains no Positive_Active_Evidence, and contains no transient, blocked-access, consent, authentication, anti-bot, or access-denied signal, THE Classification_Engine SHALL classify the Posting_State as Closed.
8. IF Check_Evidence contains both Positive_Active_Evidence and Conclusive_Closed_Evidence, THEN THE Classification_Engine SHALL classify the Posting_State as Unknown and identify the conflicting evidence categories in the Classification_Reason.
9. IF Check_Evidence contains neither Positive_Active_Evidence nor Conclusive_Closed_Evidence, THEN THE Classification_Engine SHALL classify the Posting_State as Unknown.
10. IF a Posting_Check ends with a Transient_Failure, THEN THE Classification_Engine SHALL classify the Posting_State as Unknown and identify the Transient_Failure category in the Classification_Reason.
11. IF a Posting_Check receives an HTTP 401, 403, 429, or 5xx response, THEN THE Classification_Engine SHALL classify the Posting_State as Unknown.
12. WHEN a Posting_URL redirects, THE Classification_Engine SHALL classify the final response and final URL while retaining the requested URL, final URL, and redirect status evidence in Check_Evidence.
13. IF a redirect ends on a generic careers page without matching job identity and an enabled Application_Affordance associated with the Posting, THEN THE Classification_Engine SHALL classify the Posting_State as Unknown.
14. IF a successful HTTP response presents a consent page, authentication page, anti-bot challenge, or access-denied page, THEN THE Classification_Engine SHALL classify the Posting_State as Unknown.
15. WHEN the Classification_Engine normalizes equivalent Check_Evidence repeatedly, THE Classification_Engine SHALL produce equivalent normalized evidence, Posting_State, and Classification_Reason values.
16. WHEN the Classification_Engine assigns a Posting_State, THE Classification_Engine SHALL produce a non-empty Classification_Reason within the Progress_Contract message bound that names the decisive evidence or failure categories.
17. WHEN a Posting_State changes, THE Run_Coordinator SHALL change only posting-availability fields and preserve pipeline status and unrelated user-maintained fields.

### Requirement 7: Evidence and Failure Transparency

**User Story:** As a job seeker, I want to know why each posting received a classification, so that I can judge uncertain results and troubleshoot failures.

#### Acceptance Criteria

1. WHEN a Posting_Check completes, THE Run_Coordinator SHALL persist structured Check_Evidence and a non-empty Classification_Reason with the Posting_Check result.
2. WHERE an HTTP response is available, THE Check_Evidence SHALL include the HTTP status and final URL.
3. WHERE a provider signal contributes to the Posting_State, THE Check_Evidence SHALL identify the provider and signal category without storing provider credentials.
4. WHERE a content signal contributes to the Posting_State, THE Check_Evidence SHALL identify the matched content category without storing response-body text or excerpts.
5. IF a Posting_Check encounters a timeout, connection failure, blocked destination, redirect failure, provider failure, internal processing failure, or persistence failure, THEN THE Check_Evidence SHALL identify the corresponding failure category.
6. WHEN the Run_Monitor displays a Posting_Check result, THE Run_Monitor SHALL provide an evidence control that reveals the associated Classification_Reason and Check_Evidence upon activation.
7. WHEN the latest Posting_Check produces Unknown, THE Run_Coordinator SHALL retain the preceding conclusive Posting_State in posting-check history.
8. WHEN a Posting_State changes between consecutive Posting_Check results, THE Run_Coordinator SHALL add exactly one job-history entry containing the previous state, new state, Classification_Reason, and change time.
9. THE Run_Coordinator SHALL exclude response bodies, response-body excerpts, authentication values, cookies, session values, and URL credential or session values from Progress_Contract events and persisted Check_Evidence.
10. IF persistence of Check_Evidence or the Posting_Check result fails, THEN THE Run_Coordinator SHALL publish Posting_Status Error and persist no partial Posting_Check result for the failed attempt.

### Requirement 8: Timeouts and Result Accounting

**User Story:** As a job seeker, I want every scheduled posting to reach an explained outcome, so that a stalled endpoint cannot leave a run indefinitely incomplete.

#### Acceptance Criteria

1. WHILE a Posting_Check waits for one network evaluation, THE Run_Coordinator SHALL end the evaluation no later than 30 seconds after the evaluation starts.
2. IF a network evaluation exceeds 30 seconds, THEN THE Run_Coordinator SHALL record timeout Check_Evidence, classify the Posting_State as Unknown, and assign the Posting_Status Completed.
3. THE Run_Coordinator SHALL assign every Posting_Check to exactly one of Queued, Active, Completed, Error, or Canceled in every progress snapshot.
4. THE Run_Coordinator SHALL maintain counts satisfying total equals Queued plus Active plus Completed plus Error plus Canceled in every progress snapshot.
5. WHEN a Run reaches a Terminal_State, THE Run_Coordinator SHALL report zero Posting_Check entries with a Posting_Status of Queued or Active.
6. WHEN a Posting_Check reaches a terminal Posting_Status, THE Run_Coordinator SHALL adjust the Posting_Status counters exactly once for that transition.
7. WHEN a Posting_Check result is persisted, THE Run_Coordinator SHALL set the posting last-checked time equal to the completed attempt time.
8. IF a network result arrives after the Posting_Check has timed out, THEN THE Run_Coordinator SHALL store the late network result as Supplementary_Check_Evidence for audit.
9. WHEN the Run_Coordinator stores a late network result as Supplementary_Check_Evidence, THE Run_Coordinator SHALL retain the timeout Authoritative_Result, Posting_State Unknown, Posting_Status Completed, and counters without modification.

### Requirement 9: Completion Summary and Desktop Refresh

**User Story:** As a job seeker, I want a useful completion summary and refreshed job data, so that I can immediately see what changed and what needs attention.

#### Acceptance Criteria

1. WHEN a Posting_Check_Run reaches a Terminal_State, THE Run_Summary SHALL report Active, Closed, Unknown, Error, and Canceled counts including zero-valued counts.
2. WHEN a Posting_Check_Run reaches a Terminal_State, THE Run_Summary SHALL include each Job_Identity from the Run exactly once.
3. WHEN a Jobs_Cycle reaches a Terminal_State, THE Run_Summary SHALL report the posting-check counts and a terminal outcome for every Jobs_Cycle stage that started.
4. WHEN a Jobs_Cycle reaches a Terminal_State, THE Run_Summary SHALL represent each unstarted stage as not-started or skipped consistently with the stage outcomes in Requirement 2.
5. WHEN a Run reaches Completed or Completed_With_Errors, THE Run_Monitor SHALL refresh affected visible desktop job data within 5 seconds after receiving the terminal transition.
6. WHEN a Run reaches Canceled, THE Run_Monitor SHALL refresh affected visible desktop job data for results completed before cancellation within 5 seconds after receiving the terminal transition.
7. WHERE a Run contains Unknown or Error results, THE Run_Monitor SHALL provide a filtered result view containing exactly the affected Job_Identity entries.
8. WHEN a Run reaches a Terminal_State, THE Run_Summary SHALL report the number of distinct Posting_State changes measured from the Posting_State values at Run start, including zero.
9. IF desktop job-data refresh fails, THEN THE Run_Monitor SHALL preserve the previously displayed job data and surface a refresh error.

### Requirement 10: Tauri and CLI Compatibility

**User Story:** As an operator or automation author, I want the enhanced visibility feature to preserve existing desktop and CLI behavior, so that current workflows continue to function.

#### Acceptance Criteria

1. THE Run_Monitor SHALL communicate with the Rust Run_Coordinator only through Tauri commands and Tauri events.
2. THE Existing_Command_Surface SHALL retain the exact existing command names.
3. WHEN `run_jobs_cycle_cmd` completes successfully, THE Existing_Command_Surface SHALL return the existing postings, watches, careers, and CSV summary fields with their existing meanings.
4. WHEN `check_all_postings_cmd` completes successfully, THE Existing_Command_Surface SHALL return the existing postings count field with its existing meaning.
5. WHEN `check_job_posting` completes successfully, THE Existing_Command_Surface SHALL return posting state, last-check result, and last-checked time fields with their existing meanings.
6. WHEN `jt sync --json` or `--run-jobs` executes, THE Existing_Command_Surface SHALL run the Jobs_Cycle without requiring the desktop user interface.
7. WHEN a CLI command receives `--json`, THE Existing_Command_Surface SHALL write exactly one valid machine-readable JSON payload to standard output.
8. WHEN a CLI command receives `--quiet`, THE Existing_Command_Surface SHALL suppress human-oriented progress output while retaining the requested result payload.
9. THE Run_Coordinator SHALL resolve the Data_Directory by applying an explicit `--data-dir` value or `JOB_TRACKER_DATA_DIR` value first, a debug repository data directory second, and the packaged application data directory third.
10. WHEN a Run mutates posting-check data, THE Run_Coordinator SHALL preserve the existing CSV_Mirror synchronization behavior.
11. THE external automation surface for this feature SHALL remain the CLI.
12. THE desktop execution surface for this feature SHALL remain the Tauri invoke and event channels.
13. IF the Existing_Command_Surface rejects an invocation before accepting a Run, THEN THE Existing_Command_Surface SHALL return a non-success result containing a structured machine-readable error code or category.
14. IF the Existing_Command_Surface rejects an invocation before accepting a Run, THEN THE Existing_Command_Surface SHALL claim no successful Run.
15. IF the Existing_Command_Surface rejects an invocation before accepting a Run, THEN THE Existing_Command_Surface SHALL perform no partial mutation for the rejected invocation.

### Requirement 11: Progress Contract Consistency

**User Story:** As a frontend developer, I want one typed progress contract for both run controls, so that Run jobs and Check postings cannot misinterpret or consume each other's events.

#### Acceptance Criteria

1. THE Progress_Contract SHALL represent Run_Identifier, Run_Type, Run_Status, stage, message, current count, total count, completion indicator, and per-posting progress when applicable.
2. THE Progress_Contract SHALL define and enforce finite encoded-length bounds for string identifiers, messages, reasons, and evidence-category values.
3. THE Progress_Contract SHALL represent current and total as non-negative integers no greater than 9,007,199,254,740,991 with current less than or equal to total.
4. WHEN the Run_Coordinator publishes a Progress_Contract event, THE Progress_Contract SHALL use the same field names, optionality, data types, and value meanings consumed by the Run_Monitor.
5. WHEN a Progress_Contract value is serialized by the Rust backend and deserialized by the React/Vite frontend, THE Progress_Contract SHALL preserve every defined field value, optional field absence, and data type.
6. WHERE an existing consumer reads stage, message, current, total, or done fields, THE Progress_Contract SHALL retain those field names and existing meanings.
7. THE Progress_Contract SHALL include a version value for compatibility checks.
8. IF the Run_Monitor receives an unsupported Progress_Contract version, THEN THE Run_Monitor SHALL retain the last valid displayed state and surface a compatibility error.
9. IF the Run_Monitor receives a malformed Progress_Contract event, THEN THE Run_Monitor SHALL retain the last valid displayed state and surface a validation error.
10. WHEN the Run_Monitor consumes a Progress_Contract event, THE Run_Monitor SHALL apply the event only when the Run_Identifier and Run_Type match the displayed Run.
11. WHEN the Run_Monitor successfully retrieves the current Run_Summary after a compatibility, validation, or missed-update error, THE Run_Monitor SHALL replace the prior displayed state with the retrieved Run_Summary as the new last valid displayed state.
12. IF retrieval of the current Run_Summary fails after a compatibility, validation, or missed-update error, THEN THE Run_Monitor SHALL retain the prior last valid displayed state and surface a retrieval error.

### Requirement 12: Accessible and Non-Blocking Interaction

**User Story:** As a job seeker using assistive technology or navigating elsewhere in the application, I want progress controls to remain understandable and non-blocking, so that I can continue using Job Tracker during a run.

#### Acceptance Criteria

1. WHILE a Run is non-terminal, THE Run_Monitor SHALL allow navigation to every existing desktop view without changing the Run lifecycle or results.
2. WHILE a Run is non-terminal, THE Run_Monitor SHALL disable duplicate submission from the corresponding run control both visually and through the control's semantic disabled state.
3. WHEN Run_Status or aggregate progress changes, THE Run_Monitor SHALL announce the change through a polite live region.
4. WHEN a Posting_Check enters Error or a Run enters Error, THE Run_Monitor SHALL announce that transition once through an assertive live region without moving keyboard focus.
5. THE Run_Monitor SHALL provide keyboard-reachable, keyboard-activatable, and programmatically named start, cancel, retry, expand-evidence, and dismiss controls.
6. THE Run_Monitor SHALL present Run_Status and Posting_Status with text in addition to color and animation.
7. WHILE the operating system reduced-motion preference is enabled, THE Run_Monitor SHALL replace progress animation with a static textual or graphical equivalent.
8. WHEN progress content updates, THE Run_Monitor SHALL preserve the user's current keyboard focus.

### Requirement 13: Accuracy and Lifecycle Verification

**User Story:** As a maintainer, I want repeatable verification of classification and progress invariants, so that UI improvements do not conceal incorrect posting results or broken run accounting.

#### Acceptance Criteria

1. WHEN representative Active, Closed, and Unknown fixtures for generic HTML postings, Greenhouse, Lever, and Ashby are evaluated, THE Classification_Engine SHALL produce the expected Posting_State and Classification_Reason category for every fixture.
2. WHEN HTTP 404, HTTP 410, HTTP 401, HTTP 403, HTTP 429, HTTP 5xx, timeout, redirect-to-generic-careers, consent-page, authentication-page, anti-bot, access-denied, explicit-closure-copy, posting-identifier, normalized-company-and-title, and enabled-application examples are evaluated, THE Classification_Engine SHALL produce the outcomes defined in Requirement 6.
3. WHEN letter case, surrounding whitespace, or equivalent normalization variants change in recognized posting identity or closure content, THE Classification_Engine SHALL preserve the resulting evidence category and Posting_State.
4. WHEN Posting_Check completion-order permutations are evaluated, THE Run_Coordinator SHALL preserve result identity and the accounting invariants defined in Requirements 3 and 8.
5. WHEN cancellation occurs at any Posting_Check completion boundary, THE Run_Coordinator SHALL allow every Posting_Check that was Active when cancellation occurred to finish, preserve the completed results from those Posting_Check entries and every earlier completed result, start zero queued Posting_Check entries, transition through Canceling to Canceled, and finish with zero Queued or Active Posting_Check entries.
6. WHEN selected Unknown, Error, or Canceled entries are retried, THE Run_Coordinator SHALL schedule exactly the selected unique eligible Job_Identifier values.
7. WHEN overlapping Run requests contend for the existing runner lock, THE Existing_Command_Surface SHALL accept exactly one Run and return `operation_in_progress:runner` for every overlapping request.
8. WHEN the Progress_Contract crosses the Rust-to-frontend boundary, THE Progress_Contract SHALL preserve every contract field, optional value, Run_Identifier, Run_Type, lifecycle state, counter, Job_Identity, and evidence metadata.
9. WHEN existing CLI JSON compatibility tests execute, THE Existing_Command_Surface SHALL retain every applicable legacy field and field meaning defined in Requirement 10.
