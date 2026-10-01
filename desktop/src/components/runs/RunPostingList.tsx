import { EvidenceDisclosure } from "@/components/runs/EvidenceDisclosure";
import { toneClasses, type Tone } from "@/lib/ui";
import { useRunMonitor } from "@/lib/RunMonitorContext";
import type { PostingProgress } from "@/lib/run-contract";
import {
  attentionPostings,
  isRetryEligible,
  needsAttention,
  postingRowStatusText,
  visiblePostings,
} from "@/lib/run-state";

/** Status pill tone; text still carries the meaning (Req 12.6). */
function statusTone(row: PostingProgress): Tone {
  switch (row.status) {
    case "queued":
    case "canceled":
      return "stone";
    case "active":
      return "blue";
    case "error":
      return "danger";
    case "completed":
      if (row.postingState === "active") return "green";
      if (row.postingState === "inactive") return "stone";
      return "amber"; // unknown — "Couldn't confirm"
    default: {
      const _exhaustive: never = row.status;
      return _exhaustive;
    }
  }
}

/**
 * The per-posting list for the run panel (Req 3.7, 3.8, 5.8, 9.7).
 *
 * Presentational: the displayed run, filter, and retry selection all come from
 * `useRunMonitor`, so task 13.4 only has to render `<RunPostingList />` inside
 * the panel with no props. Rows are keyed by `jobId` (stable across event
 * updates, so a focused control is never remounted — Req 12.8).
 *
 * Each row shows the frozen Job_Identity (title, company), a text status pill,
 * and — once completed — the Posting_State label with its reason and an
 * evidence disclosure. Retry checkboxes appear only on retry-eligible rows
 * (Unknown state, Error, or Canceled), and "Select all needing attention"
 * selects the Unknown/Error rows.
 */
export function RunPostingList() {
  const { state, setFilter, toggleRetry, selectAllNeedingAttention } = useRunMonitor();

  const snapshot = state.displayed;
  if (snapshot === null || snapshot.runType === "careerCheck") return null;

  const rows = visiblePostings(state);
  const attentionCount = attentionPostings(snapshot).length;
  const totalCount = snapshot.postings.length;

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div
          role="group"
          aria-label="Filter postings"
          className="inline-flex rounded-lg border border-[var(--border)] p-0.5"
        >
          <button
            type="button"
            aria-pressed={state.filter === "all"}
            onClick={() => setFilter("all")}
            className={`rounded-md px-2.5 py-1 text-xs font-semibold transition-colors ${
              state.filter === "all"
                ? "bg-[var(--accent-soft)] text-[var(--accent-ink)]"
                : "text-[var(--muted)] hover:text-[var(--foreground)]"
            }`}
          >
            All ({totalCount})
          </button>
          <button
            type="button"
            aria-pressed={state.filter === "attention"}
            onClick={() => setFilter("attention")}
            className={`rounded-md px-2.5 py-1 text-xs font-semibold transition-colors ${
              state.filter === "attention"
                ? "bg-[var(--accent-soft)] text-[var(--accent-ink)]"
                : "text-[var(--muted)] hover:text-[var(--foreground)]"
            }`}
          >
            Needs attention ({attentionCount})
          </button>
        </div>

        {attentionCount > 0 ? (
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={() => selectAllNeedingAttention()}
          >
            Select all needing attention
          </button>
        ) : null}
      </div>

      {rows.length === 0 ? (
        <p className="rounded-xl border border-dashed border-[var(--border)] p-4 text-center text-xs text-[var(--faint)]">
          {state.filter === "attention" ? "Nothing needs attention." : "No postings in this run."}
        </p>
      ) : (
        <ul className="divide-y divide-[var(--border)]">
          {rows.map((row) => {
            const eligible = isRetryEligible(row);
            const selected = state.retrySelection.has(row.jobId);
            const tone = statusTone(row);
            return (
              <li key={row.jobId} className="flex flex-wrap items-start gap-x-3 gap-y-2 py-2.5 first:pt-0 last:pb-0">
                {eligible ? (
                  <label className="mt-0.5 flex shrink-0 items-center gap-1.5 text-xs text-[var(--muted)]">
                    <input
                      type="checkbox"
                      checked={selected}
                      onChange={() => toggleRetry(row.jobId)}
                      aria-label={`Select ${row.title} at ${row.companyName} for retry`}
                    />
                    <span className="sr-only">Retry</span>
                  </label>
                ) : (
                  <span className="mt-0.5 w-4 shrink-0" aria-hidden />
                )}

                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <a
                      href={row.postingUrl}
                      target="_blank"
                      rel="noreferrer"
                      className="min-w-0 truncate text-sm font-medium text-[var(--foreground)] hover:text-[var(--accent)] hover:underline"
                    >
                      {row.title}
                    </a>
                    <span className={`pill text-[11px] ${toneClasses[tone]}`}>
                      <span className="pill-dot" />
                      {postingRowStatusText(row)}
                    </span>
                    {needsAttention(row) ? (
                      <span className="text-[11px] font-medium text-[var(--amber-ink)]">Needs attention</span>
                    ) : null}
                  </div>
                  <p className="mt-0.5 truncate text-xs text-[var(--muted)]">{row.companyName}</p>
                  {row.reason ? (
                    <p className="mt-0.5 text-xs text-[var(--faint)]">{row.reason}</p>
                  ) : null}
                  {row.status === "completed" ? (
                    <EvidenceDisclosure evidence={row.evidence} reason={row.reason} />
                  ) : null}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
