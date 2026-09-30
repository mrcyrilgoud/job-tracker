import { useId, useState } from "react";

import type { EvidenceView } from "@/lib/run-contract";

/**
 * A native disclosure for one posting's Check_Evidence (Req 7.6).
 *
 * The trigger is a real `<button>` carrying `aria-expanded` and `aria-controls`,
 * so it is keyboard operable and announced correctly. Toggling it never moves
 * focus elsewhere (Req 12.8): the button keeps focus and the panel is revealed
 * below it. When there is no evidence (a row that has not completed yet) nothing
 * renders.
 *
 * The revealed panel shows the human reason plus the sanitized evidence facts:
 * HTTP status, requested and final URL, redirect statuses, the provider signal,
 * the content-signal categories, and the failure category. Every value is text
 * (Req 12.6); no meaning is carried by color alone.
 */
export function EvidenceDisclosure({
  evidence,
  reason,
}: {
  /** The sanitized evidence for a completed posting, or `undefined` if absent. */
  evidence: EvidenceView | undefined;
  /** The Classification_Reason for the row, shown at the top of the panel. */
  reason?: string;
}) {
  const [open, setOpen] = useState(false);
  const panelId = useId();

  if (evidence === undefined && (reason === undefined || reason === "")) {
    return null;
  }

  const rows: Array<{ label: string; value: string }> = [];
  if (evidence !== undefined) {
    if (evidence.httpStatus !== undefined) {
      rows.push({ label: "HTTP status", value: String(evidence.httpStatus) });
    }
    rows.push({ label: "Requested URL", value: evidence.requestedUrl });
    if (evidence.finalUrl !== undefined) {
      rows.push({ label: "Final URL", value: evidence.finalUrl });
    }
    if (evidence.redirectStatuses !== undefined && evidence.redirectStatuses.length > 0) {
      rows.push({ label: "Redirects", value: evidence.redirectStatuses.join(" → ") });
    }
    if (evidence.provider !== undefined) {
      const p = evidence.provider;
      const providerStatus = p.httpStatus !== undefined ? ` (HTTP ${p.httpStatus})` : "";
      rows.push({
        label: "Provider signal",
        value: `${p.provider}: ${p.signal.replace(/_/g, " ")} (id ${p.postingId})${providerStatus}`,
      });
    }
    if (evidence.content !== undefined && evidence.content.length > 0) {
      rows.push({
        label: "Content signals",
        value: evidence.content.map((c) => c.replace(/_/g, " ")).join(", "),
      });
    }
    if (evidence.failureCategory !== undefined) {
      rows.push({ label: "Failure category", value: evidence.failureCategory.replace(/_/g, " ") });
    }
    rows.push({ label: "Checked at", value: evidence.attemptedAt });
  }

  return (
    <div className="mt-1.5">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((v) => !v)}
        className="btn btn-ghost btn-sm px-0 text-xs"
      >
        {open ? "Hide details" : "Show details"}
      </button>
      {open ? (
        <div
          id={panelId}
          className="mt-1.5 space-y-1.5 rounded-xl border border-[var(--border)] bg-[var(--surface-muted)] p-3 text-xs"
        >
          {reason ? (
            <p className="text-[var(--foreground)]">{reason}</p>
          ) : null}
          {rows.length > 0 ? (
            <dl className="grid grid-cols-[auto,1fr] gap-x-3 gap-y-1">
              {rows.map((row) => (
                <div key={row.label} className="contents">
                  <dt className="font-medium text-[var(--muted)]">{row.label}</dt>
                  <dd className="break-all text-[var(--foreground)]">{row.value}</dd>
                </div>
              ))}
            </dl>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
