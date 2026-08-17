import { formatDistanceToNow } from "date-fns";
import { Link } from "react-router-dom";
import { useMemo, useState } from "react";

import { canResetDismissedWatchRole, matchesRoleSearch, providerLabel } from "@/lib/companies-ui";
import type { CompanyWatch, JobListItem, WatchProvider } from "@/lib/schema";

const SEARCHABLE_FROM = 8;

/** The full, latest-known board for one company, with personal triage shown
 * as a lightweight state instead of hiding an otherwise open position. */
export function OpenPositionsPanel({
  positions,
  watches,
  onSave,
  onDismissNew,
  onResetDismissed,
  isPending,
}: {
  positions: JobListItem[];
  watches: CompanyWatch[];
  onSave: (jobId: string) => void;
  onDismissNew: (jobId: string) => void;
  onResetDismissed: (jobId: string) => void;
  isPending: (key: string) => boolean;
}) {
  const [search, setSearch] = useState("");
  const matching = useMemo(
    () => positions.filter((position) => matchesRoleSearch(position, search)),
    [positions, search],
  );
  const lastSyncedAt = watches
    .map((watch) => watch.lastSyncedAt)
    .filter((value): value is string => Boolean(value))
    .sort()
    .at(-1);
  const hasAttention = watches.some((watch) => watch.lastSyncError);

  return (
    <section className="card p-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="font-display text-xl font-medium">Open positions</h2>
          <p className="mt-1 text-sm text-[var(--muted)]">
            {positions.length === 0
              ? "No current openings found from this company’s connected boards."
              : `${positions.length} ${positions.length === 1 ? "role" : "roles"} currently listed. New roles appear first.`}
          </p>
        </div>
        {lastSyncedAt ? (
          <p className="text-xs text-[var(--faint)]">
            Last checked {formatDistanceToNow(new Date(lastSyncedAt), { addSuffix: true })}
          </p>
        ) : null}
      </div>

      {hasAttention ? (
        <p className="mt-3 rounded-xl bg-[var(--amber-soft)] px-3.5 py-2.5 text-sm text-[var(--amber-ink)]">
          One of these boards needs attention, so this may be the last known list.
        </p>
      ) : null}

      {positions.length >= SEARCHABLE_FROM ? (
        <input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder="Search open positions…"
          aria-label="Search open positions"
          className="field mt-4 border-transparent bg-[var(--surface-muted)]"
        />
      ) : null}

      {positions.length > 0 ? (
        <ul className="mt-4 divide-y divide-[var(--border)]">
          {matching.map(({ job }) => {
            const disposition = job.watchDisposition ?? "saved";
            const pending =
              isPending(`save-open:${job.id}`) ||
              isPending(`triage:${job.id}`) ||
              isPending(`reset:${job.id}`);
            const canReset = canResetDismissedWatchRole(job);
            const detail = [job.location, providerLabel(job.source as WatchProvider)]
              .filter(Boolean)
              .join(" · ");
            return (
              <li
                key={job.id}
                className={`flex flex-wrap items-center justify-between gap-x-4 gap-y-2 py-3 first:pt-0 last:pb-0 ${
                  disposition === "dismissed" ? "opacity-65" : ""
                }`}
              >
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <a
                      href={job.url}
                      target="_blank"
                      rel="noreferrer"
                      className="text-sm font-medium text-[var(--foreground)] hover:text-[var(--accent)] hover:underline"
                    >
                      {job.title}
                    </a>
                    {disposition === "new" ? (
                      <span className="pill bg-[var(--accent-soft)] text-[var(--accent-ink)]">New</span>
                    ) : null}
                    {disposition === "dismissed" ? (
                      <span className="pill bg-[var(--stone-soft)] text-[var(--stone-ink)]">Not for me</span>
                    ) : null}
                  </div>
                  {detail ? <p className="mt-0.5 text-xs text-[var(--muted)]">{detail}</p> : null}
                </div>

                <div className="flex shrink-0 items-center gap-1.5">
                  {disposition === "saved" ? (
                    <Link to={`/jobs/${job.id}`} className="btn btn-secondary btn-sm">
                      On my list
                    </Link>
                  ) : disposition === "new" ? (
                    <button
                      type="button"
                      className="btn btn-primary btn-sm"
                      disabled={pending}
                      onClick={() => onSave(job.id)}
                    >
                      {pending ? <span className="spinner" aria-hidden /> : null}
                      Save to my list
                    </button>
                  ) : null}
                  {canReset ? (
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      disabled={pending}
                      onClick={() => onResetDismissed(job.id)}
                    >
                      {pending ? <span className="spinner" aria-hidden /> : null}
                      Reset
                    </button>
                  ) : null}
                  {disposition === "new" ? (
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      disabled={pending}
                      onClick={() => onDismissNew(job.id)}
                    >
                      Not for me
                    </button>
                  ) : null}
                </div>
              </li>
            );
          })}
        </ul>
      ) : null}

      {positions.length > 0 && matching.length === 0 ? (
        <p className="mt-4 text-sm text-[var(--muted)]">No open positions match “{search.trim()}”.</p>
      ) : null}
    </section>
  );
}
