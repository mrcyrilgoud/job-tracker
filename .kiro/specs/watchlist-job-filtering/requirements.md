# Requirements Document

## Introduction

Job Tracker watches company ATS boards (Greenhouse, Lever, Ashby) and surfaces newly discovered roles. The existing filtering approach is imprecise (broad substring `LIKE` matching, region maps that ship only in code, and ambiguity such as `ca` meaning California or Canada) and static (no runtime configurability beyond a global comma-separated country/city/keyword string).

This feature replaces that approach with a structured, runtime-configurable filter engine. A normalized Filter_Criteria value is evaluated in Rust against each job's title and location. Criteria are resolved from a two-level hierarchy — a global default and optional per-watch overrides — allowing accurate filtering (word-boundary token matching, explicit include/exclude precedence, remote handling, country disambiguation) and dynamic filtering (edit criteria at runtime per board without recompiling). Region knowledge moves from hardcoded code into a data-driven alias table. Existing legacy settings are migrated once into structured global criteria, and legacy keys and commands are retained for a release to enable safe rollback. The engine fails open on malformed data so users see more rather than losing visibility.

## Glossary

- **Filter_Engine**: The pure evaluation function that decides whether one Filter_Criteria matches one job's title and location, returning a match result and a human-readable reason.
- **Filter_Resolver**: The component that produces the effective Filter_Criteria for a watch by selecting the per-watch override if present, otherwise the global criteria, otherwise match-all.
- **Filter_Criteria**: The versioned, serializable representation of what to include and exclude, comprising title criteria, location criteria, and remote mode.
- **Title_Criteria**: The include list, exclude list, and match mode applied to a job title.
- **Location_Criteria**: The country, include list, exclude list, and match mode applied to a job location.
- **Remote_Mode**: A setting of Any, RemoteOnly, or OnsiteOnly constraining a job by remote status.
- **Match_Mode**: A setting of Word (whole-token boundary matching) or Substring (legacy `LIKE` substring matching).
- **Alias_Table**: The data-driven table mapping region tokens to member tokens, country names to expansion tokens (states, abbreviations, hubs), and the set of tokens indicating remote work.
- **Match_All**: The identity Filter_Criteria that includes every job (all include and exclude lists empty and Remote_Mode of Any).
- **Command_Surface**: The set of Tauri commands exposing get and set for global and per-watch criteria plus a preview dry-run.
- **Filter_Editor**: The frontend UI in Settings (global) and WatchRow (per-watch) for editing structured criteria with a live preview.
- **Migration_Process**: The one-time process that converts legacy settings into structured global criteria and seeds the alias table.
- **App_Settings**: The SQLite settings store holding the global criteria, the alias table, and the retained legacy keys.
- **CSV_Export**: The exported jobs.csv file produced from tracked pipeline jobs, whose row selection is independent of Filter_Criteria.

## Requirements

### Requirement 1: Word-Boundary Title and Location Matching

**User Story:** As a job seeker, I want filter terms to match whole words rather than substrings, so that unrelated roles are not incorrectly included or excluded.

#### Acceptance Criteria

1. WHERE Match_Mode is Word, THE Filter_Engine SHALL match a single-token term only when the term equals a whole token in the normalized text, such that the match is case-insensitive and no partial-token match is reported.
2. WHERE Match_Mode is Word AND a term contains 2 or more words, THE Filter_Engine SHALL report a match only when the term's tokens appear as an ordered, contiguous run of tokens in the normalized text with no intervening tokens.
3. WHERE Match_Mode is Substring, THE Filter_Engine SHALL report a match wherever the normalized term appears as a contiguous substring of the normalized text, regardless of token boundaries.
4. THE Filter_Engine SHALL normalize text before matching by converting all characters to lowercase, removing leading and trailing whitespace, and replacing every run of 1 or more internal whitespace characters with a single space character.
5. THE Filter_Engine SHALL tokenize normalized text into tokens by splitting on whitespace and on each of the punctuation delimiters comma, forward slash, opening parenthesis, closing parenthesis, hyphen, and bullet, excluding empty tokens from the result.
6. IF a term is empty after normalization, THEN THE Filter_Engine SHALL exclude that term from matching and report no match for it while retaining all other terms for evaluation.
7. IF the normalized text contains zero tokens, THEN THE Filter_Engine SHALL report no match for any Word-mode term.

### Requirement 2: Include and Exclude Precedence

**User Story:** As a job seeker, I want exclusions to take priority over inclusions, so that I can reliably suppress roles I never want to see.

#### Acceptance Criteria

1. IF any active exclude term in Title_Criteria matches the job title OR any active exclude term in Location_Criteria matches the job location, THEN THE Filter_Engine SHALL exclude the job regardless of any include terms.
2. THE Filter_Engine SHALL treat a term as matching a field when the field contains the term as a case-insensitive substring after trimming leading and trailing whitespace from both the term and the field.
3. THE Filter_Engine SHALL treat a term as active when, after trimming leading and trailing whitespace, its length is at least 1 character, and SHALL ignore any term that is empty or whitespace-only.
4. WHERE the title include list contains at least one active term, THE Filter_Engine SHALL satisfy the title dimension for a job only if at least one active title include term matches the job title.
5. WHERE the location include list contains at least one active term, THE Filter_Engine SHALL satisfy the location dimension for a job only if at least one active location include term matches the job location.
6. WHERE the title include list contains no active terms, THE Filter_Engine SHALL satisfy the title dimension for every job.
7. WHERE the location include list contains no active terms, THE Filter_Engine SHALL satisfy the location dimension for every job.
8. WHEN a job is not excluded under criterion 1, THE Filter_Engine SHALL include the job only if both the title dimension and the location dimension are satisfied, and SHALL otherwise omit the job.

### Requirement 3: Match-All Behavior

**User Story:** As a job seeker, I want an empty filter to show every role, so that I am never silently hidden from results when I have set no criteria.

#### Acceptance Criteria

1. WHERE Filter_Criteria has all title and location include and exclude lists empty AND Remote_Mode is Any, THE Filter_Engine SHALL include every job.
2. THE Filter_Criteria SHALL provide a Match_All value whose include and exclude lists are empty and whose Remote_Mode is Any.
3. WHEN Filter_Criteria is Match_All, THE Filter_Engine SHALL report the match reason as no criteria applied.

### Requirement 4: Case and Whitespace Invariance

**User Story:** As a job seeker, I want matching to ignore letter case and formatting differences, so that results are consistent regardless of how locations and titles are punctuated.

#### Acceptance Criteria

1. THE Filter_Engine SHALL compare job text and criteria tokens case-insensitively.
2. WHEN two job texts differ only in letter case, THE Filter_Engine SHALL produce the same inclusion result for both.
3. WHEN two locations differ only in surrounding whitespace or delimiter spacing, THE Filter_Engine SHALL produce the same inclusion result for both.

### Requirement 5: Location Alias Expansion and Country Disambiguation

**User Story:** As a job seeker, I want region and country terms to expand to their member locations without cross-country confusion, so that ambiguous abbreviations resolve to the location I intend.

#### Acceptance Criteria

1. WHEN a location include term matches a known region token after trimming surrounding whitespace and lowercasing, THE Alias_Table SHALL expand the term to itself plus its member location tokens.
2. WHERE a country is present in Location_Criteria, THE Filter_Engine SHALL add the country's expansion tokens to the effective location include set.
3. WHEN the effective location include set contains duplicate tokens after expansion, THE Filter_Engine SHALL deduplicate the set so each token appears exactly once.
4. IF a location include token does not match any known region, country, or alias token after trimming and lowercasing, THEN THE Alias_Table SHALL pass the token through unchanged as its own effective include token.
5. WHERE the country is Canada, THE Alias_Table SHALL NOT expand the token `ca` to California tokens.
6. WHERE the country is United States, THE Alias_Table SHALL expand the token `ca` to California tokens.
7. IF no country is present in Location_Criteria, THEN THE Alias_Table SHALL NOT expand the ambiguous token `ca` to California tokens.
8. THE Alias_Table SHALL ship a default seed whose location coverage includes all United States state names and their two-letter abbreviations, major United States city hubs, and the known regions bay area, greater new york, greater seattle, and greater los angeles.

### Requirement 6: Remote Mode Handling

**User Story:** As a job seeker, I want to filter by remote or onsite status, so that I only see roles matching my work-location preference.

#### Acceptance Criteria

1. WHERE Remote_Mode is RemoteOnly, THE Filter_Engine SHALL include a job if and only if the Alias_Table identifies the job location as remote.
2. WHERE Remote_Mode is OnsiteOnly, THE Filter_Engine SHALL exclude every job whose location the Alias_Table identifies as remote.
3. WHERE Remote_Mode is Any, THE Filter_Engine SHALL apply no remote-status constraint, so that a job's remote status alone neither includes nor excludes it.
4. WHERE Remote_Mode is not OnsiteOnly, WHILE a job is identified as remote and the job location field contains no non-remote place name, THE Filter_Engine SHALL include the job regardless of any location-name include list.
5. THE Alias_Table SHALL identify a job location as remote if and only if the location text contains at least one configured remote token (remote, anywhere, distributed, wfh) matched case-insensitively.
6. IF Remote_Mode is absent or holds a value other than Any, RemoteOnly, or OnsiteOnly, THEN THE Filter_Engine SHALL apply the Any behavior defined in criterion 3.

### Requirement 7: Global and Per-Watch Criteria Resolution

**User Story:** As a job seeker, I want to set a global filter and optionally override it per board, so that I can tune filtering broadly and precisely without repeating myself.

#### Acceptance Criteria

1. WHERE a watch has a per-watch override, THE Filter_Resolver SHALL return exactly that override without blending it field-by-field with the global criteria.
2. WHERE a watch has no per-watch override, THE Filter_Resolver SHALL return exactly the global criteria.
3. IF neither a per-watch override nor global criteria are available, THEN THE Filter_Resolver SHALL return Match_All.
4. THE Filter_Resolver SHALL resolve criteria once per company and source group and reuse the resolved value within a single list call.
5. WHILE a single list call is in progress, IF the underlying global or per-watch criteria change, THEN THE Filter_Resolver SHALL continue using the value already resolved for that company and source group for the remainder of the call.

### Requirement 8: Query-Time Filtering of Watch Roles

**User Story:** As a job seeker, I want criteria changes to re-filter my existing roles immediately, so that I do not have to re-sync boards or permanently lose already-discovered roles.

#### Acceptance Criteria

1. WHEN new-from-watch roles or open watch positions are listed, THE Filter_Engine SHALL evaluate the resolved criteria against each candidate job at query time.
2. WHEN listing watch roles, THE Filter_Engine SHALL return only jobs whose evaluation result is included.
3. WHEN a result limit is applied to a watch listing, THE listing SHALL apply the limit after filtering.
4. WHEN criteria are changed, THE next watch listing SHALL reflect the new criteria without requiring a re-sync.

### Requirement 9: Ingest-Time Filter Annotation

**User Story:** As a job seeker, I want the system to record whether synced roles match current criteria without discarding them, so that tightening criteria never permanently deletes discovered roles.

#### Acceptance Criteria

1. WHEN a watch sync ingests a remote role, THE Filter_Engine SHALL evaluate the resolved criteria against the role.
2. WHEN a watch sync ingests a remote role, THE Migration_Process SHALL store the role and record the evaluation result as a filter hint.
3. THE watch sync SHALL store every ingested remote role regardless of the evaluation result.
4. THE watch sync SHALL NOT delete or remove any stored job as a result of the evaluation result.
5. WHEN a role's filter evaluation result is not included, THE watch sync SHALL retain the stored job AND SHALL leave the previously recorded filter hint value unchanged.

### Requirement 10: Consistent Matching Across Call Sites

**User Story:** As a job seeker, I want query-time and sync-time filtering to agree, so that filtering behaves predictably no matter where it runs.

#### Acceptance Criteria

1. WHEN the same criteria, alias table, and job are evaluated at query time and at sync time, THE Filter_Engine SHALL produce identical inclusion results.
2. THE Filter_Engine SHALL be the single authority that decides whether a role matches.

### Requirement 11: Deterministic Pure Evaluation

**User Story:** As a developer, I want the matching engine to be pure and deterministic, so that its behavior is testable and repeatable.

#### Acceptance Criteria

1. WHEN the same criteria, alias table, and job are evaluated repeatedly, THE Filter_Engine SHALL return the same result each time.
2. THE Filter_Engine SHALL evaluate a job without mutating the criteria, the alias table, or the job.
3. THE Filter_Engine SHALL return a match result containing the inclusion decision and a human-readable reason.

### Requirement 12: Tauri Command Surface

**User Story:** As a frontend developer, I want commands to get and set criteria and preview matches, so that the editor UI can read, write, and dry-run filters.

#### Acceptance Criteria

1. WHEN the global get command is invoked, THE Command_Surface SHALL return the current global Filter_Criteria.
2. WHEN the global set command is invoked with valid criteria, THE Command_Surface SHALL persist the criteria as the global Filter_Criteria.
3. WHEN the per-watch get command is invoked, THE Command_Surface SHALL return the watch's override criteria or a null value when no override exists.
4. WHEN the per-watch set command is invoked with a null override, THE Command_Surface SHALL clear the override so the watch inherits the global criteria.
5. WHEN the preview command is invoked with criteria and sample entries, THE Command_Surface SHALL return the inclusion result and reason for each sample.

### Requirement 13: Frontend Filter Editor

**User Story:** As a job seeker, I want a structured editor for filters in Settings and per board, so that I can configure include/exclude tokens, country, and remote mode instead of free-text fields.

#### Acceptance Criteria

1. THE Filter_Editor SHALL render a global criteria editor in Settings with title include and exclude inputs, a country selector, location include and exclude inputs, and a remote mode selector.
2. THE Filter_Editor SHALL render a per-watch filter control in WatchRow that indicates when the global filter is in use.
3. WHEN a user customizes a watch filter, THE Filter_Editor SHALL save the customized criteria as that watch's override.
4. WHEN a user resets a watch filter to global, THE Filter_Editor SHALL clear the watch's override.
5. WHEN criteria are edited in the Filter_Editor, THE Filter_Editor SHALL show a live preview of which sample roles pass.

### Requirement 14: Legacy Settings Migration

**User Story:** As an existing user, I want my prior country, city, and keyword settings preserved, so that upgrading does not lose my configuration or require reconfiguration.

#### Acceptance Criteria

1. WHERE global structured Filter_Criteria are absent, THE Migration_Process SHALL build global Filter_Criteria that map the legacy country value, each legacy city value, and each legacy role-keyword value into their corresponding Filter_Criteria fields, preserving all non-empty legacy values without loss or reordering.
2. WHEN building global criteria from legacy role-keyword settings that contain at least one non-empty keyword, THE Migration_Process SHALL add each keyword to the title include list using Word match mode.
3. IF the legacy role-keyword setting is absent or contains no non-empty keyword, THEN THE Migration_Process SHALL create the global Filter_Criteria with an empty title include list and SHALL NOT record a migration failure.
4. WHILE global structured Filter_Criteria already exist, THE Migration_Process SHALL leave the existing structured Filter_Criteria unchanged and SHALL produce identical global Filter_Criteria output for every subsequent run given identical legacy input.
5. THE Migration_Process SHALL retain the legacy country, cities, and role-keyword keys after migration completes so the pre-migration configuration can be restored.
6. IF the Migration_Process fails before completion, THEN THE Migration_Process SHALL leave the legacy keys and any pre-existing structured Filter_Criteria in their pre-migration state and SHALL surface an error indication reporting the migration failure.
7. WHERE the Alias_Table is absent, THE Migration_Process SHALL seed the default Alias_Table.

### Requirement 15: Backward-Compatible Command Coexistence

**User Story:** As an operator, I want the old commands and keys to keep working during transition, so that a rollback is safe and the two surfaces stay consistent.

#### Acceptance Criteria

1. THE Command_Surface SHALL keep the legacy keyword and location commands registered during the transition release.
2. WHEN a legacy setter command is invoked, THE Command_Surface SHALL write through to the structured global Filter_Criteria so both surfaces stay consistent.
3. IF the write-through to the structured global Filter_Criteria does not succeed, THEN THE Command_Surface SHALL fail the legacy setter command and report an error.

### Requirement 16: Fail-Open Error Handling

**User Story:** As a job seeker, I want malformed or missing filter data to show more roles rather than hide everything, so that I never silently lose visibility due to a storage error.

#### Acceptance Criteria

1. IF stored global or per-watch criteria fail to deserialize or carry an unknown version, THEN THE Filter_Resolver SHALL fall back to Match_All for that scope and log a warning.
2. IF the alias table is absent or invalid, THEN THE Filter_Resolver SHALL use the default seed alias table in memory.
3. WHEN a subsequent valid set command runs for a scope with previously bad data, THE Command_Surface SHALL overwrite the bad value with valid criteria.

### Requirement 17: Input Validation on Write

**User Story:** As a job seeker, I want invalid filter submissions rejected without saving, so that a bad edit cannot corrupt my stored configuration.

#### Acceptance Criteria

1. IF a set command receives criteria with an unknown version or non-array token fields, THEN THE Command_Surface SHALL return an error and SHALL NOT persist the criteria.
2. IF a per-watch set or get command receives an unknown watch identifier, THEN THE Command_Surface SHALL return a watch-not-found error.
3. WHEN persisting criteria, THE Command_Surface SHALL trim tokens and drop empty tokens.

### Requirement 18: Non-Destructive Filtering and CSV Preservation

**User Story:** As a job seeker, I want filtering to change only what I see and never what is stored or exported, so that tightening my filters can never permanently lose discovered roles or shrink my CSV export.

#### Acceptance Criteria

1. WHEN Filter_Criteria are applied or changed, THE Filter_Engine SHALL NOT delete or remove any job from the jobs table.
2. WHEN Filter_Criteria are applied or changed, THE Filter_Engine SHALL NOT remove, exclude, or alter which jobs appear in the CSV_Export.
3. THE CSV_Export row selection SHALL remain independent of Filter_Criteria, selecting rows by the existing pipeline-tracking rules rather than by filter match results.
4. WHERE a job would be omitted from a watch listing by the current Filter_Criteria, THE CSV_Export SHALL still export that job when it otherwise qualifies as a tracked pipeline job.
5. THE Filter_Engine SHALL affect only which jobs are displayed at query time and SHALL NOT modify stored data.
