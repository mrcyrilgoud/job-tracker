import { formatDistanceToNow } from "date-fns";
import { useMemo, useState } from "react";
import { Link } from "react-router-dom";

import { FavoriteButton } from "@/components/FavoriteButton";
import { ArchiveIcon, statusIcons, TrashIcon } from "@/components/icons";
import { jobStatuses, type JobListItem, type JobStatus } from "@/lib/schema";
import {
  jobStatusPresentation,
  nextActiveJobStage,
  postingStateMatters,
  postingStatePresentation,
  toneClasses,
} from "@/lib/ui";

const BOARD_COLUMNS: JobStatus[] = ["wishlist", "applied", "interviewing", "offer"];

const BOARD_PAGE_SIZE = 25;

export function JobsBoardView({
  jobs,
  onToggleFavorite,
  onUpdateStatus,
  onToggleArchive,
  onDeleteJob,
  isPendingFavorite,
}: {
  jobs: JobListItem[];
  onToggleFavorite: (jobId: string) => void;
  onUpdateStatus?: (jobId: string, nextStatus: JobStatus) => void;
  onToggleArchive?: (jobId: string, currentStatus: JobStatus) => void;
  onDeleteJob?: (job: { id: string; title: string; companyName: string }) => void;
  isPendingFavorite?: (jobId: string) => boolean;
}) {
  const [expandedStages, setExpandedStages] = useState<Set<JobStatus>>(new Set());
  const jobsByStage = useMemo(() => {
    const grouped: Record<JobStatus, JobListItem[]> = {
      wishlist: [],
      applied: [],
      interviewing: [],
      offer: [],
      rejected: [],
      withdrawn: [],
      closed: [],
      archived: [],
    };
    for (const item of jobs) {
      grouped[item.job.status].push(item);
    }
    return grouped;
  }, [jobs]);

  // Check if there are closed / archived jobs in the current set
  const archivedJobs = useMemo(
    () => jobs.filter((item) => item.job.status === "rejected" || item.job.status === "withdrawn" || item.job.status === "closed" || item.job.status === "archived"),
    [jobs],
  );

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-4">
        {BOARD_COLUMNS.map((stage) => {
          const stageInfo = jobStatusPresentation(stage);
          const StageIcon = statusIcons[stage];
          const stageJobs = jobsByStage[stage] ?? [];
          const isExpanded = expandedStages.has(stage);
          const visibleJobs = isExpanded ? stageJobs : stageJobs.slice(0, BOARD_PAGE_SIZE);
          const remainingJobs = stageJobs.length - visibleJobs.length;
          const nextStage = nextActiveJobStage(stage);

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
                  visibleJobs.map(({ job, companyName }) => {
                    const postingInfo = postingStateMatters(job.status)
                      ? postingStatePresentation(job.postingState, job.lastCheckedAt)
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
                            <div className="flex items-center gap-1 -mr-1.5 -mt-1.5">
                              {onToggleArchive ? (
                                <button
                                  type="button"
                                  onClick={(e) => {
                                    e.preventDefault();
                                    e.stopPropagation();
                                    onToggleArchive(job.id, job.status);
                                  }}
                                  className="rounded p-1 text-[var(--muted)] hover:bg-[var(--surface)] hover:text-[var(--accent)] transition-colors opacity-0 group-hover:opacity-100"
                                  title={job.status === "archived" ? "Restore to active" : "Archive role"}
                                >
                                  <ArchiveIcon size={14} />
                                </button>
                              ) : null}
                              {onDeleteJob ? (
                                <button
                                  type="button"
                                  onClick={(e) => {
                                    e.preventDefault();
                                    e.stopPropagation();
                                    onDeleteJob({ id: job.id, title: job.title, companyName });
                                  }}
                                  className="rounded p-1 text-[var(--muted)] hover:bg-[var(--danger-soft)] hover:text-[var(--danger)] transition-colors opacity-0 group-hover:opacity-100"
                                  title="Delete role"
                                >
                                  <TrashIcon size={14} />
                                </button>
                              ) : null}
                              <FavoriteButton
                                isFavorite={job.isFavorite}
                                onToggle={() => onToggleFavorite(job.id)}
                                disabled={isPending}
                                size={16}
                              />
                            </div>
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

                        <div className="mt-3 flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border)]/60 pt-2.5 text-xs text-[var(--faint)]">
                          <span className="mr-auto">
                            {job.appliedAt
                              ? `Applied ${formatDistanceToNow(new Date(job.appliedAt), { addSuffix: true })}`
                              : `Updated ${formatDistanceToNow(new Date(job.updatedAt), { addSuffix: true })}`}
                          </span>

                          <div className="flex items-center gap-2">
                            {onUpdateStatus && nextStage ? (
                              <button
                                type="button"
                                onClick={(e) => {
                                  e.preventDefault();
                                  e.stopPropagation();
                                  onUpdateStatus(job.id, nextStage);
                                }}
                                className="rounded-lg bg-[var(--accent-soft)] px-2.5 py-1.5 text-xs font-semibold text-[var(--accent-ink)] transition-colors hover:bg-[var(--accent)] hover:text-white"
                              >
                                Move to {jobStatusPresentation(nextStage).label}
                              </button>
                            ) : null}
                            {onUpdateStatus ? (
                              <label className="flex items-center gap-1.5 text-xs font-medium text-[var(--muted)]">
                                <span>Move to</span>
                                <select
                                  value={job.status}
                                  onClick={(e) => e.stopPropagation()}
                                  onChange={(e) => {
                                    e.preventDefault();
                                    e.stopPropagation();
                                    onUpdateStatus(job.id, e.target.value as JobStatus);
                                  }}
                                  aria-label="Move to another stage"
                                  className="cursor-pointer rounded-lg border border-[var(--border)] bg-[var(--surface)] px-2 py-1.5 text-xs font-medium text-[var(--foreground)] hover:border-[var(--accent)]"
                                >
                                  {jobStatuses.map((st) => (
                                    <option key={st} value={st}>
                                      {jobStatusPresentation(st).label}
                                    </option>
                                  ))}
                                </select>
                              </label>
                            ) : null}
                          </div>
                        </div>
                      </div>
                    );
                  })
                )}
              </div>
              {remainingJobs > 0 ? (
                <button
                  type="button"
                  onClick={() =>
                    setExpandedStages((current) => new Set(current).add(stage))
                  }
                  className="mt-3 rounded-lg border border-[var(--border)] px-3 py-2 text-xs font-semibold text-[var(--muted)] hover:bg-[var(--surface)] hover:text-[var(--foreground)]"
                >
                  Show {remainingJobs} more
                </button>
              ) : null}
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
                className="group flex items-center justify-between rounded-xl bg-[var(--surface-muted)] p-3 text-sm"
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
                <div className="flex items-center gap-1">
                  {onToggleArchive ? (
                    <button
                      type="button"
                      onClick={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        onToggleArchive(job.id, job.status);
                      }}
                      className="rounded p-1 text-[var(--muted)] hover:bg-[var(--surface)] hover:text-[var(--accent)] transition-colors opacity-0 group-hover:opacity-100"
                      title={job.status === "archived" ? "Restore to active" : "Archive role"}
                    >
                      <ArchiveIcon size={14} />
                    </button>
                  ) : null}
                  {onDeleteJob ? (
                    <button
                      type="button"
                      onClick={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        onDeleteJob({ id: job.id, title: job.title, companyName });
                      }}
                      className="rounded p-1 text-[var(--muted)] hover:bg-[var(--danger-soft)] hover:text-[var(--danger)] transition-colors opacity-0 group-hover:opacity-100"
                      title="Delete role"
                    >
                      <TrashIcon size={14} />
                    </button>
                  ) : null}
                  <FavoriteButton
                    isFavorite={job.isFavorite}
                    onToggle={() => onToggleFavorite(job.id)}
                    size={16}
                  />
                </div>
              </div>
            ))}
          </div>
        </section>
      ) : null}
    </div>
  );
}
