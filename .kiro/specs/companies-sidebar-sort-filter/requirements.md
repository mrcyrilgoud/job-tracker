# Requirements Document

## Introduction

The Jobs page (`desktop/src/pages/JobsPage.tsx`) has a "Companies" sidebar card with a "Manage" link, a "Search companies…" input, and a scrollable list of company links. Each link sets the `companyId` URL parameter on the jobs list. Today the list has no sort or filter controls. It renders companies in the order returned by the `list_companies` Tauri command (`ORDER BY name` in `src-tauri/src/companies.rs`), which is case-sensitive, so "AKASA" appears before "Abby Care". The only refinement is a case-insensitive substring search (`filterCompaniesBySearch` in `desktop/src/lib/job-filters.ts`).

This feature adds two small buttons next to the sidebar search input, one for sorting and one for filtering, so the user can narrow down and reorder the visible companies before picking one. Sorting and filtering happen in the desktop UI. They use the per-company data that `list_companies` already returns (`CompanyRow`: company `name` and `createdAt`, `watches` with sync status, pending careers-page `reviews`, and `openPositionCount`), plus one new per-company aggregate: the number of tracked jobs. The user's choices persist across app restarts. Search keeps working and combines with the filters.

Out of scope: the jobs-list Filter_Popover and URL filter state (covered by the `enhanced-filters` spec), watch filter criteria (covered by `watchlist-job-filtering`), the `/companies` management page, and CLI changes. No Node or Next server is introduced. Data reaches the UI only through Tauri `invoke` (`desktop/src/lib/api.ts`).

## Glossary

- **Companies_Sidebar**: The "Companies" card in the Jobs page sidebar. It contains the heading, the "Manage" link, the Company_Search input, the Sort_Button, the Filter_Button, and the Company_List.
- **Company_List**: The scrollable list of company links rendered inside the Companies_Sidebar.
- **Company_Search**: The existing "Search companies…" text input. It keeps companies whose name contains the trimmed query, compared case-insensitively.
- **Sort_Button**: A compact button next to the Company_Search that opens the Sort_Menu.
- **Sort_Menu**: A menu opened from the Sort_Button that lists the Sort_Options and marks the active one.
- **Sort_Option**: One of `nameAsc` (Name A–Z), `nameDesc` (Name Z–A), `openPositionsDesc` (Most open positions), `trackedJobsDesc` (Most tracked jobs), or `recentlyAdded` (Recently added, newest company `createdAt` first).
- **Filter_Button**: A compact button next to the Company_Search that opens the Filter_Menu.
- **Filter_Menu**: A menu opened from the Filter_Button that lists the Company_Filters as independent toggles.
- **Company_Filter**: One of these toggles:
  - `watched`: the company has one or more watches.
  - `hasOpenPositions`: the company's Open_Position_Count is greater than 0.
  - `hasTrackedJobs`: the company's Tracked_Job_Count is greater than 0.
  - `needsAttention`: the company has a watch whose `consecutiveSyncFailures` is greater than 0, or has one or more pending careers-page reviews.
- **Open_Position_Count**: The `openPositionCount` value in a `CompanyRow`. It counts the company's jobs with `posting_state = 'active'` from the `greenhouse`, `lever`, or `ashby` sources.
- **Tracked_Job_Count**: For one company, the number of jobs whose `status` is not `archived`.
- **Company_Data_Service**: The Rust function `list_companies_with_watches` in `src-tauri/src/companies.rs`, exposed through the `list_companies` Tauri command.
- **Company_View_State**: The active Sort_Option plus the set of active Company_Filters.
- **Preference_Store**: Browser `localStorage` in the desktop WebView. The existing theme preference is stored there.
- **Name_Order**: Ascending order by company name using locale-aware, case-insensitive comparison. When two names compare equal, the company `id` breaks the tie in ascending order.

## Requirements

### Requirement 1: Compact sort and filter buttons

**User Story:** As a job seeker, I want small sort and filter buttons next to the company search box, so that I can refine the company list without leaving the sidebar.

#### Acceptance Criteria

1. WHILE the Company_Data_Service has returned one or more companies, THE Companies_Sidebar SHALL render the Company_Search input, the Sort_Button, and the Filter_Button on a single row, in that left-to-right order, without wrapping onto a second row.
2. THE Companies_Sidebar SHALL render the Sort_Button and the Filter_Button each with a rendered height less than or equal to the rendered height of the Company_Search input.
3. WHEN the user activates the Sort_Button while the Sort_Menu is closed, THE Companies_Sidebar SHALL open the Sort_Menu and move keyboard focus to the active Sort_Option in the Sort_Menu.
4. WHEN the user activates the Filter_Button while the Filter_Menu is closed, THE Companies_Sidebar SHALL open the Filter_Menu and move keyboard focus to the first Company_Filter toggle in the Filter_Menu.
5. WHEN the user presses the Escape key while the Sort_Menu or the Filter_Menu is open, THE Companies_Sidebar SHALL close that menu and return keyboard focus to the button that opened the menu.
6. WHEN the user clicks outside an open Sort_Menu or Filter_Menu and outside the button that opened it, THE Companies_Sidebar SHALL close that menu without changing the Company_View_State.
7. THE Sort_Button and the Filter_Button SHALL each expose an accessible name ("Sort companies" for the Sort_Button and "Filter companies" for the Filter_Button) and an `aria-expanded` value that is true while the button's menu is open and false while it is closed.
8. THE Sort_Button, the Filter_Button, and every option and action in the Sort_Menu and the Filter_Menu SHALL be operable with the keyboard alone, using Tab and Shift+Tab to move focus and Enter or Space to activate.
9. WHILE the Company_Data_Service has returned zero companies, THE Companies_Sidebar SHALL hide the Sort_Button, the Filter_Button, and the Company_Search input.
10. WHEN the user activates the Sort_Button or the Filter_Button while that button's own menu is open and the other menu is closed, THE Companies_Sidebar SHALL close that button's own menu and keep keyboard focus on the activated button.
11. WHEN the user activates the Sort_Button or the Filter_Button while that button's own menu is closed and the other menu is open, THE Companies_Sidebar SHALL close the other menu and open the activated button's menu, so that at most one of the Sort_Menu and the Filter_Menu is open at any time.

### Requirement 2: Sort options

**User Story:** As a job seeker, I want to sort companies by name, open positions, tracked jobs, or date added, so that the companies I care about are easy to find.

#### Acceptance Criteria

1. THE Sort_Menu SHALL list exactly five Sort_Options in this order, each with this visible label: `nameAsc` ("Name A–Z"), `nameDesc` ("Name Z–A"), `openPositionsDesc` ("Most open positions"), `trackedJobsDesc` ("Most tracked jobs"), and `recentlyAdded` ("Recently added").
2. THE Sort_Menu SHALL mark exactly one Sort_Option, the active one, as selected, both with a visible checkmark next to its label and with a programmatic selected state that assistive technologies can read. No other Sort_Option shall be marked.
3. WHEN the user selects a Sort_Option in the Sort_Menu, THE Companies_Sidebar SHALL make that Sort_Option active, reorder the Company_List by it before the next rendered frame without reloading data from the Company_Data_Service, close the Sort_Menu, and return keyboard focus to the Sort_Button.
4. WHEN the user selects the Sort_Option that is already active, THE Companies_Sidebar SHALL leave the Company_List order unchanged and close the Sort_Menu.
5. WHILE the active Sort_Option is `nameAsc`, THE Company_List SHALL order companies by Name_Order. For example, "Abby Care" comes before "AKASA", and "AKASA" comes before "beta".
6. WHILE the active Sort_Option is `nameDesc`, THE Company_List SHALL order companies in the exact reverse of Name_Order, including the reversed `id` tie-break.
7. WHILE the active Sort_Option is `openPositionsDesc`, THE Company_List SHALL order companies by Open_Position_Count from highest to lowest, and SHALL use Name_Order for companies with equal Open_Position_Count, including companies with an Open_Position_Count of 0.
8. WHILE the active Sort_Option is `trackedJobsDesc`, THE Company_List SHALL order companies by Tracked_Job_Count from highest to lowest, and SHALL use Name_Order for companies with equal Tracked_Job_Count, including companies with a Tracked_Job_Count of 0.
9. WHILE the active Sort_Option is `recentlyAdded`, THE Company_List SHALL order companies by `createdAt` from newest to oldest, and SHALL use Name_Order for companies with equal `createdAt`.
10. IF the Preference_Store holds no stored Sort_Option, THEN THE Companies_Sidebar SHALL use `nameAsc` as the active Sort_Option.
11. WHILE the active Sort_Option is not `nameAsc`, THE Sort_Button SHALL display an active indicator, such as a dot or a distinct highlighted style, that is absent when the active Sort_Option is `nameAsc`. THE Sort_Button SHALL also include the active Sort_Option's label in its accessible description.
12. FOR ALL lists of 0 to 10,000 companies and every Sort_Option, THE Company_List SHALL contain the same set of companies, with the same count, before and after sorting. Sorting changes order only.
13. FOR ALL lists of companies and every Sort_Option, sorting an already-sorted list by the same Sort_Option SHALL produce the identical order (idempotence). Sorting the same input list twice SHALL also produce the identical order (determinism), whatever order the Company_Data_Service returned the companies in.
14. WHILE the active Sort_Option is `openPositionsDesc` or `trackedJobsDesc`, THE Company_List SHALL show next to each company name the integer count used for sorting (Open_Position_Count or Tracked_Job_Count), including 0. WHILE any other Sort_Option is active, THE Company_List SHALL show no sort count.
15. WHEN the Jobs page reloads its sidebar data from the Company_Data_Service, THE Company_List SHALL keep the active Sort_Option and reorder the reloaded companies by it.

### Requirement 3: Filter options

**User Story:** As a job seeker, I want to toggle filters such as "watched" or "has open positions", so that the list shows only the companies relevant to what I'm doing right now.

#### Acceptance Criteria

1. THE Filter_Menu SHALL list exactly the four Company_Filters `watched`, `hasOpenPositions`, `hasTrackedJobs`, and `needsAttention` as independent toggles, each with a visible text label and a checked state that is both shown visually and exposed to assistive technologies as checked or unchecked.
2. WHEN the user toggles a Company_Filter in the Filter_Menu, THE Companies_Sidebar SHALL switch that Company_Filter from inactive to active or from active to inactive without changing any other Company_Filter, update the visible companies in the Company_List within 100 milliseconds, keep the Filter_Menu open, and keep keyboard focus on the toggled option.
3. WHILE one or more Company_Filters are active, THE Company_List SHALL show only companies that satisfy every active Company_Filter (logical conjunction), where each Company_Filter is evaluated against the most recent `CompanyRow` data returned by the Company_Data_Service using the definitions in the Glossary (a company with zero watches does not satisfy `watched`; a company with zero watches with `consecutiveSyncFailures` greater than 0 and zero pending careers-page reviews does not satisfy `needsAttention`).
4. WHILE zero Company_Filters are active, THE Company_List SHALL apply no Company_Filter restriction.
5. WHILE one or more Company_Filters are active, THE Filter_Button SHALL display a badge whose number equals the count of active Company_Filters (1 to 4), and WHILE zero Company_Filters are active, THE Filter_Button SHALL display no badge.
6. THE Filter_Menu SHALL include a "Clear filters" action positioned after the four Company_Filter toggles.
7. FOR ALL lists of companies and every set of active Company_Filters, THE Company_List SHALL show a subset of the companies that would be shown with zero Company_Filters active.
8. FOR ALL lists of companies and any two sets of active Company_Filters that contain the same filters selected in a different order, THE Company_List SHALL show the identical set of companies (confluence).
9. WHEN the user activates "Clear filters", THE Companies_Sidebar SHALL deactivate every Company_Filter, keep the active Sort_Option and the Company_Search text unchanged, and keep the Filter_Menu open.
10. WHILE zero Company_Filters are active, THE Filter_Menu SHALL render the "Clear filters" action as disabled, and activating it SHALL change no state.
11. WHEN the Jobs page reloads its sidebar data from the Company_Data_Service, THE Companies_Sidebar SHALL keep the same set of active Company_Filters and re-evaluate them against the reloaded `CompanyRow` data.

### Requirement 4: Per-company tracked job count

**User Story:** As a job seeker, I want the sidebar to know how many jobs I'm tracking at each company, so that I can sort and filter by my own pipeline.

#### Acceptance Criteria

1. THE Company_Data_Service SHALL include a `trackedJobCount` value in every returned `CompanyRow`, as a non-negative integer equal to that company's Tracked_Job_Count.
2. THE Company_Data_Service SHALL count toward Tracked_Job_Count every job linked to the company whose `status` is not `archived`, from any job source, including jobs whose `status` is `closed`, and regardless of the job's `posting_state` or favorite flag.
3. IF a company has zero linked jobs, or only linked jobs whose `status` is `archived`, THEN THE Company_Data_Service SHALL return a `trackedJobCount` of 0 for that company.
4. THE Company_Data_Service SHALL count each job toward the Tracked_Job_Count of at most one company, and SHALL count no job that is not linked to a company, so that the sum of `trackedJobCount` across all returned rows is less than or equal to the total number of non-`archived` jobs in the database.
5. WHEN a job is added, permanently deleted, archived, unarchived, or has its `status` changed through the GUI or the CLI, and the Jobs page then reloads its sidebar data from the Company_Data_Service, THE Companies_Sidebar SHALL use the reloaded `trackedJobCount` values for the `trackedJobsDesc` Sort_Option, the `hasTrackedJobs` Company_Filter, and the displayed counts, with no value retained from before the reload.
6. THE Company_Data_Service SHALL return the same set of companies, in the same order, with `company`, `openPositionCount`, `watches`, and `reviews` values identical to those returned before this feature for the same database state.
7. IF the Company_Data_Service fails while computing Tracked_Job_Count values, THEN THE Company_Data_Service SHALL return an error to the caller instead of a partial list or rows missing `trackedJobCount`, and SHALL leave the database unchanged.

### Requirement 5: Interaction with search and job selection

**User Story:** As a job seeker, I want search, sort, and filters to work together, so that the list reflects every refinement I've applied.

#### Acceptance Criteria

1. WHILE the Company_Search contains a non-empty trimmed query and one or more Company_Filters are active, THE Company_List SHALL show only companies whose name contains the trimmed query (compared case-insensitively) and that satisfy every active Company_Filter.
2. WHILE the Company_Search query is empty or contains only whitespace, THE Company_List SHALL apply no Company_Search restriction and SHALL show the same companies as it would with an empty Company_Search.
3. THE Company_List SHALL order the companies that remain after the Company_Search and the Company_Filters are applied according to the active Sort_Option, with every remaining company appearing exactly once.
4. FOR ALL lists of companies, search queries, Company_Filter sets, and Sort_Options, applying the Company_Search before the Company_Filters SHALL produce the same visible companies in the same order as applying the Company_Filters before the Company_Search.
5. WHEN the user changes the Sort_Option or toggles a Company_Filter, THE Companies_Sidebar SHALL keep the Company_Search text unchanged, including leading and trailing whitespace.
6. WHEN the user changes the Company_Search text, THE Companies_Sidebar SHALL keep the active Sort_Option and the set of active Company_Filters unchanged.
7. WHEN the user changes the Sort_Option, toggles a Company_Filter, edits the Company_Search text, or the Company_View_State is restored on mount, THE Jobs page SHALL keep every URL query parameter, including `companyId`, unchanged in name and value.
8. WHEN the user activates a company link in the Company_List, THE Jobs page SHALL set the `companyId` URL parameter to that company's `id`, and THE Companies_Sidebar SHALL keep the Company_Search text, the active Sort_Option, and the active Company_Filters unchanged.
9. WHILE one or more Company_Filters are active or the Company_Search contains a non-empty trimmed query, THE Companies_Sidebar SHALL display a visible count in the form "N of M companies", where N is the number of companies currently shown in the Company_List and M is the total number of companies returned by the Company_Data_Service, and THE Companies_Sidebar SHALL update N within the same render as the change to the Company_Search text or Company_Filters.
10. WHILE zero Company_Filters are active and the Company_Search query is empty after trimming, THE Companies_Sidebar SHALL hide the "N of M companies" count.
11. WHILE the company identified by the `companyId` URL parameter is shown in the Company_List, THE Company_List SHALL mark that company, and only that company, as active.
12. IF the company identified by the `companyId` URL parameter is excluded from the Company_List by the Company_Search or the active Company_Filters, THEN THE Jobs page SHALL keep the `companyId` URL parameter and the jobs list scoped to that company unchanged, and THE Company_List SHALL mark no company as active.

### Requirement 6: Persistence of sort and filter choices

**User Story:** As a job seeker, I want my sort and filter choices remembered, so that the sidebar looks the same when I come back.

#### Acceptance Criteria

1. WHEN the user selects a Sort_Option, toggles a Company_Filter, activates "Clear filters", or activates the "Reset" action, THE Companies_Sidebar SHALL write the resulting Company_View_State to the Preference_Store before the next user interaction is processed, so that closing and relaunching the app at any point after the change restores that Company_View_State.
2. WHEN the Jobs page mounts, THE Companies_Sidebar SHALL restore the Company_View_State from the Preference_Store and apply it to the first render of the Company_List, so that the Company_List is never shown in a different sort order or filter set than the restored Company_View_State.
3. IF the Preference_Store holds no Company_View_State, THEN THE Companies_Sidebar SHALL use `nameAsc` as the active Sort_Option and zero active Company_Filters.
4. FOR ALL valid Company_View_State values, writing to the Preference_Store and then reading back SHALL produce an equivalent Company_View_State, where equivalent means the same Sort_Option and the same set of active Company_Filters regardless of the order in which the filters were activated (round trip).
5. IF the Preference_Store holds a Company_View_State that cannot be parsed, or that has a Sort_Option or Company_Filter outside the defined sets, THEN THE Companies_Sidebar SHALL ignore the invalid values, use `nameAsc` for a missing or invalid Sort_Option, keep a valid stored Sort_Option, keep only the valid Company_Filters with each filter counted at most once, and render the Company_List without displaying an error.
6. IF reading from or writing to the Preference_Store throws an error, THEN THE Companies_Sidebar SHALL keep the in-memory Company_View_State the user selected (without reverting it), render the Company_List without displaying an error, and, for a failed read, use `nameAsc` with zero active Company_Filters.
7. THE Companies_Sidebar SHALL keep the Company_Search text in memory only, SHALL NOT write the Company_Search text to the Preference_Store, and SHALL show an empty Company_Search when the Jobs page mounts.

### Requirement 7: Empty states

**User Story:** As a job seeker, I want clear messages when no companies are shown, so that I know why and how to get them back.

#### Acceptance Criteria

1. WHILE the Company_Data_Service has completed successfully and returned zero companies, THE Companies_Sidebar SHALL display the message "No companies yet." in place of the Company_List, render zero company links, and render no "Reset" action.
2. WHILE the Company_Data_Service has returned one or more companies, the Company_Search contains a non-empty trimmed query, and the Company_Search and the active Company_Filters (zero or more) together leave zero companies, THE Companies_Sidebar SHALL display the message "No matching companies." and a "Reset" action in place of the Company_List, render zero company links, and keep the Company_Search, Sort_Button, and Filter_Button visible.
3. WHEN the user activates the "Reset" action, THE Companies_Sidebar SHALL, in a single update, clear the Company_Search text to an empty string, deactivate every Company_Filter, write the resulting Company_View_State (with zero active Company_Filters) to the Preference_Store, keep the active Sort_Option unchanged, keep the Jobs page URL query parameters (including `companyId`) unchanged, show all M companies returned by the Company_Data_Service ordered by the active Sort_Option, and move keyboard focus to the Company_Search input.
4. WHILE the Company_Data_Service has returned one or more companies, the Company_Search is empty or contains only whitespace, and one or more active Company_Filters leave zero companies, THE Companies_Sidebar SHALL display the message "No companies match the selected filters." and a "Reset" action in place of the Company_List, render zero company links, and keep the Company_Search, Sort_Button, and Filter_Button visible.
5. THE "Reset" action SHALL be a focusable control with the accessible name "Reset", reachable with Tab and activatable with Enter or Space.
6. WHILE the Company_Data_Service request has not yet completed, THE Companies_Sidebar SHALL display none of the messages "No companies yet.", "No matching companies.", or "No companies match the selected filters.".
7. IF the Company_Data_Service request fails, THEN THE Companies_Sidebar SHALL display an error message indicating that companies could not be loaded instead of "No companies yet.", SHALL render zero company links, and SHALL leave the Company_View_State in the Preference_Store unchanged.
