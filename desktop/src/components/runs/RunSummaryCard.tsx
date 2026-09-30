import { useRunMonitor } from "@/lib/RunMonitorContext";
import type { RunSummary } from "@/lib/run-contract";
import { formatElapsed, RUN_TYPE_LABELS, STAGE_LABELS, STAGE_OUTCOME_LABELS } from "@/lib/run-state";

/**
 * The terminal Run_Summary card (Req 9.1, 9.3, 9.4).
 *
 * Rendered only when the displayed run is terminal and carries a summary. It
 * lists all five posting outcome counts including zeros (Req 9.1), the number
 * of postings whose state changed, and — for a Jobs_Cycle — every stage's
 * outcome as text (Req 9.3, 9.4). Dismiss removes the terminal run from view
 * (Req 2.3). Every value is text; no meaning is carried by color alone.
 */

const OUTCOME_LABELS: Array<{ key: keyof RunSummary["postingOutcomes"]; label: string }> = [
  { key: "active", label: "Open" },
  { key: "closed", label: "Closed" },
  { key: "unknown", label: "Couldn't confirm" },
  { key: "error", label: "Errors" },
  { key: "canceled", label: "Canceled" },
];

export function RunSummaryCard() {
  const { state, dismiss } = useRunMonitor();
  const summary = state.displayed?.summary;
  if (summary === undefined) return null;

  const o = summary.postingOutcomes;

  return (
    <section className="space-y-3" aria-label="Run summary">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold text-[var(--foreground)]">
            {RUN_TYPE_LABELS[summary.runType]} finished
          </h3>
          <p className="mt-0.5 text-xs text-[var(--muted)]">
            {formatElapsed(summary.durationMs)} · {summary.stateChanges}{" "}
            {summary.stateChanges === 1 ? "state change" : "state changes"}
          </p>
        </div>
        <button type="button" className="btn btn-secondary btn-sm" onClick={() => void dismiss()}>
          Dismiss
        </button>
      </div>

      <dl className="grid grid-cols-2 gap-2 sm:grid-cols-5">
        {OUTCOME_LABELS.map(({ key, label }) => (
          <div
            key={key}
            className="rounded-xl border border-[var(--border)] bg-[var(--surface-muted)] px-3 py-2"
          >
            <dt className="text-[11px] font-medium text-[var(--muted)]">{label}</dt>
            <dd className="mt-0.5 text-lg font-semibold text-[var(--foreground)]">{o[key]}</dd>
          </div>
        ))}
      </dl>

      {summary.stages !== undefined && summary.stages.length > 0 ? (
        <ul className="divide-y divide-[var(--border)] rounded-xl border border-[var(--border)]">
          {summary.stages.map((stage) => (
            <li
              key={stage.name}
              className="flex items-center justify-between gap-3 px-3 py-2 text-xs"
            >
              <span className="font-medium text-[var(--foreground)]">{STAGE_LABELS[stage.name]}</span>
              <span className="text-[var(--muted)]">
                {STAGE_OUTCOME_LABELS[stage.outcome]}
                {stage.error ? ` — ${stage.error}` : ""}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}
