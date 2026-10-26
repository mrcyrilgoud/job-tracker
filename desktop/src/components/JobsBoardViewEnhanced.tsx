import { formatDistanceToNow } from "date-fns";
import { useMemo, useState } from "react";
import { Link } from "react-router-dom";

import { DragIcon } from "@/components/icons";
import { StatusQuickChange } from "@/components/StatusQuickChange";
import { DragDropStatusUpdate } from "@/components/DragDropStatusUpdate";
import { FavoriteButton } from "@/components/FavoriteButton";
import { ArchiveIcon, statusIcons, TrashIcon } from "@/components/icons";
import { jobStatuses, type JobListSummary, type JobStatus } from "@/lib/schema";
import {
  jobStatusPresentation,
  nextActiveJobStage,
  postingStateMatters,
  postingStatePresentation,
  toneClasses,
} from "@/lib/ui";

const BOARD_COLUMNS: JobStatus[] = ["wishlist", "applied", "interviewing", "offer"];
const BOARD_PAGE_SIZE = 25;

interface JobsBoardViewEnhancedProps {
  jobs: JobListSummary[];
  onToggleFavorite: (jobId: string) => void;
  onUpdateStatus: (jobId: string, nextStatus: JobStatus) => void;
  onToggleArchive?: (jobId: string, currentStatus: JobStatus) => void;
  onDeleteJob?: (job: { id: string; title: string; companyName: string }) => void;
  selection?: {
    selectedIds: ReadonlySet<string>;
    disabled: boolean;
    onToggle: (jobId: string) => void;
  };
  isPendingFavorite?: (jobId: string) => boolean;
}

export function JobsBoardViewEnhanced({
  jobs,
  onToggleFavorite,
  onUpdateStatus,
  onToggleArchive,
  onDeleteJob,
  selection,
  isPendingFavorite,
}: JobsBoardViewEnhancedProps) {
  const [expandedStages, setExpandedStages] = useState<Set<JobStatus>>(new Set());
  const [showKeyboardHelp, setShowKeyboardHelp] = useState(false);

  const jobsByStage = useMemo(() => {
    const grouped: Record<JobStatus, JobListSummary[]> = {
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

  const handleQuickStatusChange = (jobId: string, newStatus: JobStatus) => {
    onUpdateStatus(jobId, newStatus);
  };

  const enhancedToneClasses = {
    violet: "status-enhanced-wishlist",
    blue: "status-enhanced-applied",
    amber: "status-enhanced-interviewing",
    green: "status-enhanced-offer",
    stone: "status-enhanced-stone",
    danger: "bg-[var(--danger-soft)] text-[var(--danger)]",
  };

  return (
    <DragDropStatusUpdate
      jobs={jobs}
      onStatusChange={onUpdateStatus}
    >
      {({ isDragging, dragOverStatus, handleDragStart, handleDragOver, handleDrop, handleDragEnd }) => (
        <div className="space-y-6">
          {/* Quick actions toolbar */}
          <div className="card p-4 mb-2">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => setShowKeyboardHelp(true)}
                  className="btn btn-secondary btn-sm inline-flex items-center gap-1.5"
                >
                  <span className="text-xs">⌘K</span>
                  <span>Quick Actions</span>
                </button>
                <div className="text-xs text-[var(--muted)]">
                  Drag jobs between columns or use number keys (1-5)
                </div>
              </div>
              {isDragging && (
                <div className="flex items-center gap-2 text-sm text-[var(--accent-ink)] animate-pulse">
                  <DragIcon size={14} />
                  <span>Drag to another column to change status</span>
                </div>
              )}
            </div>
          </div>

          <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-4">
            {BOARD_COLUMNS.map((stage) => {
              const stageInfo = jobStatusPresentation(stage);
              const StageIcon = statusIcons[stage];
              const stageJobs = jobsByStage[stage] ?? [];
              const isExpanded = expandedStages.has(stage);
              const visibleJobs = isExpanded ? stageJobs : stageJobs.slice(0, BOARD_PAGE_SIZE);
              const remainingJobs = stageJobs.length - visibleJobs.length;
              const nextStage = nextActiveJobStage(stage);
              const isDragTarget = dragOverStatus === stage;

              return (
                <div
                  key={stage}
                  className={`status-column flex min-h-[450px] flex-col rounded-2xl border-2 ${
                    isDragTarget ? "drag-target border-[var(--accent)]" : "border-[var(--border)]"
                  } ${enhancedToneClasses[stageInfo.tone]} p-3.5 transition-all duration-300`}
                  onDragOver={(e) => handleDragOver(e, stage)}
                  onDragLeave={(e) => e.currentTarget.classList.remove("drag-over-active")}
                  onDrop={(e) => handleDrop(e, stage)}
                >
                  <div className="mb-3.5 flex items-center justify-between px-1">
                    <div className="flex items-center gap-2">
                      <span className={`flex h-8 w-8 items-center justify-center rounded-xl ${enhancedToneClasses[stageInfo.tone]} shadow-sm`}>
                        <StageIcon size={16} />
                      </span>
                      <div>
                        <h3 className="font-display text-base font-semibold">
                          {stageInfo.label}
                        </h3>
                        <p className="text-xs text-[var(--muted)] mt-0.5">
                          {stageJobs.length} {stageJobs.length === 1 ? "role" : "roles"}
                        </p>
                      </div>
                    </div>
                    <div className="flex items-center gap-2">
                      <StatusQuickChange
                        currentStatus={stage}
                        onStatusChange={(newStatus) => {
                          // This would change all jobs in this column
                          console.log(`Change all ${stage} jobs to ${newStatus}`);
                        }}
                        size="sm"
                        showLabel={false}
                      />
                    </div>
                  </div>

                  <div className="flex-1 space-y-2.5 overflow-y-auto">
                    {stageJobs.length === 0 ? (
                      <div 
                        className={`flex h-32 flex-col items-center justify-center rounded-xl border-2 border-dashed ${
                          isDragTarget ? "border-[var(--accent)] bg-[var(--accent-soft)]/20" : "border-[var(--border)]"
                        } p-4 text-center transition-all duration-300`}
                        onDragOver={(e) => handleDragOver(e, stage)}
                        onDrop={(e) => handleDrop(e, stage)}
                      >
                        {isDragTarget ? (
                          <>
                            <div className="text-[var(--accent)] animate-bounce mb-2">↓</div>
                            <p className="text-sm font-medium text-[var(--accent-ink)]">Drop here to move job</p>
                          </>
                        ) : (
                          <>
                            <p className="text-sm text-[var(--faint)]">No roles here yet</p>
                            <p className="text-xs text-[var(--faint)] mt-1">Drag a job here to add</p>
                          </>
                        )}
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
                            draggable
                            onDragStart={(e) => handleDragStart(e, job.id, job.status)}
                            onDragEnd={handleDragEnd}
                            className="card group relative flex flex-col justify-between p-4 transition-all duration-150 hover:-translate-y-0.5 hover:shadow-[var(--shadow-md)] cursor-move"
                          >
                            <div className="space-y-2">
                              <div className="flex items-start justify-between gap-2">
                                <div className="flex items-center gap-2 flex-1">
                                  <div className="drag-handle p-1 -ml-1">
                                    <DragIcon size={14} />
                                  </div>
                                  <span className="text-xs font-semibold text-[var(--muted)] truncate">
                                    {companyName}
                                  </span>
                                </div>
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
                                  <span className={`pill text-[11px] ${enhancedToneClasses[postingInfo.tone]}`}>
                                    <span className="pill-dot" />
                                    {postingInfo.label}
                                  </span>
                                ) : null}
                                {job.location ? (
                                  <span className="truncate text-xs text-[var(--faint)]">
                                    {job.location}
                                  </span>
                                ) : null}
                                {job.appeal != null ? (
                                  <span className="text-xs text-[var(--muted)]" title="Appeal (1–5, 5=best)">
                                    ⭐ {job.appeal}
                                  </span>
                                ) : null}
                                {job.applicationEase != null ? (
                                  <span
                                    className="text-xs text-[var(--muted)]"
                                    title="Application ease (1–5, 5=easiest)"
                                  >
                                    📝 {job.applicationEase}
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
                                <StatusQuickChange
                                  currentStatus={job.status}
                                  onStatusChange={(newStatus) => handleQuickStatusChange(job.id, newStatus)}
                                  size="sm"
                                  showLabel={false}
                                />
                                {onUpdateStatus && nextStage ? (
                                  <button
                                    type="button"
                                    onClick={(e) => {
                                      e.preventDefault();
                                      e.stopPropagation();
                                      onUpdateStatus(job.id, nextStage);
                                    }}
                                    className="rounded-lg bg-[var(--accent-soft)] px-2.5 py-1.5 text-xs font-semibold text-[var(--accent-ink)] transition-colors hover:bg-[var(--accent)] hover:text-white hover:scale-105"
                                  >
                                    →
                                  </button>
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
                      className="mt-3 rounded-lg border border-[var(--border)] px-3 py-2 text-xs font-semibold text-[var(--muted)] hover:bg-[var(--surface)] hover:text-[var(--foreground)] hover:border-[var(--accent)] transition-all"
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
              <div className="flex items-center justify-between mb-3">
                <h4 className="text-xs font-semibold uppercase tracking-wider text-[var(--muted)]">
                  Closed or archived favorites ({archivedJobs.length})
                </h4>
                <StatusQuickChange
                  currentStatus="archived"
                  onStatusChange={(newStatus) => {
                    console.log(`Change all archived jobs to ${newStatus}`);
                  }}
                  size="sm"
                />
              </div>
              <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2 md:grid-cols-3">
                {archivedJobs.map(({ job, companyName }) => (
                  <div
                    key={job.id}
                    className={`group archived-selection-row flex items-center justify-between gap-2 rounded-xl p-3 text-sm transition-colors ${
                      selection?.selectedIds.has(job.id)
                        ? "is-selected bg-[var(--accent-soft)] ring-1 ring-[var(--accent)]"
                        : "bg-[var(--surface-muted)]"
                    }`}
                  >
                    {selection ? (
                      <input
                        type="checkbox"
                        checked={selection.selectedIds.has(job.id)}
                        disabled={selection.disabled}
                        onChange={() => selection.onToggle(job.id)}
                        onClick={(event) => event.stopPropagation()}
                        aria-label={`Select ${job.title} at ${companyName}`}
                        className="archived-selection-checkbox h-4 w-4 shrink-0 cursor-pointer accent-[var(--accent)] disabled:cursor-wait"
                      />
                    ) : null}
                    <div className="min-w-0 flex-1 pr-2">
                      <Link
                        to={`/jobs/${job.id}`}
                        className="truncate font-medium text-[var(--foreground)] hover:text-[var(--accent)] block"
                      >
                        {job.title}
                      </Link>
                      <p className="text-xs text-[var(--muted)]">
                        {companyName} · {jobStatusPresentation(job.status).label}
                        {job.appeal != null ? ` · ⭐ ${job.appeal}` : ""}
                        {job.applicationEase != null ? ` · 📝 ${job.applicationEase}` : ""}
                      </p>
                    </div>
                    <div className="flex items-center gap-1">
                      <StatusQuickChange
                        currentStatus={job.status}
                        onStatusChange={(newStatus) => handleQuickStatusChange(job.id, newStatus)}
                        size="sm"
                        showLabel={false}
                      />
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

          {/* Quick tips footer */}
          <div className="card p-4 mt-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="space-y-1">
                <h5 className="text-sm font-semibold text-[var(--foreground)]">
                  💡 Quick Status Tips
                </h5>
                <ul className="text-xs text-[var(--muted)] space-y-0.5">
                  <li>• Drag jobs between columns to change status</li>
                  <li>• Press 1-5 keys for quick status changes (when not typing)</li>
                  <li>• Click the status badge on any job for quick options</li>
                  <li>• Use the → button to move to the next stage</li>
                </ul>
              </div>
              <div className="flex items-center gap-2">
                <kbd className="keyboard-shortcut">1</kbd>
                <kbd className="keyboard-shortcut">2</kbd>
                <kbd className="keyboard-shortcut">3</kbd>
                <kbd className="keyboard-shortcut">4</kbd>
                <kbd className="keyboard-shortcut">5</kbd>
                <span className="text-xs text-[var(--muted)]">= Quick status</span>
              </div>
            </div>
          </div>
        </div>
      )}
    </DragDropStatusUpdate>
  );
}