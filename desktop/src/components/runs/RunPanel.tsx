import { useEffect, useState } from "react";

import { RunLiveRegions } from "@/components/runs/RunLiveRegions";
import { RunPostingList } from "@/components/runs/RunPostingList";
import { RunSummaryCard } from "@/components/runs/RunSummaryCard";
import { useRunMonitor } from "@/lib/RunMonitorContext";
import {
  canRetry,
  isTerminalRunStatus,
  runViewModel,
  STAGE_LABELS,
  STAGE_OUTCOME_LABELS,
} from "@/lib/run-state";

/**
 * The persistent run panel shown under the header (Req 2.1, 2.2, 2.3, 5.1, 5.2).
 *
 * It renders the displayed run's identity and progress from `runViewModel`:
 * run id, type label, status text, current stage, completed/total counts, a
 * decorative progress bar with an always-visible `n / total` text (Req 12.7),
 * and elapsed time. For a Jobs_Cycle it also lists each stage's outcome as
 * text. It hosts the Cancel button (enabled only for queued/active — Req 5.1,
 * 5.2), the Retry action for a terminal run with a selection, transient
 * notices, the posting list, the summary card, and the live regions.
 *
 * The elapsed clock ticks once a second for a live, non-terminal run so the
 * duration advances between events; terminal runs freeze on the summary
 * duration. Progress updates never call `focus()` here (Req 12.8): the ticking
 * `now` only feeds `runViewModel`.
 */
export function RunPanel() {
  const { state, cancel, retry } = useRunMonitor();
  const snapshot = state.displayed;
  const terminal = snapshot !== null && isTerminalRunStatus(snapshot.runStatus);

  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (snapshot === null || terminal) return undefined;
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [snapshot, terminal]);

  if (snapshot === null) return null;

  const vm = runViewModel(snapshot, terminal ? undefined : now);
  const total = vm.total > 0 ? vm.total : 0;
  const pct = total > 0 ? Math.min(100, Math.round((vm.completed / total) * 100)) : 0;
  const notice = state.notice;
  const showRetry = canRetry(state);

  return (
    <section className="card mb-6 space-y-4 p-4" aria-label="Run status">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-sm font-semibold text-[var(--foreground)]">{vm.runTypeLabel}</h2>
            <span className="pill bg-[var(--surface-muted)] text-[var(--muted)] text-[11px]">
              <span className="pill-dot" />
              {vm.statusLabel}
            </span>
          </div>
          <p className="mt-0.5 truncate font-mono text-[11px] text-[var(--faint)]">{vm.runId}</p>
        </div>
        <div className="flex items-center gap-2">
          {vm.canCancel ? (
            <button type="button" className="btn btn-secondary btn-sm" onClick={() => void cancel()}>
              Cancel
            </button>
          ) : null}
          {showRetry ? (
            <button type="button" className="btn btn-secondary btn-sm" onClick={() => void retry()}>
              Retry selected
            </button>
          ) : null}
        </div>
      </div>

      {!terminal ? (
        <div className="space-y-1.5">
          <div className="flex items-center justify-between text-xs text-[var(--muted)]">
            <span>
              {STAGE_LABELS[vm.stage]} · {vm.elapsedLabel}
            </span>
            <span aria-hidden>
              {vm.completed} / {total}
            </span>
          </div>
          <div
            className="run-progress-bar h-1.5"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={total}
            aria-valuenow={vm.completed}
            aria-label={`${vm.completed} of ${total} complete`}
          >
            <div className="run-progress-fill" style={{ width: `${pct}%` }} />
          </div>
        </div>
      ) : null}

      {vm.errorReason ? (
        <p className="rounded-xl bg-[var(--danger-soft)] px-3 py-2 text-xs text-[var(--danger)]">
          {vm.errorReason}
        </p>
      ) : null}

      {notice !== undefined ? (
        <p
          className="rounded-xl bg-[var(--surface-muted)] px-3 py-2 text-xs text-[var(--muted)]"
          role="status"
        >
          {notice.message}
        </p>
      ) : null}

      {vm.stages !== undefined && vm.stages.length > 0 && !terminal ? (
        <ul className="flex flex-wrap gap-2 text-[11px] text-[var(--muted)]">
          {vm.stages.map((stage) => (
            <li key={stage.name} className="pill bg-[var(--surface-muted)]">
              {STAGE_LABELS[stage.name]}: {STAGE_OUTCOME_LABELS[stage.outcome]}
            </li>
          ))}
        </ul>
      ) : null}

      <RunSummaryCard />

      <RunPostingList />

      <RunLiveRegions />
    </section>
  );
}
