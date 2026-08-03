import type { JobListItem } from "@/lib/schema";

export type TriageAction = "save" | "skip";

/**
 * The roles a watch found that are not on the board yet. Kept deliberately
 * scannable: title first, everything else is supporting detail, and the two
 * decisions are always in the same place on every row.
 */
export function NewRolesList({
  roles,
  showCompany,
  onTriage,
  isPending,
}: {
  roles: JobListItem[];
  /** Company name belongs on the row only when the list spans companies. */
  showCompany: boolean;
  onTriage: (jobId: string, action: TriageAction) => void;
  isPending: (jobId: string) => boolean;
}) {
  return (
    <ul className="divide-y divide-[var(--border)]">
      {roles.map(({ job, companyName }) => {
        const pending = isPending(job.id);
        const detail = [showCompany ? companyName : null, job.location]
          .filter(Boolean)
          .join(" · ");
        return (
          <li
            key={job.id}
            className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 py-2.5 first:pt-0 last:pb-0"
          >
            <div className="min-w-0 flex-1">
              <a
                href={job.url}
                target="_blank"
                rel="noreferrer"
                className="text-sm font-medium text-[var(--foreground)] hover:text-[var(--accent)] hover:underline"
              >
                {job.title}
              </a>
              {detail ? (
                <p className="mt-0.5 truncate text-xs text-[var(--muted)]">{detail}</p>
              ) : null}
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              <button
                type="button"
                className="btn btn-primary btn-sm"
                disabled={pending}
                onClick={() => onTriage(job.id, "save")}
              >
                {pending ? <span className="spinner" aria-hidden /> : null}
                Save to my list
              </button>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                disabled={pending}
                onClick={() => onTriage(job.id, "skip")}
              >
                Not for me
              </button>
            </div>
          </li>
        );
      })}
    </ul>
  );
}
