# Requirements Document

## Introduction

The Jobs page has a "Companies" sidebar (`desktop/src/components/CompaniesSidebar.tsx`, logic in `desktop/src/lib/company-sidebar.ts`) built by the `companies-sidebar-sort-filter` spec. It has a Sort_Button that opens a Sort_Menu with five Sort_Options (`nameAsc`, `nameDesc`, `openPositionsDesc`, `trackedJobsDesc`, `recentlyAdded`) and a Filter_Button with four Company_Filters (`watched`, `hasOpenPositions`, `hasTrackedJobs`, `needsAttention`). Sorting and filtering run in the desktop UI over the `CompanyRow` values returned by the `list_companies` Tauri command (`list_companies_with_watches` in `src-tauri/src/companies.rs`). Today each `CompanyRow` carries only `openPositionCount` and `trackedJobCount` as job aggregates.

The user asked to "enhance the companies tab filtering" with: most wishlist positions, most applied positions (applied, rejected, or otherwise past wishlist), the "least" versions of both, most recently updated job, and most recently applied job. All of these describe an **ordering** of the company list, so this spec adds them as new Sort_Options, not as Company_Filters. A Sort_Option reorders the list and keeps every company. A Company_Filter hides companies. To cover the "filtering" wording, this spec also adds two Company_Filters that pair with the new counts (marked as suggested additions).

The Company_Data_Service computes the new per-company aggregates in Rust and returns them on every `CompanyRow`. The UI sorts on them with the existing deterministic, Name_Order tie-broken comparator.

Items marked **[Suggested addition]** were not in the user's request. They are proposed because they reuse the same aggregates at low cost. The user can drop any of them during review.

Out of scope: the `/companies` management page (`desktop/src/components/CompaniesClient.tsx`), the jobs-list sort controls, CLI output changes, response-rate or rejection-rate sorting (small per-company sample sizes make rates misleading), and any Node or Next server. Data reaches the UI only through Tauri `invoke` (`desktop/src/lib/api.ts`).

## Glossary

- **Companies_Sidebar**: The "Companies" card in the Jobs page sidebar, including the Company_Search input, the Sort_Button, the Sort_Menu, the Filter_Button, the Filter_Menu, and the Company_List.
- **Company_List**: The scrollable list of company links in the Companies_Sidebar.
- **Sort_Menu**: The menu opened from the Sort_Button that lists the Sort_Options and marks the active one.
- **Sort_Option**: One entry in the Sort_Menu that defines an ordering of the Company_List. The full set is listed in Requirement 2.
- **Filter_Menu**: The menu opened from the Filter_Button that lists the Company_Filters as independent toggles.
- **Company_Filter**: One toggle in the Filter_Menu that hides companies that do not satisfy its predicate.
- **Company_Data_Service**: The Rust function `list_companies_with_watches` in `src-tauri/src/companies.rs`, exposed through the `list_companies` Tauri command.
- **Company_View_State**: The active Sort_Option plus the set of active Company_Filters, persisted in the Preference_Store under the key `jobTracker.companiesSidebar.view.v1`.
- **Preference_Store**: Browser `localStorage` in the desktop WebView.
- **Name_Order**: Ascending order by company name using locale-aware, case-insensitive comparison, with the company `id` (compared by code unit) breaking ties. This is the existing `compareByNameOrder`.
- **Pipeline_Job**: A job linked to the company whose `status` is not `archived`, whose `is_new_from_watch` is 0, and whose `watch_disposition` is not `dismissed`. This matches the jobs counted by the existing pipeline stats (`get_pipeline_counts`), minus archived jobs. Untriaged and dismissed watch discoveries are excluded.
- **Wishlist_Job**: A Pipeline_Job whose `status` is `wishlist`.
- **Applied_Job**: A Pipeline_Job that has moved past wishlist: its `status` is `applied`, `interviewing`, `offer`, `rejected`, or `withdrawn`, or its `status` is `closed` and its `applied_at` is not null.
- **Interviewing_Job**: A Pipeline_Job whose `status` is `interviewing` or `offer`.
- **Wishlist_Job_Count**: For one company, the number of Wishlist_Jobs.
- **Applied_Job_Count**: For one company, the number of Applied_Jobs.
- **Interviewing_Job_Count**: For one company, the number of Interviewing_Jobs.
- **User_Activity_Event**: A row in `job_events` whose `type` is not `posting_state_changed` and not `discovered_from_watch`. Background ATS syncs and posting checks write only those two event types, so User_Activity_Events reflect changes made through the GUI, the CLI, or CSV import (for example `created`, `status_changed`, `note_added`, `favorited`, `company_changed`, `csv_note`).
- **Last_Activity_At**: For one company, the latest Valid_Timestamp `occurred_at` across all User_Activity_Events of the company's Pipeline_Jobs (selected as defined in Requirement 1), or null when there are none. The job `updated_at` column is not used because background syncs rewrite it every hour.
- **Last_Applied_At**: For one company, the latest Valid_Timestamp `applied_at` across the company's Pipeline_Jobs (selected as defined in Requirement 1), or null when there are none.
- **Wishlist_Ease_Average**: For one company, the arithmetic mean of `application_ease` across the company's Wishlist_Jobs whose `application_ease` is an integer from 1 to 5 inclusive, or null when there are none.
- **Valid_Timestamp**: A timestamp string (an `occurred_at`, `applied_at`, Last_Activity_At, or Last_Applied_At value) that parses as a date (ISO 8601 date or date-time). Unparseable values are treated as null.
- **Sort_Annotation**: The short value shown next to each company name in the Company_List that explains the active Sort_Option (a count, a date, or an average).

## Requirements

### Requirement 1: New per-company aggregates

**User Story:** As a job seeker, I want the companies list to know how many roles I have at each stage and when I last touched each company, so that I can sort by my own pipeline.

#### Acceptance Criteria

1. THE Company_Data_Service SHALL include in every returned `CompanyRow` a `wishlistJobCount` equal to the company's Wishlist_Job_Count, an `appliedJobCount` equal to the company's Applied_Job_Count, and an `interviewingJobCount` equal to the company's Interviewing_Job_Count, each as an integer greater than or equal to 0.
2. THE Company_Data_Service SHALL include in every returned `CompanyRow` a `lastActivityAt` value equal to the company's Last_Activity_At and a `lastAppliedAt` value equal to the company's Last_Applied_At, each returned as the stored timestamp string exactly as written in the database (not reformatted), or null.
3. WHEN the Company_Data_Service determines the latest value for Last_Activity_At or Last_Applied_At, THE Company_Data_Service SHALL consider only Valid_Timestamps, compare them by the instant they represent (a date-only value compares as 00:00:00 UTC on that date), and return null when the company has no Valid_Timestamp for that field.
4. IF two or more Valid_Timestamps tie for the latest instant within one company's Last_Activity_At or Last_Applied_At, THEN THE Company_Data_Service SHALL return the tied stored string that is greatest by code-unit comparison, so that repeated calls on the same database state return the same string.
5. THE Company_Data_Service SHALL include in every returned `CompanyRow` a `wishlistEaseAverage` value equal to the company's Wishlist_Ease_Average. The value SHALL be unrounded and within 0.001 of the exact mean, SHALL be computed only from `application_ease` values that are integers from 1 to 5 inclusive, SHALL therefore lie between 1 and 5 inclusive, and SHALL be null when the company has no Wishlist_Job with such a value.
6. IF a company has zero Pipeline_Jobs, THEN THE Company_Data_Service SHALL return 0 for `wishlistJobCount`, `appliedJobCount`, and `interviewingJobCount`, and null for `lastActivityAt`, `lastAppliedAt`, and `wishlistEaseAverage`.
7. THE Company_Data_Service SHALL count each Pipeline_Job toward at most one of Wishlist_Job_Count and Applied_Job_Count, so that for every company `wishlistJobCount + appliedJobCount` is less than or equal to the number of the company's Pipeline_Jobs.
8. THE Company_Data_Service SHALL count every Interviewing_Job as an Applied_Job, so that for every company `interviewingJobCount` is less than or equal to `appliedJobCount`.
9. THE Company_Data_Service SHALL return the same set of companies, in the same order, with `company`, `watches`, `reviews`, `openPositionCount`, and `trackedJobCount` values identical to those returned before this feature for the same database state.
10. IF the Company_Data_Service fails while computing any new aggregate, THEN THE Company_Data_Service SHALL return an error to the caller instead of a partial list or rows missing a new field, and SHALL leave the database unchanged.
11. WHEN a job is added, deleted, archived, unarchived, has its `status`, `applied_at`, or `application_ease` changed, or receives a note through the GUI, the CLI, or CSV import, and the Jobs page then reloads its sidebar data, THE Companies_Sidebar SHALL sort, filter, and annotate using only the reloaded aggregate values, with no value retained from before the reload.
12. WHEN a background ATS sync or posting check updates a job without writing a User_Activity_Event, THE Company_Data_Service SHALL return the same `lastActivityAt` for that job's company as before the sync.

### Requirement 2: Sort_Option set and menu layout

**User Story:** As a job seeker, I want the new orderings listed next to the existing ones, so that I can pick any of them from one menu.

#### Acceptance Criteria

1. THE Sort_Menu SHALL list exactly these 13 Sort_Options, top to bottom in this order, with these visible labels and no other entries:
   - `nameAsc` ("Name A–Z") (existing)
   - `nameDesc` ("Name Z–A") (existing)
   - `recentlyAdded` ("Recently added company") (existing; label changed from "Recently added" to distinguish it from job-based dates)
   - `openPositionsDesc` ("Most open positions") (existing)
   - `trackedJobsDesc` ("Most tracked jobs") (existing)
   - `wishlistJobsDesc` ("Most wishlist jobs")
   - `wishlistJobsAsc` ("Fewest wishlist jobs")
   - `appliedJobsDesc` ("Most applied jobs")
   - `appliedJobsAsc` ("Fewest applied jobs")
   - `interviewingJobsDesc` ("Most interviews") **[Suggested addition]**
   - `recentActivity` ("Recently updated job")
   - `recentlyApplied` ("Recently applied")
   - `easiestWishlist` ("Easiest wishlist to apply") **[Suggested addition]**
2. THE Sort_Menu SHALL render exactly two group separators, each exposed to assistive technologies with the separator role and not focusable or selectable: one between `recentlyAdded` and `openPositionsDesc`, and one between `interviewingJobsDesc` and `recentActivity`, so that the count group contains `openPositionsDesc`, `trackedJobsDesc`, `wishlistJobsDesc`, `wishlistJobsAsc`, `appliedJobsDesc`, `appliedJobsAsc`, and `interviewingJobsDesc` in the order given in criterion 1.
3. THE Sort_Menu SHALL apply the existing selection, keyboard navigation, focus, and ARIA behavior from the `companies-sidebar-sort-filter` spec to all 13 Sort_Options, with keyboard navigation skipping the group separators.
4. THE Sort_Menu SHALL mark exactly one Sort_Option as selected at any time, and that Sort_Option SHALL equal the active Sort_Option in the Company_View_State.
5. WHILE the Sort_Menu is open and its full height exceeds the vertical space available in the window, THE Sort_Menu SHALL scroll within its own bounds, keeping every Sort_Option reachable by mouse scrolling and keeping the keyboard-focused Sort_Option fully inside the visible area of the Sort_Menu.
6. IF the Preference_Store holds no stored Sort_Option, THEN THE Companies_Sidebar SHALL use `nameAsc` as the active Sort_Option.
7. IF the Preference_Store holds a stored Sort_Option value that is not one of the 13 Sort_Option identifiers in criterion 1, THEN THE Companies_Sidebar SHALL use `nameAsc` as the active Sort_Option, keep the stored active Company_Filters unchanged, and show no error to the user.
8. WHEN the user selects a Sort_Option from the Sort_Menu, THE Companies_Sidebar SHALL store that Sort_Option in the Company_View_State and restore it as the active Sort_Option after the app is reloaded.

### Requirement 3: Count-based ordering

**User Story:** As a job seeker, I want to rank companies by how many wishlist or applied roles I have there, in either direction, so that I can see where I'm concentrated and where I've barely engaged.

#### Acceptance Criteria

1. WHILE the active Sort_Option is `wishlistJobsDesc`, THE Company_List SHALL order companies by Wishlist_Job_Count from highest to lowest, using Name_Order for companies with equal Wishlist_Job_Count, so that companies with a Wishlist_Job_Count of 0 appear after all companies with a Wishlist_Job_Count of 1 or more.
2. WHILE the active Sort_Option is `appliedJobsDesc`, THE Company_List SHALL order companies by Applied_Job_Count from highest to lowest, using Name_Order for companies with equal Applied_Job_Count, so that companies with an Applied_Job_Count of 0 appear after all companies with an Applied_Job_Count of 1 or more.
3. WHILE the active Sort_Option is `interviewingJobsDesc`, THE Company_List SHALL order companies by Interviewing_Job_Count from highest to lowest, using Name_Order for companies with equal Interviewing_Job_Count, so that companies with an Interviewing_Job_Count of 0 appear after all companies with an Interviewing_Job_Count of 1 or more.
4. WHILE the active Sort_Option is `wishlistJobsAsc`, THE Company_List SHALL list companies whose Wishlist_Job_Count is 1 or more in order from lowest to highest Wishlist_Job_Count, using Name_Order for companies with equal Wishlist_Job_Count, followed by all companies whose Wishlist_Job_Count is 0 in Name_Order.
5. WHILE the active Sort_Option is `appliedJobsAsc`, THE Company_List SHALL list companies whose Applied_Job_Count is 1 or more in order from lowest to highest Applied_Job_Count, using Name_Order for companies with equal Applied_Job_Count, followed by all companies whose Applied_Job_Count is 0 in Name_Order.
6. THE Company_List SHALL place any two companies whose Wishlist_Job_Counts differ and are both 1 or more in opposite relative order under `wishlistJobsAsc` and `wishlistJobsDesc`, and SHALL place any two companies with equal Wishlist_Job_Count in Name_Order under both Sort_Options.
7. THE Company_List SHALL place any two companies whose Applied_Job_Counts differ and are both 1 or more in opposite relative order under `appliedJobsAsc` and `appliedJobsDesc`, and SHALL place any two companies with equal Applied_Job_Count in Name_Order under both Sort_Options.
8. WHILE the active Sort_Option is `wishlistJobsDesc`, `appliedJobsDesc`, `interviewingJobsDesc`, `wishlistJobsAsc`, or `appliedJobsAsc`, THE Company_List SHALL include every company that matches the current Company_Search text and all active Company_Filters, including companies with no Pipeline_Jobs, which have a count of 0.
9. WHILE the active Sort_Option is `wishlistJobsDesc`, `appliedJobsDesc`, `interviewingJobsDesc`, `wishlistJobsAsc`, or `appliedJobsAsc`, THE Company_List SHALL show each company that matches the current Company_Search text and all active Company_Filters exactly once.
10. IF a `CompanyRow` returned by the Company_Data_Service has a Wishlist_Job_Count, Applied_Job_Count, or Interviewing_Job_Count that is missing, null, negative, or not a whole number, THEN THE Company_List SHALL treat that count as 0 for ordering, SHALL keep the company in the list, and SHALL keep showing the other companies without an error state.
11. WHEN the Companies_Sidebar receives a new `list_companies` result in which a company's Wishlist_Job_Count, Applied_Job_Count, or Interviewing_Job_Count has changed, THE Company_List SHALL reorder under the active count-based Sort_Option using the new counts, without the user reselecting the Sort_Option.

### Requirement 4: Date-based ordering

**User Story:** As a job seeker, I want to rank companies by when I last worked on or applied to a role there, so that I can pick up where I left off or spot companies I've neglected.

#### Acceptance Criteria

1. WHILE the active Sort_Option is `recentActivity`, THE Company_List SHALL list every company whose Last_Activity_At is a Valid_Timestamp first, ordered from newest instant to oldest instant, with Name_Order deciding the order of companies whose instants are equal.
2. WHILE the active Sort_Option is `recentActivity`, THE Company_List SHALL list every company whose Last_Activity_At is null or not a Valid_Timestamp after all companies that have a Valid_Timestamp Last_Activity_At, ordered among themselves by Name_Order.
3. WHILE the active Sort_Option is `recentlyApplied`, THE Company_List SHALL list every company whose Last_Applied_At is a Valid_Timestamp first, ordered from newest instant to oldest instant, with Name_Order deciding the order of companies whose instants are equal.
4. WHILE the active Sort_Option is `recentlyApplied`, THE Company_List SHALL list every company whose Last_Applied_At is null or not a Valid_Timestamp after all companies that have a Valid_Timestamp Last_Applied_At, ordered among themselves by Name_Order.
5. THE Companies_Sidebar SHALL compare Last_Activity_At and Last_Applied_At values as instants. It SHALL treat a date-only value (`YYYY-MM-DD`) as 00:00:00 UTC on that day and a date-time value without a UTC offset as UTC. Under this rule, `2026-10-01` orders before `2026-10-01T00:00:01Z` and equal to `2026-10-01T00:00:00Z`.
6. WHILE the active Sort_Option is `recentlyAdded`, THE Company_List SHALL keep the existing ordering by company `createdAt` from newest to oldest, with Name_Order for equal values.
7. WHILE the active Sort_Option is `recentActivity` or `recentlyApplied`, THE Company_List SHALL show a Sort_Annotation next to each company with a Valid_Timestamp for the active Sort_Option's field. The Sort_Annotation SHALL show that value's calendar date as defined in Requirement 6, and two companies with the same calendar date SHALL show identical Sort_Annotations.
8. IF the active Sort_Option is `recentActivity` or `recentlyApplied` and a company's value for that Sort_Option's field is null or not a Valid_Timestamp, THEN THE Company_List SHALL show no date in that company's Sort_Annotation and SHALL still list the company without raising an error.
9. WHEN the `list_companies` Tauri command returns updated CompanyRow values while `recentActivity` or `recentlyApplied` is the active Sort_Option, THE Company_List SHALL re-order the companies and update their Sort_Annotations from the new Last_Activity_At or Last_Applied_At values, without changing the active Sort_Option or the active Company_Filters.

### Requirement 5: Application ease ordering [Suggested addition]

**User Story:** As a job seeker, I want to rank companies by how easy their wishlist roles are to apply to, so that I can batch quick applications.

#### Acceptance Criteria

1. WHILE the active Sort_Option is `easiestWishlist`, THE Company_List SHALL first list the companies whose Wishlist_Ease_Average is non-null, ordered by Wishlist_Ease_Average from highest (5) to lowest (1). Companies with equal averages SHALL be ordered by Wishlist_Job_Count from highest to lowest, then by Name_Order. THE Company_List SHALL then list every company whose Wishlist_Ease_Average is null, in Name_Order.
2. THE Companies_Sidebar SHALL compare Wishlist_Ease_Average values exactly as the Company_Data_Service returns them, with no rounding before comparison. For example, 4.67 sorts above 4.6, even though both display as Sort_Annotations rounded to one decimal place.
3. WHILE the active Sort_Option is `easiestWishlist`, THE Company_List SHALL use the company's full Wishlist_Job_Count, including Wishlist_Jobs whose `application_ease` is null, as the tie-breaker for equal Wishlist_Ease_Average values.
4. IF a company has one or more Wishlist_Jobs and none of them has a non-null `application_ease`, or the company has zero Wishlist_Jobs, THEN THE Company_List SHALL place that company in the null-average group, which follows all companies with a non-null Wishlist_Ease_Average, in Name_Order.
5. IF the Wishlist_Ease_Average for a company is not a finite number from 1 to 5 inclusive, THEN THE Companies_Sidebar SHALL treat that value as null for ordering and annotation, and SHALL still show the company in the Company_List.
6. WHILE the active Sort_Option is `easiestWishlist` and one or more Company_Filters are active, THE Company_List SHALL apply the ordering in criterion 1 only to the companies that pass every active Company_Filter, and SHALL show no company that a Company_Filter hides.

### Requirement 6: Sort annotations

**User Story:** As a job seeker, I want to see the value each company is sorted by, so that I understand why it is in that position.

#### Acceptance Criteria

1. WHILE the active Sort_Option is `wishlistJobsDesc` or `wishlistJobsAsc`, THE Company_List SHALL show each company's Wishlist_Job_Count as a whole number Sort_Annotation, including companies whose count is 0.
2. WHILE the active Sort_Option is `appliedJobsDesc` or `appliedJobsAsc`, THE Company_List SHALL show each company's Applied_Job_Count as a whole number Sort_Annotation, including companies whose count is 0.
3. WHILE the active Sort_Option is `interviewingJobsDesc`, THE Company_List SHALL show each company's Interviewing_Job_Count as a whole number Sort_Annotation, including companies whose count is 0.
4. WHILE the active Sort_Option is `recentActivity` or `recentlyApplied`, THE Company_List SHALL show the sorted Valid_Timestamp (Last_Activity_At for `recentActivity`, Last_Applied_At for `recentlyApplied`) as a short date in the user's locale as the Sort_Annotation, containing the abbreviated month and the day of month, plus the four-digit year only when that year differs from the current year in the user's local time zone. A date-only value SHALL display as that calendar date with no time-zone shift, and a date-time value SHALL display as its calendar date in the user's local time zone.
5. WHILE the active Sort_Option is `recentActivity` or `recentlyApplied`, THE Company_List SHALL show no Sort_Annotation for a company whose sorted timestamp is null or is not a Valid_Timestamp, and SHALL still list that company.
6. WHILE the active Sort_Option is `easiestWishlist`, THE Company_List SHALL show the Wishlist_Ease_Average rounded to exactly one decimal place, with halves rounded away from zero (for example 4.25 shows as "4.3" and 4 shows as "4.0"), as the Sort_Annotation, and SHALL show no Sort_Annotation for a company whose Wishlist_Ease_Average is null.
7. THE Company_List SHALL give every displayed Sort_Annotation an accessible label that names the measure and the value, using singular wording for a count of 1 and plural wording otherwise, for example "1 wishlist job", "3 wishlist jobs", "0 applied jobs", "Last applied Mar 4", "Last activity Mar 4, 2025", or "Average ease 4.5".
8. WHILE the active Sort_Option is `openPositionsDesc` or `trackedJobsDesc`, THE Company_List SHALL show the same count Sort_Annotation it showed before this feature for that Sort_Option.
9. WHILE the active Sort_Option is `nameAsc`, `nameDesc`, or `recentlyAdded`, THE Company_List SHALL show no Sort_Annotation for any company.
10. WHEN the user selects a different Sort_Option in the Sort_Menu, THE Company_List SHALL replace every Sort_Annotation with the annotation defined for the newly selected Sort_Option in the same render that reorders the list, without requiring a reload of the Jobs page.

### Requirement 7: Pipeline Company_Filters [Suggested addition]

**User Story:** As a job seeker, I want to hide companies where I have no wishlist roles or no applications, so that the "fewest" sorts and the list in general show only companies I'm engaged with.

#### Acceptance Criteria

1. THE Filter_Menu SHALL list six Company_Filters in this order: the existing four (`watched`, `hasOpenPositions`, `hasTrackedJobs`, `needsAttention`) in their current order, then `hasWishlistJobs` labeled "Has wishlist jobs", then `hasAppliedJobs` labeled "Has applied jobs".
2. THE Filter_Menu SHALL give `hasWishlistJobs` and `hasAppliedJobs` the same toggle interaction, keyboard focus order, and ARIA roles and checked-state attributes as the existing Company_Filters. Each new toggle SHALL be inactive by default when no Company_View_State is stored.
3. WHILE `hasWishlistJobs` is active, THE Company_List SHALL show only companies whose Wishlist_Job_Count is greater than or equal to 1, and SHALL hide every company whose Wishlist_Job_Count is 0.
4. WHILE `hasAppliedJobs` is active, THE Company_List SHALL show only companies whose Applied_Job_Count is greater than or equal to 1, and SHALL hide every company whose Applied_Job_Count is 0.
5. WHILE one or more Company_Filters are active, THE Companies_Sidebar SHALL show a company only if it satisfies every active Company_Filter and matches the current Company_Search text (logical AND). For example, with `hasWishlistJobs` and `hasAppliedJobs` both active, only companies with Wishlist_Job_Count ≥ 1 and Applied_Job_Count ≥ 1 appear.
6. WHILE any Company_Filters are active, THE Company_List SHALL preserve strict relative order among the visible companies by placing any visible company A before any visible company B whenever A is ordered strictly before B in the unfiltered list under the active Sort_Option. Tie relationships among visible companies are not required to be preserved after filtering.
7. WHEN the user toggles `hasWishlistJobs` or `hasAppliedJobs` on or off, THE Companies_Sidebar SHALL respond to that toggle event by updating the Company_List, the "N of M companies" count, and the Filter_Button badge in the same render as the toggle, without reloading data from the Company_Data_Service, with the badge showing the number of active Company_Filters after the toggle (from 1 to 6) or hidden when 0 Company_Filters are active after the toggle.
8. WHEN the user activates "Clear filters", THE Companies_Sidebar SHALL deactivate all six Company_Filters, including `hasWishlistJobs` and `hasAppliedJobs`, and SHALL leave the active Sort_Option and the Company_Search text unchanged.
9. IF the active Company_Filters and the Company_Search together exclude every company, THEN THE Companies_Sidebar SHALL show the existing filtered empty state and the "Clear filters" action, and SHALL NOT show the empty state used when no companies exist at all.

### Requirement 8: Ordering correctness for every Sort_Option

**User Story:** As a job seeker, I want every ordering to be stable and predictable, so that the list does not reshuffle between reloads when nothing changed.

#### Acceptance Criteria

1. FOR ALL lists of 0 to 10,000 companies and every Sort_Option listed in Requirement 2, THE Companies_Sidebar SHALL return a sorted list that has the same length as the input and contains each input company `id` exactly as many times as the input does. No company is added, dropped, or duplicated.
2. FOR ALL lists of 0 to 10,000 companies and every Sort_Option listed in Requirement 2, sorting an already-sorted list again by the same Sort_Option SHALL produce the identical sequence of company `id`s (idempotence).
3. FOR ALL lists of 0 to 10,000 companies and every Sort_Option listed in Requirement 2, sorting any permutation of the same input list SHALL produce the identical sequence of company `id`s (determinism). This includes inputs where two or more companies have equal names, equal counts, equal instants, or equal Wishlist_Ease_Average values.
4. FOR ALL lists of companies and every Sort_Option listed in Requirement 2, THE Companies_Sidebar SHALL sort without mutating the input. After sorting, the input list SHALL keep its original element order, and every field of every input `CompanyRow` SHALL keep its original value.
5. FOR ALL lists of companies, THE Company_List SHALL place the following trailing groups after every company outside the group, and SHALL order each trailing group in Name_Order:
   - `wishlistJobsAsc`: companies whose Wishlist_Job_Count is 0.
   - `appliedJobsAsc`: companies whose Applied_Job_Count is 0.
   - `recentActivity`: companies whose Last_Activity_At is null or is not a Valid_Timestamp.
   - `recentlyApplied`: companies whose Last_Applied_At is null or is not a Valid_Timestamp.
   - `easiestWishlist`: companies whose Wishlist_Ease_Average is null.

   No other Sort_Option has a trailing group. For example, under `wishlistJobsDesc` and `appliedJobsDesc`, companies with a count of 0 sort last only because 0 is the lowest count.
6. WHEN the Jobs page reloads its sidebar data from the Company_Data_Service, THE Company_List SHALL keep the active Sort_Option unchanged and SHALL order the reloaded companies by that Sort_Option. Companies added or removed since the previous load SHALL appear or disappear in the positions the Sort_Option assigns to them.
7. WHEN the Jobs page reloads its sidebar data and the Company_Data_Service returns the same companies with the same aggregate values as the previous load, THE Company_List SHALL display the same sequence of company `id`s as before the reload.
8. IF a reload of sidebar data from the Company_Data_Service fails, THEN THE Companies_Sidebar SHALL keep the previously displayed Company_List order, the active Sort_Option, and the active Company_Filters unchanged.

### Requirement 9: Persistence of new choices

**User Story:** As a job seeker, I want the new sort and filter choices remembered, so that the sidebar looks the same when I come back.

#### Acceptance Criteria

1. FOR ALL Company_View_State values that combine any one Sort_Option from Requirement 2 with any subset (including the empty set and the full set) of the Company_Filters from Requirement 7 and the existing Company_Filters (`watched`, `hasOpenPositions`, `hasTrackedJobs`, `needsAttention`), writing to the Preference_Store under the key `jobTracker.companiesSidebar.view.v1` and then reading back SHALL produce a Company_View_State with the same Sort_Option and the same set of active Company_Filters (round trip).
2. FOR ALL Company_View_State values, two states with the same Sort_Option and the same set of active Company_Filters SHALL serialize to identical stored values, regardless of the order in which the Company_Filters were toggled on.
3. WHEN the user selects a Sort_Option in the Sort_Menu or toggles a Company_Filter in the Filter_Menu, THE Companies_Sidebar SHALL write the resulting Company_View_State to the Preference_Store under the existing key `jobTracker.companiesSidebar.view.v1`, without changing the key name.
4. WHEN the Jobs page mounts with a Company_View_State stored before this feature (a stored Sort_Option from the original five and any subset of the original four Company_Filters), THE Companies_Sidebar SHALL restore the stored Sort_Option and Company_Filters unchanged, with no Company_Filter from Requirement 7 active.
5. IF the Preference_Store holds a Sort_Option value outside the set defined in Requirement 2, THEN THE Companies_Sidebar SHALL use `nameAsc` as the active Sort_Option and SHALL NOT display an error.
6. IF the Preference_Store holds Company_Filter values outside the set defined in Requirement 7 plus the existing Company_Filters, or holds a valid Company_Filter more than once, THEN THE Companies_Sidebar SHALL keep each valid Company_Filter active exactly once, discard the invalid values, and SHALL NOT display an error.
7. IF the stored value under `jobTracker.companiesSidebar.view.v1` is missing, is not valid JSON, is not a JSON object, or the Preference_Store is unavailable or throws on read, THEN THE Companies_Sidebar SHALL use the default Company_View_State (`nameAsc` with no active Company_Filters) and SHALL NOT display an error.
8. IF writing the Company_View_State to the Preference_Store fails, THEN THE Companies_Sidebar SHALL keep the user's selected Sort_Option and Company_Filters applied to the Company_List for the current session and SHALL NOT display an error.
9. THE Companies_Sidebar SHALL NOT store the Company_Search text in the Preference_Store.
