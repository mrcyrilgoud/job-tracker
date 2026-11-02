# Requirements Document

## Introduction

Job Tracker stores two optional per-job scores today: Appeal (1–5, 5 = most appealing) and Application_Ease (1–5, 5 = easiest to apply). This feature adds a third optional per-job score, Company_Stability, that records how stable the user judges the hiring company to be for that posting (1 = least stable, 5 = most stable). The score is stored on each job, not on the company, so that it follows the same model as Appeal and Application_Ease and can differ between postings (for example, a stable parent company and a risky new division).

Company_Stability follows the existing score conventions end to end: a nullable `company_stability` column on `jobs` with a 1–5 check, the `companyStability` JSON field, the Tauri `update_job` IPC partial-update semantics, `jt update --company-stability` / `--clear-company-stability`, the Jobs CSV mirror, the score select controls in the desktop UI, the Jobs list sort fields, and the Companies sidebar sort options. It also adds a minimum-score filter to the Jobs list and to `jt list`.

Out of scope: an in-app LLM or automatic scoring, any HTTP API, MCP server, or Node/Next server, changes to favorite behavior, and adding new filters for Appeal or Application_Ease.

## Glossary

- **Job_Tracker**: The Tauri 2 desktop application as a whole, including the Rust backend in `src-tauri/`, the desktop UI in `desktop/`, and the `jt` CLI.
- **Database**: The SQLite database resolved by `src-tauri/src/db/paths.rs`.
- **Migrator**: The schema creation and migration code in `src-tauri/src/db/migrate.rs`.
- **Job_Service**: The Rust job services in `src-tauri/src/jobs/service.rs` that read, list, sort, filter, page, and update jobs, shared by Tauri IPC and the CLI.
- **IPC_Layer**: The Tauri commands in `src-tauri/src/commands/` that the desktop UI calls through `desktop/src/lib/api.ts`.
- **CLI**: The `jt` / `job-tracker` command-line interface implemented in `src-tauri/src/cli/`.
- **CSV_Mirror**: The Jobs CSV export and import code in `src-tauri/src/jobs/csv.rs`, including the sync sidecar state.
- **Desktop_UI**: The Vite + React UI in `desktop/`.
- **Jobs_List**: The list view of the Jobs page (`desktop/src/pages/JobsPage.tsx`), including its sort control, filter panel, and per-row score controls.
- **Jobs_Board**: The board view of the Jobs page (`desktop/src/components/JobsBoardView.tsx`).
- **Job_Detail_Form**: The job edit form in `desktop/src/components/JobDetailClient.tsx`.
- **Companies_Sidebar**: The Companies sidebar on the Jobs page, whose sort and filter logic lives in `desktop/src/lib/company-sidebar.ts` and whose aggregates come from `src-tauri/src/companies.rs`.
- **Appeal**: The existing nullable per-job score from 1 (least appealing) to 5 (most appealing).
- **Application_Ease**: The existing nullable per-job score from 1 (hardest to apply) to 5 (easiest to apply).
- **Company_Stability**: The new nullable per-job score from 1 (least stable) to 5 (most stable). Stored in the `company_stability` column, exposed in JSON as `companyStability`, and labeled "Company stability (1–5, 5=most stable)" in the Desktop_UI.
- **Valid_Stability_Score**: An integer from 1 to 5 inclusive.
- **Unscored_Job**: A job whose Company_Stability is null.
- **Stability_Range_Message**: The error text "Company stability must be an integer from 1 to 5 (5 = most stable)".
- **Stability_Sort_Field**: The Jobs_List sort field with wire value `companyStability`.
- **Minimum_Stability_Filter**: A Jobs_List and CLI filter holding an optional Valid_Stability_Score. When set, a job matches only when the job's Company_Stability is greater than or equal to the filter value.
- **Pipeline_Job**: A job linked to a company whose `status` is not `archived`, whose `is_new_from_watch` is 0, and whose `watch_disposition` is not `dismissed`, as defined in the companies-tab-sorting spec.
- **Company_Stability_Average**: For one company, the arithmetic mean of Company_Stability across the company's Pipeline_Jobs whose Company_Stability is a Valid_Stability_Score, or null when there are none.
- **Stability_Company_Sort**: The Companies_Sidebar sort option with value `mostStable` and label "Most stable company".
- **Sort_Annotation**: The short value shown next to each company name in the Companies_Sidebar that explains the active sort option.

## Requirements

### Requirement 1: Store the Company_Stability score

**User Story:** As a job seeker, I want each stored job to hold an optional company stability score, so that I can record how stable I judge the hiring company to be for that posting.

#### Acceptance Criteria

1. WHEN the Migrator creates a new Database, THE Migrator SHALL create the `jobs` table with a nullable integer `company_stability` column that the Database constrains to null or a Valid_Stability_Score.
2. WHEN the Migrator runs against an existing Database whose `jobs` table lacks a `company_stability` column, THE Migrator SHALL add a nullable integer `company_stability` column constrained to null or a Valid_Stability_Score.
3. WHEN the Migrator adds the `company_stability` column to an existing Database, THE Migrator SHALL set Company_Stability to null for every existing job and SHALL keep every existing job's other column values, including Appeal and Application_Ease, unchanged, so that the job count and all other values read back after migration equal those read before migration.
4. WHEN the Migrator runs a second or later time against a Database that already has a `company_stability` column, THE Migrator SHALL complete without error, SHALL leave exactly one `company_stability` column in the `jobs` table, and SHALL keep every stored Company_Stability value unchanged.
5. IF a write attempts to store a `company_stability` value that is not null and not a Valid_Stability_Score, including 0, 6, a negative integer, a non-integer number such as 2.5, or a text value, THEN THE Database SHALL reject the write, SHALL return an error to the caller, and SHALL keep the job's stored Company_Stability and all other column values unchanged.
6. WHEN a job is created through the Desktop_UI, the CLI, a watch save, or a CSV row without a `company_stability` value, THE Job_Tracker SHALL store the new job as an Unscored_Job, and a subsequent read of that job SHALL return `companyStability` as null.
7. IF the Migrator fails while adding the `company_stability` column, THEN THE Migrator SHALL report an error, SHALL leave the `jobs` table schema and every stored job value as they were before the migration started, and SHALL NOT mark the migration as applied.

### Requirement 2: Set and clear Company_Stability through the Job_Service and IPC_Layer

**User Story:** As a job seeker, I want to set, change, and clear the stability score from any client, so that the score stays correct as I learn more about a company.

#### Acceptance Criteria

1. THE Job_Service SHALL include a `companyStability` field in every serialized job returned by job detail, job list, and job page responses, with the value set to the job's stored Valid_Stability_Score, or to JSON null when the job is an Unscored_Job. The field SHALL be present, not omitted, in both cases.
2. WHEN the IPC_Layer receives an `update_job` request for an existing job whose `companyStability` field is a Valid_Stability_Score, THE Job_Service SHALL store that score on the job and return the updated job with `companyStability` equal to that score.
3. WHEN the IPC_Layer receives an `update_job` request for an existing job whose `companyStability` field is explicitly null, THE Job_Service SHALL make the job an Unscored_Job and return the updated job with `companyStability` equal to null, including when the job was already an Unscored_Job.
4. WHEN the IPC_Layer receives an `update_job` request for an existing job that omits the `companyStability` field, THE Job_Service SHALL keep the job's stored Company_Stability equal to its value before the request.
5. IF an `update_job` request carries a `companyStability` value that is not a Valid_Stability_Score (an integer below 1 or above 5, a non-integer number such as 3.5, a string, a boolean, an array, or an object), THEN THE Job_Service SHALL reject the whole update with an error containing the Stability_Range_Message and keep every field of the job, including Company_Stability and the updated timestamp, equal to its value before the request.
6. WHEN the Job_Service changes a job's Company_Stability, THE Job_Service SHALL keep the job's Appeal, Application_Ease, favorite flag, and status equal to their values before the change.
7. WHEN the Job_Service changes a job's Appeal, Application_Ease, favorite flag, or status, THE Job_Service SHALL keep the job's Company_Stability equal to its value before the change.
8. WHEN a job is archived, restored from archive, or set to `closed`, THE Job_Service SHALL keep the job's Company_Stability equal to its value before the status change.
9. WHEN the Job_Service stores a new Company_Stability value (set or clear), THE Job_Service SHALL refresh the CSV_Mirror in the same way as for an Appeal change, so that the CSV_Mirror row for that job shows the new Company_Stability value after the update returns.
10. FOR ALL Valid_Stability_Scores (1, 2, 3, 4, 5) and null, setting Company_Stability through the Job_Service and then reading the job by job detail and by job list SHALL return the value that was set (round-trip property).
11. IF an `update_job` request that includes a `companyStability` field targets a job id that does not exist in the Database, THEN THE Job_Service SHALL reject the request with an error indicating the job was not found, and SHALL NOT create a job or change any other job.

### Requirement 3: Set and read Company_Stability from the CLI

**User Story:** As an external agent or script, I want CLI flags and JSON output for the stability score, so that I can automate scoring without driving the GUI.

#### Acceptance Criteria

1. WHEN a user runs `jt update <id> --company-stability <n>` with n a Valid_Stability_Score, THE CLI SHALL store n as the job's Company_Stability through the Job_Service, exit with status 0, and leave the job's Appeal, Application_Ease, status, and notes unchanged unless other flags in the same command change them.
2. WHEN a user runs `jt update <id> --clear-company-stability`, THE CLI SHALL make the job an Unscored_Job through the Job_Service and exit with status 0, including when the job was already an Unscored_Job.
3. WHEN a user runs `jt update <id>` without `--company-stability` or `--clear-company-stability`, THE CLI SHALL keep the job's stored Company_Stability unchanged, whether it was a Valid_Stability_Score or null.
4. IF a user passes a `--company-stability` value that is not a Valid_Stability_Score (for example 0, 6, -1, 2.5, or a non-numeric string), THEN THE CLI SHALL exit with a nonzero status before writing to the Database, print an argument error stating the 1–5 range with 5 meaning most stable, and keep every stored field of the job unchanged.
5. IF a user passes both `--company-stability` and `--clear-company-stability` in one command, THEN THE CLI SHALL exit with a nonzero status before writing to the Database, print an argument conflict error naming both flags, and keep every stored field of the job unchanged.
6. IF the `<id>` given to `jt update` with `--company-stability` or `--clear-company-stability` matches no job, THEN THE CLI SHALL exit with a nonzero status, print an error indicating the job was not found, and change no job in the Database.
7. WHEN a user runs `jt get <id> --json`, `jt list --json`, or `jt update <id> ... --json`, THE CLI SHALL include each returned job's Company_Stability as the `companyStability` field, with the integer value 1–5 for a scored job and JSON null for an Unscored_Job, and SHALL reflect the value stored after the update for `jt update`.
8. WHEN a user runs `jt list` without `--json`, THE CLI SHALL show a `STABILITY` table column holding the job's Company_Stability as an integer 1–5, or "—" for an Unscored_Job.
9. WHEN a user runs `jt get <id>` or `jt update <id>` without `--json`, THE CLI SHALL print a "Stability" line showing the Company_Stability (or "—" for an Unscored_Job) followed by the scale hint "(1-5, 5 = most stable)".
10. THE CLI SHALL describe `--company-stability` and `--clear-company-stability` in `jt update --help`, including the 1–5 range, that 5 means most stable, and that the clear flag leaves the job unscored.
11. WHEN the CLI applies `--company-stability` or `--clear-company-stability`, THE CLI SHALL keep the job's favorite flag unchanged.
12. WHEN `jt update` successfully sets or clears Company_Stability, THE CLI SHALL refresh the CSV_Mirror so that the job's Company_Stability value in the Jobs CSV equals the stored value (empty for an Unscored_Job) before the command exits.

### Requirement 4: Round-trip Company_Stability through the CSV_Mirror

**User Story:** As a job seeker who edits jobs in a spreadsheet, I want the stability score in the Jobs CSV, so that I can bulk-score jobs and keep the CSV and the app in sync.

#### Acceptance Criteria

1. WHEN the CSV_Mirror exports jobs, THE CSV_Mirror SHALL write a header row with a `company_stability` column immediately after the `application_ease` column, keep every existing column in its current relative order, and write the same number of cells in every data row as in the header row.
2. WHEN the CSV_Mirror exports a job, THE CSV_Mirror SHALL write the job's Company_Stability as a single digit from 1 to 5 with no sign, decimal point, or surrounding whitespace, or an empty cell for an Unscored_Job.
3. WHEN the CSV_Mirror imports a row whose `company_stability` cell, after trimming leading and trailing whitespace, parses as a base-10 integer that is a Valid_Stability_Score, THE CSV_Mirror SHALL store that score as the Company_Stability of the matching existing job, or of the new job created from that row when no existing job matches.
4. WHEN the CSV_Mirror imports a row whose `company_stability` cell is empty or holds only whitespace, THE CSV_Mirror SHALL set the matching existing job's Company_Stability to null, or create the new job from that row as an Unscored_Job, and leave the job's Appeal, Application_Ease, and favorite state unchanged.
5. WHEN the CSV_Mirror imports a file whose header row has no `company_stability` column, THE CSV_Mirror SHALL keep every matching existing job's stored Company_Stability unchanged and create every new job from that file as an Unscored_Job.
6. IF the CSV_Mirror imports a row whose trimmed `company_stability` cell is not empty and is not a base-10 integer from 1 to 5 (for example `0`, `6`, `3.5`, `-1`, or `high`), THEN THE CSV_Mirror SHALL skip that row, report an error for that row whose message contains "Invalid company stability" and the trimmed cell value, leave the matching job's fields unchanged (or create no job when no job matches), and continue importing the remaining rows.
7. WHILE the CSV_Mirror runs in merge mode, WHEN Company_Stability differs between the CSV row and the Database, THE CSV_Mirror SHALL apply the CSV value only when the Database value is unchanged since the last export, and SHALL keep the Database value when both the CSV value and the Database value changed since the last export.
8. WHEN the CSV_Mirror imports a file or reads sync sidecar state written before this feature (with no `company_stability` column or `companyStability` field), THE CSV_Mirror SHALL complete the import without reporting any error caused by the missing column or field, and SHALL leave every existing job's Company_Stability unchanged.
9. FOR ALL jobs with any Company_Stability value (a Valid_Stability_Score or null), exporting the jobs and then importing the exported file in merge mode with no edits SHALL leave every job's Company_Stability, Appeal, Application_Ease, and favorite state unchanged (round-trip property).
10. FOR ALL `company_stability` cell values that parse to a Valid_Stability_Score or null, parsing the cell, writing the result back as a cell, and parsing again SHALL produce the same value (round-trip property).

### Requirement 5: Edit and display Company_Stability in the Desktop_UI

**User Story:** As a job seeker, I want to set the stability score from the job list and the job detail page, so that I can score jobs while I review them.

#### Acceptance Criteria

1. THE Job_Detail_Form SHALL show a Company_Stability select labeled "Company stability (1–5, 5=most stable)" with exactly these six options, in this order: "Unscored", "1 — least stable", "2", "3", "4", and "5 — most stable".
2. WHEN the Job_Detail_Form loads a job, THE Job_Detail_Form SHALL preselect the option matching the job's stored Company_Stability, or "Unscored" when the job is an Unscored_Job.
3. WHEN a user changes the Company_Stability select in the Job_Detail_Form to a value different from the last saved value, THE Job_Detail_Form SHALL mark the form as having unsaved changes, in the same way a change to the Appeal or Application_Ease select does.
4. WHEN a user changes the Company_Stability select in the Job_Detail_Form and saves, THE Desktop_UI SHALL send the chosen Valid_Stability_Score, or null for "Unscored", as `companyStability` in the `update_job` request, and after a successful save SHALL show the Company_Stability value returned in the `update_job` response as the new saved value.
5. WHEN a user saves the Job_Detail_Form without changing the Company_Stability select, THE Desktop_UI SHALL leave the job's stored Company_Stability unchanged, so that a later `jt get <id> --json` returns the same `companyStability` value as before the save.
6. IF the `update_job` request from a Job_Detail_Form save fails, THEN THE Job_Detail_Form SHALL keep the user's selected Company_Stability in the form, keep the form marked as having unsaved changes, and show the error message, or "Save failed" when the error has no message.
7. THE Jobs_List SHALL show a compact Company_Stability select labeled "Stability" on each job row, placed after the Appeal and Ease selects, with the same six options as the Job_Detail_Form select and preselected to the row's stored Company_Stability, or "Unscored" for an Unscored_Job.
8. WHEN a user changes a row's compact Company_Stability select, THE Jobs_List SHALL show the new value in that row before the `update_job` response arrives and send an `update_job` request carrying only the `companyStability` field, set to the chosen Valid_Stability_Score or null for "Unscored".
9. WHEN the `update_job` request from a row's compact Company_Stability select succeeds, THE Jobs_List SHALL set that row's Company_Stability to the value returned in the `update_job` response and leave every other row's Company_Stability unchanged.
10. IF the `update_job` request from a row's compact Company_Stability select fails, THEN THE Jobs_List SHALL restore the row's Company_Stability to its value before the change and show the error message, or "Failed to update company stability" when the error has no message.
11. WHEN a user clicks, focuses, or changes a row's compact Company_Stability select, THE Jobs_List SHALL not open the job detail page for that row.
12. WHILE a job has a Valid_Stability_Score, THE Jobs_Board SHALL show the text "Stability n", where n is the job's Company_Stability, immediately after the job's Ease label (or after the Appeal label when Ease is not shown) in every Jobs_Board view that shows the Appeal and Ease labels, with the tooltip "Company stability (1–5, 5=most stable)".
13. WHILE a job is an Unscored_Job, THE Jobs_Board SHALL omit the Company_Stability label and its separator for that job.

### Requirement 6: Sort the Jobs_List by Company_Stability

**User Story:** As a job seeker, I want to sort my jobs by company stability, so that I can focus on postings at stable companies first.

#### Acceptance Criteria

1. THE Jobs_List sort control SHALL offer, on every Jobs tab including the applied tab, exactly one option labeled "Company stability" that selects the Stability_Sort_Field, alongside the existing Date added, Appeal, and Company name options.
2. WHEN a user selects the Stability_Sort_Field without choosing a direction, THE Jobs_List SHALL sort in descending order.
3. WHEN the Stability_Sort_Field is active in descending order, THE Job_Service SHALL order scored jobs from Company_Stability 5 down to 1.
4. WHEN the Stability_Sort_Field is active in ascending order, THE Job_Service SHALL order scored jobs from Company_Stability 1 up to 5.
5. WHEN the Stability_Sort_Field is active in either direction, THE Job_Service SHALL place every Unscored_Job after every scored job.
6. WHEN two jobs under the Stability_Sort_Field are in the same category, meaning both are scored with equal Company_Stability or both are Unscored_Jobs, THE Job_Service SHALL order the two jobs by job `id` ascending, in both ascending and descending order; the placement of scored jobs before Unscored_Jobs defined in criterion 5 SHALL take precedence over this tie-break.
7. WHEN the URL holds `sortBy=companyStability` with `sortOrder` set to `ascending` or `descending`, THE Jobs_List SHALL apply the Stability_Sort_Field in that direction on every Jobs tab, including the applied tab, and SHALL show "Company stability" and that direction as selected in the sort control.
8. IF the URL holds `sortBy=companyStability` and `sortOrder` is absent or holds any value other than `ascending` or `descending`, THEN THE Jobs_List SHALL apply the Stability_Sort_Field in descending order without showing an error.
9. WHEN the user selects the Stability_Sort_Field or changes its direction, THE Jobs_List SHALL write `sortBy=companyStability` and the chosen `sortOrder` to the URL, replacing any previous `sortBy` and `sortOrder` values.
10. WHEN the Job_Service pages a job list under the Stability_Sort_Field, THE Job_Service SHALL encode the job's Company_Stability (or null for an Unscored_Job) together with the job `id` as the cursor, so that a page boundary falling between scored and Unscored_Jobs, or between jobs with equal Company_Stability, resumes at the next job in the order defined by criteria 3 through 6.
11. FOR ALL sets of 0 to 500 jobs and page sizes from 1 to 100, concatenating the pages returned under the Stability_Sort_Field in either direction SHALL equal the single unpaged ordered list, with no job repeated or omitted (model-based property).
12. FOR ALL sets of 0 to 500 jobs, the order the Job_Service returns under the Stability_Sort_Field SHALL equal the order produced by a reference sort on (Unscored last, Company_Stability in the chosen direction, `id` ascending) (model-based property).
13. WHEN a user changes a row's Company_Stability while the Stability_Sort_Field is active, THE Jobs_List SHALL save the new value, show it in that row, and keep the row in its current position until the next list reload (a change of tab, sort, filter, or search, or a reload of the Jobs page), matching the existing Appeal sort behavior.
14. WHILE the Minimum_Stability_Filter is set, WHEN the Stability_Sort_Field is active, THE Job_Service SHALL return only jobs matching the filter, ordered as defined in criteria 3 through 6.

### Requirement 7: Filter the Jobs_List by minimum Company_Stability

**User Story:** As a job seeker, I want to hide postings below a stability threshold, so that I can review only jobs at companies I consider stable enough.

#### Acceptance Criteria

1. THE Jobs_List filter panel SHALL offer a Minimum_Stability_Filter control with exactly six choices in this order: "Any" (no value), "1+" (value 1), "2+" (value 2), "3+" (value 3), "4+" (value 4), and "5" (value 5), with "Any" selected by default.
2. WHILE the Minimum_Stability_Filter holds a value n, THE Job_Service SHALL return only jobs whose Company_Stability is greater than or equal to n.
3. WHILE the Minimum_Stability_Filter holds a value, THE Job_Service SHALL exclude every Unscored_Job from the results.
4. WHILE the Minimum_Stability_Filter is "Any", THE Job_Service SHALL apply no Company_Stability restriction, so that both scored jobs and Unscored_Jobs remain eligible for the results.
5. WHEN a user changes the Minimum_Stability_Filter, THE Jobs_List SHALL write the selected value to the URL as `minStability=<n>` for values 1 through 5 and SHALL remove the `minStability` parameter from the URL when the selected value is "Any".
6. WHEN the Jobs_List loads a URL whose `minStability` parameter is absent, empty, non-numeric, non-integer, less than 1, or greater than 5, THE Jobs_List SHALL treat the Minimum_Stability_Filter as "Any" and SHALL display a list request with no Company_Stability restriction, without showing an error.
7. IF the IPC_Layer receives a minimum stability filter value that is not a Valid_Stability_Score, THEN THE IPC_Layer SHALL reject the list request with the Stability_Range_Message, SHALL return no job rows for that request, and SHALL leave every stored job unchanged.
8. WHILE the Minimum_Stability_Filter holds a value, THE Jobs_List SHALL add exactly 1 to the active filter count for the Minimum_Stability_Filter, and WHILE the Minimum_Stability_Filter is "Any", THE Jobs_List SHALL add 0 for it.
9. WHEN the user clears all filters, THE Jobs_List SHALL reset the Minimum_Stability_Filter to "Any" and SHALL remove the `minStability` parameter from the URL.
10. WHILE the Minimum_Stability_Filter holds a value, THE Job_Service SHALL apply the filter together with every other active filter, the search text, the selected sort, and the selected tab, so that a returned job satisfies all of them.
11. FOR ALL sets of jobs and thresholds n from 1 to 5, the ordered list of jobs the Job_Service returns across all pages with the Minimum_Stability_Filter set to n SHALL equal the ordered list returned across all pages without the filter, restricted to jobs with Company_Stability ≥ n, with the same relative order (metamorphic property).
12. FOR ALL thresholds m and n from 1 to 5 with m ≤ n, the result set with the Minimum_Stability_Filter set to n SHALL be a subset of the result set with the filter set to m, and the result set with the filter set to m SHALL be a subset of the result set with the filter set to "Any" (metamorphic property).
13. WHILE the Minimum_Stability_Filter holds a value n, THE Job_Service SHALL report a total result count equal to the number of jobs that satisfy the Minimum_Stability_Filter and every other active restriction, and THE Jobs_List SHALL display that count and page through only those jobs.
14. WHEN a user changes the Minimum_Stability_Filter, THE Jobs_List SHALL discard any page cursor from the previous result set and request the first page of results (page 0 in zero-based numbering, sent with no cursor).

### Requirement 8: Filter `jt list` by minimum Company_Stability

**User Story:** As an external agent, I want a CLI filter for the stability score, so that I can query stable-company jobs from scripts.

#### Acceptance Criteria

1. WHEN a user runs `jt list --min-company-stability <n>` with n a Valid_Stability_Score, THE CLI SHALL return only jobs whose Company_Stability is a Valid_Stability_Score greater than or equal to n, and SHALL exclude every Unscored_Job from the result.
2. WHEN a user runs `jt list` without `--min-company-stability`, THE CLI SHALL apply no Company_Stability filtering, so that the result set is identical to the result of the same command before this feature, including Unscored_Jobs.
3. WHEN a user runs `jt list --min-company-stability <n> --json` with n a Valid_Stability_Score, THE CLI SHALL include each returned job's `companyStability` value in the JSON output, and SHALL return an empty job list with a zero exit status when no job matches.
4. IF a user passes a `--min-company-stability` value that is not a Valid_Stability_Score (including 0, 6, negative numbers, decimals such as 3.5, non-numeric text, or a missing value), THEN THE CLI SHALL exit with a nonzero status, print an argument error indicating the accepted 1–5 integer range, return no jobs, and leave the Database and CSV_Mirror unchanged.
5. WHEN a user combines `--min-company-stability` with other `jt list` filters (such as `--status` and `--search`), THE CLI SHALL return only jobs that match every given filter.
6. THE CLI SHALL describe `--min-company-stability` in `jt list --help`, including the 1–5 integer range, that 5 means most stable, that matching is greater than or equal to the given value, and that Unscored_Jobs are excluded when the flag is set.

### Requirement 9: Sort the Companies_Sidebar by Company_Stability_Average

**User Story:** As a job seeker, I want to sort companies by their average stability score, so that I can see which companies I rated most stable.

#### Acceptance Criteria

1. THE Job_Service SHALL include in every Companies_Sidebar company row a `stabilityAverage` value and a count of the company's Pipeline_Jobs whose Company_Stability is a Valid_Stability_Score. `stabilityAverage` SHALL hold the unrounded Company_Stability_Average, within 0.001 of the exact mean and between 1 and 5 inclusive, or null when that count is 0.
2. WHEN the Job_Service computes Company_Stability_Average or the scored Pipeline_Job count, THE Job_Service SHALL include only Pipeline_Jobs whose Company_Stability is a Valid_Stability_Score, and SHALL exclude Unscored_Jobs and jobs that are not Pipeline_Jobs (archived jobs, jobs new from a watch, and jobs with a dismissed watch disposition).
3. THE Companies_Sidebar sort menu SHALL list the Stability_Company_Sort, labeled "Most stable company", immediately after the "Easiest wishlist to apply" option and in the same group as that option, and SHALL apply the existing selection, keyboard navigation, focus, and ARIA behavior of the sort menu to it.
4. WHILE the Stability_Company_Sort is active, THE Companies_Sidebar SHALL order companies with a non-null Company_Stability_Average by the unrounded average from highest (5) to lowest (1), then by the scored Pipeline_Job count from highest to lowest, then by the existing name order, so that an average of 4.67 sorts above 4.6 even though both display as 4.7 and 4.6 respectively.
5. WHILE the Stability_Company_Sort is active, THE Companies_Sidebar SHALL place every company whose Company_Stability_Average is null, including companies with zero Pipeline_Jobs and companies whose Pipeline_Jobs are all Unscored_Jobs, after every company with a non-null average, ordered by the existing name order.
6. IF a company row's `stabilityAverage` is missing, null, non-numeric, non-finite, less than 1, or greater than 5, THEN THE Companies_Sidebar SHALL treat the Company_Stability_Average as null for sorting and annotation, SHALL still list the company, and SHALL not raise an error.
7. WHILE the Stability_Company_Sort is active and a company has a non-null Company_Stability_Average, THE Companies_Sidebar SHALL show a Sort_Annotation with the average rounded to one decimal place (halves away from zero, always showing one decimal digit, for example "4.0") and the accessible label "Average stability <value>", where <value> is the same rounded number.
8. WHILE the Stability_Company_Sort is active and a company's Company_Stability_Average is null, THE Companies_Sidebar SHALL show no Sort_Annotation for that company.
9. WHEN the Companies_Sidebar saves and reloads its view state with the Stability_Company_Sort active, THE Companies_Sidebar SHALL restore the Stability_Company_Sort and the saved company filters unchanged.
10. WHEN the Companies_Sidebar loads a view state saved before this feature, THE Companies_Sidebar SHALL restore the saved sort and filters unchanged and SHALL not select the Stability_Company_Sort.
11. FOR ALL databases of companies and jobs, every company row's `stabilityAverage` SHALL be within 0.001 of the mean computed directly from the glossary definition of Company_Stability_Average, and SHALL be null exactly when that definition yields null (model-based property).
12. WHILE the Stability_Company_Sort is active and one or more company filters are active, THE Companies_Sidebar SHALL apply the ordering in criteria 4 and 5 only to companies that pass every active company filter, and SHALL show no company that a filter hides.
13. WHILE the Stability_Company_Sort is active, WHEN the Desktop_UI persists a set, change, or clear of a job's Company_Stability, THE Companies_Sidebar SHALL re-order companies and update their Sort_Annotations on the active view from the new Company_Stability_Average values as soon as the `update_job` request succeeds, without waiting for the next load of company rows.
14. WHILE the Stability_Company_Sort is active, WHEN the Companies_Sidebar re-orders companies after a Desktop_UI change to a job's Company_Stability, THE Companies_Sidebar SHALL keep the active sort and the active company filters unchanged.
15. WHILE the Stability_Company_Sort is active, WHEN a job's Company_Stability is set, changed, or cleared outside the running Desktop_UI (for example through the CLI or a CSV import), THE Companies_Sidebar SHALL re-order companies and update their Sort_Annotations from the new Company_Stability_Average values on its next load of company rows, and SHALL keep the active sort and the active company filters unchanged.

### Requirement 10: Document Company_Stability for users and agents

**User Story:** As an external agent or a new user, I want the stability score documented alongside the other scores, so that I can use it correctly.

#### Acceptance Criteria

1. THE Job_Tracker SHALL document in `AGENTS.md`, in the same section that documents Application_Ease, the `jt update <id> --company-stability <1..5> --json` flag, stating that the value must be a Valid_Stability_Score, that 1 = least stable and 5 = most stable, and that omitting the flag leaves the stored Company_Stability unchanged.
2. THE Job_Tracker SHALL document in `AGENTS.md` the `jt update <id> --clear-company-stability --json` flag, stating that it sets Company_Stability back to null.
3. THE Job_Tracker SHALL document in `AGENTS.md` the `jt list --min-company-stability <1..5> --json` filter, stating that it returns only jobs whose Company_Stability is greater than or equal to the given value and that Unscored_Jobs are excluded while the filter is set.
4. THE Job_Tracker SHALL document in `AGENTS.md` that `jt get --json` and `jt list --json` return the score as the `companyStability` field, with a value of null for an Unscored_Job.
5. THE Job_Tracker SHALL document in `AGENTS.md` that setting or clearing Company_Stability does not favorite the job.
6. THE Job_Tracker SHALL add to the "Commands agents actually use" table in `AGENTS.md` one row for setting Company_Stability, using the same column format as the existing Application_Ease row.
7. THE Job_Tracker SHALL document in `README.md` the `company_stability` column of the Jobs CSV, stating that the column holds a Valid_Stability_Score with 1 = least stable and 5 = most stable.
8. THE Job_Tracker SHALL document in `README.md` that a blank `company_stability` cell in an imported CSV row means the job is an Unscored_Job.
9. THE Job_Tracker SHALL document in `README.md` that importing a CSV file that has no `company_stability` column leaves every stored Company_Stability value unchanged.
