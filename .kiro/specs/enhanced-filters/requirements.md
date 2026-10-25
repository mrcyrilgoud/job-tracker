# Requirements Document

## Introduction

This feature enhances the live filtering capabilities of the Job Tracker jobs list so that a user can narrow the pipeline by three independently selectable and combinable dimensions: pipeline status (multiple statuses at once), a date range on a chosen date field, and location (text match plus remote mode). Today the jobs list supports single-select status (via sidebar links and the `status` URL parameter), a backend-only substring location filter, posting-state, and salary range in the filter popover. There is no multi-select status, no date-range filtering, and no location control surfaced in the popover UI.

The enhanced filters must combine with each other and with the existing search, posting-state, salary, favorites, and archived filters, persist in the URL so views are shareable and restorable, follow the existing filter-popover UX, and extend the shared Rust service and Tauri IPC path so the CLI (`jt list`) benefits from the same capabilities. The backend remains Rust with SQLite; no Node or Next server is introduced.

## Glossary

- **Jobs_List**: The desktop jobs listing view implemented in `desktop/src/pages/JobsPage.tsx`, including its filter popover, sidebar, list, and board renderings.
- **Filter_Popover**: The existing popover on the Jobs_List that collects filter inputs (currently search, posting state, and salary range) and applies them on submit.
- **Job_Status**: One of the persisted pipeline status values: `wishlist`, `applied`, `interviewing`, `offer`, `rejected`, `withdrawn`, `closed`, `archived`.
- **Status_Filter**: A filter that restricts the Jobs_List to jobs whose Job_Status is any one of a selected set of one or more Job_Status values.
- **Date_Field**: A selectable job timestamp used by the Date_Filter. Supported fields are `appliedAt` (date applied), `postingDate` (the job's `createdAt`), and `lastCheckedAt` (last freshness check).
- **Date_Filter**: A filter that restricts the Jobs_List to jobs whose selected Date_Field falls within an inclusive date range bounded by an optional start date and an optional end date.
- **Location_Filter**: A filter that restricts the Jobs_List by a case-insensitive substring match against the job `location` string and/or by a selected Remote_Mode.
- **Remote_Mode**: One of `any`, `remoteOnly`, or `onsiteOnly`, mirroring the existing `RemoteMode` type in `desktop/src/lib/schema.ts`.
- **Filter_State**: The complete set of active filters on the Jobs_List, encoded in the URL query parameters.
- **Job_Service**: The shared Rust filtering service in `src-tauri/src/jobs/service.rs` (notably `JobFilters` and `job_filter_sql`) that backs both Tauri IPC commands and the CLI.
- **CLI**: The `jt` / `job-tracker` command-line interface whose `list` subcommand (`src-tauri/src/cli/`) calls the same Job_Service as the Tauri IPC path.
- **JobFilters**: The filter payload type passed from the UI and CLI into the Job_Service (`JobFilters` in `desktop/src/lib/api.ts` and `src-tauri/src/jobs/service.rs`).

## Requirements

### Requirement 1: Multi-select status filtering

**User Story:** As a job seeker, I want to filter the jobs list by more than one pipeline status at once, so that I can view related stages together (for example applied and interviewing) without clicking through single-status views.

#### Acceptance Criteria

1. THE Filter_Popover SHALL present a control that allows the user to select between zero and eight distinct Job_Status values for the Status_Filter, where the selectable set is exactly the defined Job_Status values (wishlist, applied, interviewing, offer, rejected, withdrawn, closed, archived).
2. WHEN the user applies the Filter_Popover with two or more Job_Status values selected, THE Jobs_List SHALL display only jobs whose Job_Status equals any one of the selected values, and SHALL exclude every job whose Job_Status is not among the selected values.
3. WHEN the user applies the Filter_Popover with exactly one Job_Status value selected, THE Jobs_List SHALL display only jobs whose Job_Status equals that value.
4. WHEN the user applies the Filter_Popover with zero Job_Status values selected, THE Jobs_List SHALL apply no Status_Filter restriction and display jobs regardless of Job_Status.
5. WHEN the Status_Filter is active, THE Jobs_List SHALL encode the selected Job_Status values in the Filter_State using the URL query parameters, listing each selected value at most once.
6. WHEN the Jobs_List loads with Status_Filter values present in the URL query parameters, THE Jobs_List SHALL restore the Status_Filter to the set of values from the URL query parameters that are members of Job_Status, ignoring any duplicate or non-member values.
7. WHERE the CLI `list` subcommand receives more than one status value and every provided value is a member of Job_Status, THE CLI SHALL return only jobs whose Job_Status equals any one of the provided values.
8. WHEN one or more status values provided to the Status_Filter are not members of Job_Status and at least one provided value is a member of Job_Status, THE Job_Service SHALL exclude every non-member value and apply the remaining member values as the Status_Filter.
9. IF every status value provided to the Status_Filter is not a member of Job_Status, THEN THE Job_Service SHALL reject the Status_Filter request, return an error indicating no valid Job_Status value was provided, and leave the previously applied Status_Filter unchanged.
10. IF the CLI `list` subcommand receives one or more status values that are not members of Job_Status, THEN THE CLI SHALL return a nonzero exit status, emit an error indicating which value is not a valid Job_Status, and return no jobs.

### Requirement 2: Date range filtering by selectable date field

**User Story:** As a job seeker, I want to filter the jobs list by a date range on a date field I choose, so that I can find jobs applied to, posted, or last checked within a specific window.

#### Acceptance Criteria

1. THE Filter_Popover SHALL present a control that allows the user to select exactly one Date_Field from `appliedAt`, `postingDate`, and `lastCheckedAt` for the Date_Filter.
2. THE Filter_Popover SHALL present controls that allow the user to enter an optional start date and an optional end date for the Date_Filter, each expressed as a calendar date in ISO 8601 `YYYY-MM-DD` form.
3. WHEN the user applies the Filter_Popover with a Date_Field and a start date, THE Jobs_List SHALL display only jobs whose selected Date_Field value is at or after the start of the start date's calendar day in the user's local time zone.
4. WHEN the user applies the Filter_Popover with a Date_Field and an end date, THE Jobs_List SHALL display only jobs whose selected Date_Field value is at or before the end of the end date's calendar day in the user's local time zone.
5. WHEN the user applies the Filter_Popover with a Date_Field, a start date, and an end date, THE Jobs_List SHALL display only jobs whose selected Date_Field value falls within the inclusive range from the start of the start date's calendar day to the end of the end date's calendar day in the user's local time zone.
6. IF the selected Date_Field for a job has no stored value, THEN THE Job_Service SHALL exclude that job from the Date_Filter results.
7. IF the start date is later than the end date, THEN THE Filter_Popover SHALL display a validation message indicating that the start date must not be later than the end date and SHALL retain the current applied Filter_State without applying the Date_Filter.
8. IF a supplied start date or end date is not a valid calendar date in `YYYY-MM-DD` form, THEN THE Filter_Popover SHALL display a validation message indicating that the entered date is not a valid calendar date and SHALL retain the current applied Filter_State without applying the Date_Filter.
9. WHEN the Date_Filter is active, THE Jobs_List SHALL encode the selected Date_Field, start date, and end date in the Filter_State using the URL query parameters.
10. WHEN the Jobs_List loads with Date_Filter values present in the URL query parameters, THE Jobs_List SHALL restore the Date_Field, start date, and end date to those values.
11. WHERE the CLI `list` subcommand receives a Date_Field with a start date and an end date, THE CLI SHALL return only jobs whose selected Date_Field value falls within the inclusive range from the start of the start date's calendar day to the end of the end date's calendar day in the user's local time zone.
12. WHERE the CLI `list` subcommand receives a Date_Field with only a start date or only an end date, THE CLI SHALL return only jobs whose selected Date_Field value is at or after the start of the start date's calendar day, or at or before the end of the end date's calendar day, respectively, in the user's local time zone.

### Requirement 3: Location filtering by text and remote mode

**User Story:** As a job seeker, I want to filter the jobs list by location text and by whether a role is remote or onsite, so that I can focus on roles in places I can work.

#### Acceptance Criteria

1. THE Filter_Popover SHALL present a control that allows the user to enter location text of 0 to 200 characters for the Location_Filter.
2. THE Filter_Popover SHALL present a single-select control that allows the user to select one Remote_Mode value from `any`, `remoteOnly`, and `onsiteOnly` for the Location_Filter.
3. WHEN the user applies the Filter_Popover with non-empty location text, THE Jobs_List SHALL display only jobs whose `location` string contains the entered text using a case-insensitive substring match.
4. WHEN the user applies the Filter_Popover with location text that is empty or contains only whitespace, THE Jobs_List SHALL apply no location text restriction.
5. WHEN the user applies the Filter_Popover with Remote_Mode set to `remoteOnly`, THE Jobs_List SHALL display only jobs whose `location` string contains, by case-insensitive substring match, at least one of the remote indicator terms `remote`, `anywhere`, `work from home`, or `wfh`.
6. WHEN the user applies the Filter_Popover with Remote_Mode set to `onsiteOnly`, THE Jobs_List SHALL display only jobs whose `location` string contains none of the remote indicator terms `remote`, `anywhere`, `work from home`, or `wfh` by case-insensitive substring match.
7. WHEN the user applies the Filter_Popover with Remote_Mode set to `any`, THE Jobs_List SHALL apply no Remote_Mode restriction to the Location_Filter.
8. WHEN the user applies the Filter_Popover with both non-empty location text and a Remote_Mode of `remoteOnly` or `onsiteOnly`, THE Jobs_List SHALL display only jobs that satisfy both the location text match and the Remote_Mode condition.
9. IF a job has no stored `location` value, THEN THE Job_Service SHALL exclude that job from results when non-empty location text or a Remote_Mode of `remoteOnly` or `onsiteOnly` is active.
10. WHEN the Location_Filter is active, THE Jobs_List SHALL encode the location text and the Remote_Mode in the Filter_State using the URL query parameters.
11. WHEN the Jobs_List loads with Location_Filter values present in the URL query parameters, THE Jobs_List SHALL restore the location text and the Remote_Mode to those values.
12. WHERE the CLI `list` subcommand receives a Remote_Mode value that is a member of the Remote_Mode set, THE CLI SHALL apply the corresponding Remote_Mode condition to its results.
13. IF the CLI `list` subcommand receives a Remote_Mode value that is not a member of the Remote_Mode set, THEN THE CLI SHALL return a nonzero exit status, emit an error indicating the invalid value, and return no jobs.

### Requirement 4: Combined filtering and existing filter compatibility

**User Story:** As a job seeker, I want the status, date, and location filters to work together and alongside the existing filters, so that I can compose a precise view of my pipeline.

#### Acceptance Criteria

1. WHEN two or more of the Status_Filter, Date_Filter, and Location_Filter are active, THE Jobs_List SHALL display only jobs that satisfy every one of the active filters simultaneously (logical conjunction).
2. WHEN the Status_Filter, Date_Filter, or Location_Filter is active together with the existing search, posting-state, salary-range, favorites, or archived filters, THE Jobs_List SHALL display only jobs that satisfy every one of the active filters simultaneously (logical conjunction).
3. THE Job_Service SHALL apply the Status_Filter, Date_Filter, and Location_Filter as additional conjunctive conditions within the existing `job_filter_sql` query construction.
4. WHERE two Filter_State values select the same set of active filters with the same filter values in any selection order, THE Job_Service SHALL return the identical set of jobs for both.
5. WHEN the active Filter_State matches zero jobs, THE Jobs_List SHALL display an empty-result state that renders no job rows and displays a message indicating that no jobs match the active filters.
6. THE Jobs_List SHALL display a single combined active-filter indicator whose count equals the number of active existing filters plus the number of distinct filter dimensions among Status_Filter, Date_Filter, and Location_Filter that are active, where the three new dimensions contribute 0 to 3 and each dimension is counted at most once.

### Requirement 5: Filter reset and persistence

**User Story:** As a job seeker, I want to clear the enhanced filters and have my applied filters survive navigation, so that I can return to an unfiltered list or revisit a filtered view reliably.

#### Acceptance Criteria

1. WHEN the user clears all of the Status_Filter, Date_Filter, and Location_Filter, THE Jobs_List SHALL remove every corresponding Filter_State query parameter for those three filters from the URL.
2. WHEN the user clears exactly one of the Status_Filter, Date_Filter, or Location_Filter while the others remain active, THE Jobs_List SHALL remove only the cleared filter's query parameters and SHALL retain the query parameters of the still-active filters.
3. WHEN the user applies the Filter_Popover, THE Jobs_List SHALL preserve the active sort selection in the Filter_State.
4. WHEN the user applies the Filter_Popover, THE Jobs_List SHALL preserve the active favorites and archived selections in the Filter_State.
5. WHEN the user reloads the Jobs_List with Status_Filter, Date_Filter, Location_Filter, and existing-filter values present in the URL query parameters, THE Jobs_List SHALL restore each of those filters to the encoded values.
6. IF a Filter_State query parameter in the URL holds a malformed or out-of-domain value on load, THEN THE Jobs_List SHALL ignore that parameter, restore the remaining valid filters without error, and display a visible notice informing the user that an invalid filter value was ignored.

### Requirement 6: CLI parity for enhanced filters

**User Story:** As an external agent automating through the CLI, I want the same status, date, and location filtering available on `jt list`, so that CLI queries match what the GUI shows against the shared database.

#### Acceptance Criteria

1. THE CLI `list` subcommand SHALL accept a status option that specifies one or more Job_Status values for the Status_Filter and SHALL restrict results to jobs whose Job_Status equals any one of the provided values.
2. THE CLI `list` subcommand SHALL accept options that specify one Date_Field, an optional start date, and an optional end date for the Date_Filter, and SHALL restrict results to jobs whose selected Date_Field falls within the inclusive range bounded by the provided start date and end date.
3. THE CLI `list` subcommand SHALL accept options that specify location text and a Remote_Mode for the Location_Filter, and SHALL restrict results to jobs that satisfy the case-insensitive substring match against the job `location` string and the corresponding Remote_Mode condition.
4. WHEN the CLI `list` subcommand is invoked with `--json`, THE CLI SHALL emit the filtered results as the machine-readable payload consistent with the existing `jt list --json` output.
5. WHEN the CLI `list` subcommand and the Tauri IPC path are invoked with an equivalent Filter_State, THE CLI SHALL return the same set of jobs as the Tauri IPC path because both call the shared Job_Service.
6. IF a status value, Date_Field value, or Remote_Mode value provided to the CLI `list` subcommand is not a member of its defined value set, THEN THE CLI SHALL emit an error indication identifying the rejected option, SHALL exit with a nonzero status, and SHALL emit no filtered results.
7. IF the start date provided to the CLI `list` subcommand is later than the end date, or either supplied date is not a valid calendar date, THEN THE CLI SHALL emit an error indication identifying the invalid date input, SHALL exit with a nonzero status, and SHALL emit no filtered results.
8. WHEN the CLI `list` subcommand is invoked without a status option, THE CLI SHALL apply no Status_Filter restriction and return jobs regardless of Job_Status.
