import { useRunMonitor } from "@/lib/RunMonitorContext";
import { IN_PROGRESS_MESSAGE, isRunBusy, runStatusLabel } from "@/lib/run-state";

/**
 * The two run-start controls in the header (Req 2.1, 12.2, 12.5).
 *
 * "Run jobs" starts a Jobs_Cycle; "Check postings" starts a Posting_Check_Run.
 * Both are native `<button>`s so they are keyboard operable and expose their
 * disabled state to assistive tech. While any displayed run is non-terminal
 * (`isRunBusy`), both buttons are `disabled` (Req 12.2), and the current run
 * status is shown as text next to them (Req 12.6) — never color alone.
 *
 * When a start is rejected because another run already owns the runner lock,
 * `RunMonitorProvider` maps it to an `inProgress` notice; this component shows
 * the "Another run is in progress" text (Req 4.5).
 */
export function RunControls() {
  const { state, start } = useRunMonitor();
  const busy = isRunBusy(state);
  const status = state.displayed !== null ? runStatusLabel(state.displayed.runStatus) : null;
  const inProgress = state.notice?.kind === "inProgress";

  return (
    <div className="flex flex-wrap items-center gap-2">
      {busy && status !== null ? (
        <span className="text-xs font-medium text-[var(--muted)]" aria-live="off">
          {status}…
        </span>
      ) : null}
      {inProgress ? (
        <span className="text-xs font-medium text-[var(--amber-ink)]">{IN_PROGRESS_MESSAGE}</span>
      ) : null}
      <button
        type="button"
        onClick={() => void start("jobsCycle")}
        disabled={busy}
        className="btn btn-secondary text-xs"
        title="Run posting checks, watch sync, careers checks, and CSV export"
      >
        Run jobs
      </button>
      <button
        type="button"
        onClick={() => void start("postingCheck")}
        disabled={busy}
        className="btn btn-secondary text-xs"
        title="Check every saved posting's open/closed state"
      >
        Check postings
      </button>
    </div>
  );
}
