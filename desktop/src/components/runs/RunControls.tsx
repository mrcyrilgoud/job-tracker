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
export function RunControls() {
  const { state, start } = useRunMonitor();
  const busy = isRunBusy(state);
  const status = state.displayed !== null ? runStatusLabel(state.displayed.runStatus) : null;
  // RunPanel owns notices while a snapshot is displayed. The header is the
  // only visible surface for start/action failures when there is no snapshot.
  const notice = state.displayed === null ? state.notice : undefined;

  return (
    <div className="flex flex-wrap items-center gap-2">
      {busy && status !== null ? (
        <span className="text-xs font-medium text-[var(--muted)]" aria-live="off">
          {status}…
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
      <button
        type="button"
        onClick={() => void start("postingCheck")}
        disabled={busy}
        className="btn btn-secondary text-xs"
        title="Check every saved posting's open/closed state"
      >
        Check postings
      </button>
      <button
        type="button"
        onClick={() => void start("careerCheck")}
        disabled={busy}
        className="btn btn-secondary text-xs"
        title="Check connected job boards and configured careers pages for changes"
      >
        Check career sources
      </button>
    </div>
  );
}
