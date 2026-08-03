import { formatDistanceToNow } from "date-fns";
import { useState } from "react";

import { providerLabel, syncErrorNote, watchPresentation } from "@/lib/companies-ui";
import type { CompanyWatch } from "@/lib/schema";
import type { Feedback } from "@/lib/use-pending-actions";
import { toneClasses } from "@/lib/ui";

/**
 * One watched board. The status pill leads because that is what the user came to
 * check; the provider and board name are plumbing and are sized accordingly
 * (DESIGN.md: "visible if you look, never shouting").
 */
export function WatchRow({
  watch,
  syncing,
  removing,
  feedback,
  onSync,
  onRemove,
}: {
  watch: CompanyWatch;
  syncing: boolean;
  removing: boolean;
  feedback?: Feedback;
  onSync: () => void;
  onRemove: () => void;
}) {
  const [confirmingRemove, setConfirmingRemove] = useState(false);
  const status = watchPresentation(watch);

  const checked = watch.lastSyncedAt
    ? `Checked ${formatDistanceToNow(new Date(watch.lastSyncedAt), { addSuffix: true })}`
    : "Not checked yet";

  return (
    <div className="rounded-xl bg-[var(--surface-muted)] px-4 py-3">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className={`pill ${toneClasses[status.tone]}`}>
              <span className="pill-dot" aria-hidden />
              {status.label}
            </span>
            <span className="text-xs text-[var(--muted)]">{checked}</span>
          </div>
          <p className="mt-1 text-xs text-[var(--faint)]">
            {providerLabel(watch.provider)} · {watch.boardSlug}
          </p>
        </div>

        {confirmingRemove ? (
          <div className="flex items-center gap-2">
            <span className="text-sm text-[var(--muted)]">Stop watching this board?</span>
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              disabled={removing}
              onClick={onRemove}
            >
              {removing ? <span className="spinner" aria-hidden /> : null}
              Yes, stop
            </button>
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => setConfirmingRemove(false)}
            >
              Keep
            </button>
          </div>
        ) : (
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              disabled={syncing}
              onClick={onSync}
            >
              {syncing ? <span className="spinner" aria-hidden /> : null}
              {syncing ? "Checking…" : "Check now"}
            </button>
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => setConfirmingRemove(true)}
            >
              Stop watching
            </button>
          </div>
        )}
      </div>

      {watch.lastSyncError ? (
        <div className="mt-2">
          <p className="text-sm text-[var(--danger)]">
            {syncErrorNote(watch.lastSyncError, watch.provider)}
          </p>
          <details className="mt-1">
            <summary className="cursor-pointer text-xs text-[var(--faint)] hover:text-[var(--muted)]">
              Technical details
            </summary>
            <p className="mt-1 font-mono text-xs break-all text-[var(--faint)]">
              {watch.lastSyncError}
            </p>
          </details>
        </div>
      ) : null}

      {feedback ? (
        <p
          className={`mt-2 text-sm ${
            feedback.tone === "positive" ? "text-[var(--green-ink)]" : "text-[var(--danger)]"
          }`}
          role={feedback.tone === "negative" ? "alert" : undefined}
        >
          {feedback.text}
        </p>
      ) : null}
    </div>
  );
}
