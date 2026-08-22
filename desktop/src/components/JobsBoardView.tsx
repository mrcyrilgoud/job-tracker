import { formatDistanceToNow } from "date-fns";
import { Link } from "react-router-dom";

import { FavoriteButton } from "@/components/FavoriteButton";
import { statusIcons } from "@/components/icons";
import { jobStatuses, type JobListItem, type JobStatus } from "@/lib/schema";
import {
  jobStatusPresentation,
  postingStateMatters,
  postingStatePresentation,
  toneClasses,
} from "@/lib/ui";

const BOARD_COLUMNS: JobStatus[] = ["wishlist", "applied", "interviewing", "offer"];

const NEXT_STAGE: Partial<Record<JobStatus, JobStatus>> = {
  wishlist: "applied",
  applied: "interviewing",
  interviewing: "offer",
};

export function JobsBoardView({
  jobs,
  onToggleFavorite,
  onUpdateStatus,
  isPendingFavorite,
}: {
  jobs: JobListItem[];
  onToggleFavorite: (jobId: string) => void;
  onUpdateStatus?: (jobId: string, nextStatus: JobStatus) => void;
  isPendingFavorite?: (jobId: string) => boolean;
}) {
  const jobsByStage = BOARD_COLUMNS.reduce<Record<JobStatus, JobListItem[]>>(
    (acc, stage) => {
      acc[stage] = jobs.filter((item) => item.job.status === stage);
      return acc;
    },
    { wishlist: [], applied: [], interviewing: [], offer: [], rejected: [], withdrawn: [], closed: [] },
  );

  // Check if there are closed / archived jobs in the current set
  const archivedJobs = jobs.filter(
    (item) => item.job.status === "rejected" || item.job.status === "withdrawn" || item.job.status === "closed",
  );

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-4">
        {BOARD_COLUMNS.map((stage) => {
          const stageInfo = jobStatusPresentation(stage);
          const StageIcon = statusIcons[stage];
          const stageJobs = jobsByStage[stage] ?? [];
          const nextStage = NEXT_STAGE[stage];

          return (
            <div
              key={stage}
              className="flex min-h-[450px] flex-col rounded-2xl border border-[var(--border)] bg-[var(--surface-muted)]/50 p-3.5"
            >
              <div className="mb-3.5 flex items-center justify-between px-1">
                <div className="flex items-center gap-2">
                  <span className={`flex h-6 w-6 items-center justify-center rounded-lg ${toneClasses[stageInfo.tone]}`}>
                    <StageIcon size={13} />
                  </span>
                  <h3 className="font-display text-base font-semibold text-[var(--foreground)]">
                    {stageInfo.label}
                  </h3>
                </div>
                <span className="font-display text-xs font-medium text-[var(--faint)]">
                  {stageJobs.length}
                </span>
              </div>

              <div className="flex-1 space-y-2.5 overflow-y-auto">
                {stageJobs.length === 0 ? (
                  <div className="flex h-32 flex-col items-center justify-center rounded-xl border border-dashed border-[var(--border)] p-4 text-center">
                    <p className="text-xs text-[var(--faint)]">No roles here yet</p>
                  </div>
                ) : (
                  stageJobs.map(({ job, companyName }) => {
                    const postingInfo = postingStateMatters(job.status)
                      ? postingStatePresentation(job.postingState)
                      : null;
                    const isPending = isPendingFavorite?.(job.id);

                    return (
                      <div
                        key={job.id}
                        className="card group relative flex flex-col justify-between p-4 transition-all duration-150 hover:-translate-y-0.5 hover:shadow-[var(--shadow-md)]"
                      >
                        <div className="space-y-2">
                          <div className="flex items-start justify-between gap-2">
                            <span className="text-xs font-semibold text-[var(--muted)]">
                              {companyName}
                            </span>
                            <FavoriteButton
                              isFavorite={job.isFavorite}
                              onToggle={() => onToggleFavorite(job.id)}
                              disabled={isPending}
                              size={16}
                              className="-mr-1.5 -mt-1.5"
                            />
                          </div>

                          <Link
                            to={`/jobs/${job.id}`}
                            className="block font-display text-base font-medium leading-snug text-[var(--foreground)] hover:text-[var(--accent)]"
                          >
                            {job.title}
                          </Link>

                          <div className="flex flex-wrap items-center gap-1.5">
                            {postingInfo ? (
                              <span className={`pill text-[11px] ${toneClasses[postingInfo.tone]}`}>
                                <span className="pill-dot" />
                                {postingInfo.label}
                              </span>
                            ) : null}
                            {job.location ? (
                              <span className="truncate text-xs text-[var(--faint)]">
                                {job.location}
                              </span>
                            ) : null}
                          </div>
                        </div>

                        <div className="mt-3 flex items-center justify-between border-t border-[var(--border)]/60 pt-2.5 text-[11px] text-[var(--faint)]">
                          <span>
                            {job.appliedAt
                              ? `Applied ${formatDistanceToNow(new Date(job.appliedAt), { addSuffix: true })}`
                              : `Updated ${formatDistanceToNow(new Date(job.updatedAt), { addSuffix: true })}`}
                          </span>

                          <div className="flex items-center gap-1">
                            {onUpdateStatus && nextStage ? (
                              <button
                                type="button"
                                onClick={(e) => {
                                  e.preventDefault();
                                  e.stopPropagation();
                                  onUpdateStatus(job.id, nextStage);
                                }}
                                className="rounded px-1.5 py-0.5 text-[10px] font-semibold text-[var(--accent)] hover:bg-[var(--accent-soft)]"
                                title={`Advance to ${jobStatusPresentation(nextStage).label}`}
                              >
                                Advance →
                              </button>
                            ) : null}
                            {onUpdateStatus ? (
                              <select
                                value={job.status}
                                onClick={(e) => e.stopPropagation()}
                                onChange={(e) => {
                                  e.preventDefault();
                                  e.stopPropagation();
                                  onUpdateStatus(job.id, e.target.value as JobStatus);
                                }}
                                aria-label="Change stage"
                                className="cursor-pointer rounded bg-transparent p-0.5 text-[10px] font-medium text-[var(--muted)] hover:text-[var(--foreground)]"
                              >
                                {jobStatuses.map((st) => (
                                  <option key={st} value={st}>
                                    {jobStatusPresentation(st).label}
                                  </option>
                                ))}
                              </select>
                            ) : null}
                          </div>
                        </div>
                      </div>
                    );
                  })
                )}
              </div>
            </div>
          );
        })}
      </div>

      {archivedJobs.length > 0 ? (
        <section className="card p-4">
          <h4 className="mb-3 text-xs font-semibold uppercase tracking-wider text-[var(--muted)]">
            Closed or archived favorites ({archivedJobs.length})
          </h4>
          <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2 md:grid-cols-3">
            {archivedJobs.map(({ job, companyName }) => (
              <div
                key={job.id}
                className="flex items-center justify-between rounded-xl bg-[var(--surface-muted)] p-3 text-sm"
              >
                <div className="min-w-0 flex-1 pr-2">
                  <Link
                    to={`/jobs/${job.id}`}
                    className="truncate font-medium text-[var(--foreground)] hover:text-[var(--accent)] block"
                  >
                    {job.title}
                  </Link>
                  <p className="text-xs text-[var(--muted)]">
                    {companyName} · {jobStatusPresentation(job.status).label}
                  </p>
                </div>
                <FavoriteButton
                  isFavorite={job.isFavorite}
                  onToggle={() => onToggleFavorite(job.id)}
                  size={16}
                />
              </div>
            ))}
          </div>
        </section>
      ) : null}
    </div>
  );
}
