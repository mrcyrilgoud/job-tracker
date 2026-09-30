/**
 * RunLiveRegions: the two ARIA live regions for the Run panel (Req 12.3, 12.4).
 *
 * The polite region (`role="status"`, `aria-live="polite"`) announces run-status
 * changes and throttled aggregate progress. The assertive region
 * (`role="alert"`, `aria-live="assertive"`) announces each posting Error and run
 * Error once.
 *
 * Both regions are fed entirely by `useRunMonitor().announcements`, which the
 * provider has already de-duplicated and throttled via `buildAnnouncements`.
 * This component only renders that text; it never recomputes announcements and
 * never calls `focus()`, so updating progress cannot move the user's keyboard
 * focus (Req 12.8).
 *
 * Both regions are visually hidden with Tailwind's `sr-only` utility but remain
 * in the accessibility tree, and both are always mounted so screen readers keep
 * a stable region to observe.
 */

import { type ReactElement } from "react";

import { useRunMonitor } from "@/lib/RunMonitorContext";

export function RunLiveRegions(): ReactElement {
  const { announcements } = useRunMonitor();
  const { polite, assertive } = announcements;

  return (
    <>
      <div className="sr-only" role="status" aria-live="polite" aria-atomic="true">
        {polite ?? ""}
      </div>
      <div className="sr-only" role="alert" aria-live="assertive" aria-atomic="true">
        {assertive.map((text, index) => (
          // Announcements are already de-duplicated; index keeps stable ordering.
          <p key={`${index}:${text}`}>{text}</p>
        ))}
      </div>
    </>
  );
}
