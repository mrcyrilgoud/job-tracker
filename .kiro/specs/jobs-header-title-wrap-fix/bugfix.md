# Bugfix Requirements Document

## Introduction

On the Jobs page, the header places the page title block (heading plus subtitle) side by side with a toolbar of controls: the "Sort by" select, the "Order" select, the sort helper text ("Based on when the job was added to the tracker."), the List/Board view toggle, and the "+ Add job" button. At moderate window widths, where the side-by-side layout is active but there isn't room for both the title and the full toolbar on one row, the toolbar takes most of the row. The title block gets squeezed into a narrow strip. The heading "All active jobs" then breaks one word per line ("All" / "active" / "jobs"), and the subtitle "2 roles on your radar." wraps across two lines. The header looks broken and is harder to read.

The fix should keep the title and subtitle readable at every window width and make the header adapt smoothly as the window resizes. At wide widths the header should look as it does today.

## Bug Analysis

### Current Behavior (Defect)

1.1 WHEN the Jobs page header is in its side-by-side layout and the combined natural width of the title block and the toolbar exceeds the available header width THEN the system shrinks the title block to the width of its longest single word, so the heading "All active jobs" renders one word per line

1.2 WHEN the Jobs page header is in its side-by-side layout and the combined natural width of the title block and the toolbar exceeds the available header width THEN the system wraps the subtitle (for example "2 roles on your radar.") across multiple lines even though it would fit on one line with a sensible title width

1.3 WHEN the window is resized across the range of widths where the toolbar crowds the title THEN the system changes the header layout abruptly (title collapsing to a narrow column) and does not degrade gracefully

1.4 WHEN a different heading variant is shown in the side-by-side layout under the same crowded conditions (for example "Favorites Board", "Archived Postings", or a status label such as "Interviewing") THEN the system likewise squeezes that heading and its subtitle into a narrow column, with words stacked per line

### Expected Behavior (Correct)

2.1 WHEN the Jobs page header is in its side-by-side layout and the combined natural width of the title block and the toolbar exceeds the available header width THEN the system SHALL keep the title block wide enough that a short heading such as "All active jobs" renders on a single line, for example by moving toolbar controls onto a row below the title or by giving the title block a sensible minimum width

2.2 WHEN the Jobs page header is in its side-by-side layout and the combined natural width of the title block and the toolbar exceeds the available header width THEN the system SHALL render a short subtitle such as "2 roles on your radar." on a single line, and longer subtitles SHALL wrap only at the natural width of the title block, never one or two words per line

2.3 WHEN the window is resized continuously from narrow to wide THEN the system SHALL change the header layout smoothly (toolbar controls wrapping or moving below the title as needed, optional fluid heading size) with no width at which the heading collapses into one word per line

2.4 WHEN any heading variant (all active jobs, favorites, archived, a status label, or any other heading the page header can show) is displayed at any window width THEN the system SHALL keep the heading and subtitle readable, wrapping a heading only when it cannot fit on one line at the available header width

### Unchanged Behavior (Regression Prevention)

3.1 WHEN the window is wide enough for the title block and the full toolbar to fit on one row THEN the system SHALL CONTINUE TO show the title block on the left and the toolbar on the right, aligned as they are today

3.2 WHEN the window is below the medium breakpoint (stacked layout) THEN the system SHALL CONTINUE TO show the title block above the toolbar

3.3 WHEN the user changes the "Sort by" or "Order" select THEN the system SHALL CONTINUE TO update the job list sort and URL parameters exactly as today, including the "Date applied" option on the applied tab

3.4 WHEN the sort field is "Date added" (posting date) THEN the system SHALL CONTINUE TO show the helper text "Based on when the job was added to the tracker." and link it to the "Sort by" select via aria-describedby; WHEN the sort field is anything else THEN the system SHALL CONTINUE TO hide that helper text

3.5 WHEN the user clicks the List or Board toggle THEN the system SHALL CONTINUE TO switch the view mode and highlight the active option as today

3.6 WHEN the user clicks "+ Add job" THEN the system SHALL CONTINUE TO navigate to the new-job page

3.7 WHEN the favorites or archived filter is active THEN the system SHALL CONTINUE TO show the matching star or archive icon badge next to the heading

3.8 WHEN the page computes the heading and subtitle text for any filter (all active, favorites, archived, a status, empty list, or more than 100 results) THEN the system SHALL CONTINUE TO produce the same heading and subtitle strings as today

3.9 WHEN the header is rendered at any width THEN the system SHALL CONTINUE TO keep the toolbar controls fully usable, with no control clipped, overlapping, or hidden
