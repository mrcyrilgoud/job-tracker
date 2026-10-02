import { useRunMonitor } from "@/lib/RunMonitorContext";
import { IN_PROGRESS_MESSAGE, isRunBusy, runStatusLabel } from "@/lib/run-state";

/**
 * The global run-start controls in the header (Req 2.1, 12.2, 12.5).
 *
 * "Check postings" starts a Posting_Check_Run and "Check career sources" starts
 * a Career_Check_Run. Both are native `<button>`s so they are keyboard
 * operable and expose their disabled state to assistive tech. While any
 * displayed run is non-terminal (`isRunBusy`), both buttons are `disabled`
 * (Req 12.2), and the current run status is shown as text next to them (Req
 * 12.6) — never color alone.
 *
 * When a start is rejected because another run already owns the runner lock,
 * `RunMonitorProvider` maps it to an `inProgress` notice; this component shows
 * the "Another run is in progress" text (Req 4.5).
 */
export function RunControls({
  panelVisible = true,
  onShowRun,
}: {
  panelVisible?: boolean;
  onShowRun?: () => void;
}) {
  const { state, start } = useRunMonitor();
  const busy = isRunBusy(state);
  const status = state.displayed !== null ? runStatusLabel(state.displayed.runStatus) : null;
  // Keep notices accessible when navigation hides the detailed run panel.
  const notice = state.displayed === null || !panelVisible ? state.notice : undefined;

  return (
    <div className="run-controls">
      <div className="run-actions">
        <button
          type="button"
          onClick={() => {
            onShowRun?.();
            void start("postingCheck");
          }}
          disabled={busy}
          className="btn btn-secondary btn-sm"
          title="Check every saved posting's open/closed state"
        >
          Check postings
        </button>
        <button
          type="button"
          onClick={() => {
            onShowRun?.();
            void start("careerCheck");
          }}
          disabled={busy}
          className="btn btn-secondary btn-sm"
          title="Check connected job boards and configured careers pages for changes"
        >
          Check career sources
        </button>
        <div className="run-status-slot">
          {onShowRun !== undefined ? (
            <button type="button" onClick={onShowRun} disabled={state.displayed === null} className="btn btn-secondary btn-sm">
              Show run status
            </button>
          ) : null}
        </div>
      </div>
      <div className="run-control-messages">
        {(busy || !panelVisible) && status !== null ? (
          <span className="text-xs font-medium text-[var(--muted)]" aria-live="off">
            {status}{busy ? "…" : ""}
          </span>
        ) : null}
        {notice !== undefined ? (
          <span
            className={`text-xs font-medium ${
              notice.kind === "inProgress" ? "text-[var(--amber-ink)]" : "text-[var(--muted)]"
            }`}
            role="status"
          >
            {notice.message || (notice.kind === "inProgress" ? IN_PROGRESS_MESSAGE : "")}
          </span>
        ) : null}
      </div>
    </div>
  );
}
