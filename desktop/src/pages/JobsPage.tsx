import { listen } from "@tauri-apps/api/event";
import { formatDistanceToNow } from "date-fns";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";

import { FavoriteButton } from "@/components/FavoriteButton";
import {
  ArchiveIcon,
  BriefcaseIcon,
  ChatIcon,
  KanbanIcon,
  ListIcon,
  SendIcon,
  StarIcon,
  statusIcons,
  TrashIcon,
  TrophyIcon,
} from "@/components/icons";
import { JobsBoardView } from "@/components/JobsBoardView";
import { NewRolesList } from "@/components/companies/NewRolesList";
import { api, type JobsRunnerProgress } from "@/lib/api";
import { roleCountLabel } from "@/lib/companies-ui";
import { jobStatuses, type JobListItem, type JobStatus, type WeeklyActivity } from "@/lib/schema";
import { isDesktopShell } from "@/lib/tauri";
import {
  jobSourceLabel,
  jobStatusPresentation,
  postingStateMatters,
  postingStatePresentation,
  toneClasses,
} from "@/lib/ui";

/** How many watch discoveries the Jobs page previews before deferring to Companies. */
const WATCH_PREVIEW_COUNT = 5;

export function JobsPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const status = searchParams.get("status") ?? undefined;
  const companyId = searchParams.get("companyId") ?? undefined;
  const postingState = searchParams.get("postingState") ?? undefined;
  const search = searchParams.get("search") ?? undefined;
  const isFavoriteFilter = searchParams.get("favorites") === "true";
  const isArchivedFilter = searchParams.get("archived") === "true";
  const viewMode = (searchParams.get("view") as "list" | "board" | null) ?? (isFavoriteFilter ? "board" : "list");

  const [jobs, setJobs] = useState<JobListItem[]>([]);
  const [newFromWatch, setNewFromWatch] = useState<JobListItem[]>([]);
  const [counts, setCounts] = useState<Record<string, number>>({ all: 0, favorites: 0 });
  const [activity, setActivity] = useState<WeeklyActivity | null>(null);
  const [companies, setCompanies] = useState<Array<{ id: string; name: string }>>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [checkingPostings, setCheckingPostings] = useState(false);
  const [checkProgress, setCheckProgress] = useState<JobsRunnerProgress | null>(null);
  const [checkError, setCheckError] = useState<string | null>(null);
  const [triagingId, setTriagingId] = useState<string | null>(null);
  const [triageError, setTriageError] = useState<string | null>(null);
  const [togglingFavId, setTogglingFavId] = useState<string | null>(null);
  const [jobToDelete, setJobToDelete] = useState<{ id: string; title: string; companyName: string } | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const checkingPostingsRef = useRef(false);
  const loadSequenceRef = useRef(0);

  const load = useCallback(async (opts?: { quiet?: boolean }) => {
    const requestKey = JSON.stringify({
      status,
      companyId,
      postingState,
      search,
      isFavorite: isFavoriteFilter,
      isArchived: isArchivedFilter,
    });
    const sequence = ++loadSequenceRef.current;
    if (!opts?.quiet) {
      setLoading(true);
    }
    setError(null);
    try {
      const [listResult, companiesResult, watchResult] = await Promise.all([
        api.listJobs({
          status,
          companyId,
          postingState,
          search,
          isFavorite: isFavoriteFilter ? true : undefined,
          isArchived: isArchivedFilter
            ? true
            : !status && !isFavoriteFilter
              ? false
              : undefined,
        }),
        api.listCompanies(),
        api.listJobs({ newFromWatch: true }),
      ]);
      if (sequence !== loadSequenceRef.current) return;
      if (
        JSON.stringify({
          status,
          companyId,
          postingState,
          search,
          isFavorite: isFavoriteFilter,
          isArchived: isArchivedFilter,
        }) !== requestKey
      ) {
        return;
      }
      setJobs(listResult.jobs);
      setCounts(listResult.counts);
      setActivity(listResult.weeklyActivity);
      setCompanies(companiesResult.companies.map((row) => row.company));
      setNewFromWatch(watchResult.jobs);
    } catch (err) {
      if (sequence !== loadSequenceRef.current) return;
      setError(err instanceof Error ? err.message : "Failed to load jobs");
    } finally {
      if (sequence === loadSequenceRef.current && !opts?.quiet) {
        setLoading(false);
      }
    }
  }, [status, companyId, postingState, search, isFavoriteFilter, isArchivedFilter]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!isDesktopShell()) {
      return;
    }

    let unlisten: (() => void) | undefined;

    void listen<JobsRunnerProgress>("jobs-runner-progress", (event) => {
      if (checkingPostingsRef.current) {
        setCheckProgress(event.payload);
      }
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!checkProgress || checkProgress.phase !== "done" || checkingPostings) {
      return;
    }
    const timer = window.setTimeout(() => setCheckProgress(null), 3000);
    return () => window.clearTimeout(timer);
  }, [checkProgress, checkingPostings]);

  async function checkAllPostings() {
    checkingPostingsRef.current = true;
    setCheckingPostings(true);
    setCheckError(null);
    setCheckProgress({ phase: "starting", message: "Checking job postings…" });
    try {
      await api.checkAllPostings();
      setCheckProgress({ phase: "done", message: "Posting check complete" });
      await load({ quiet: true });
    } catch (err) {
      setCheckError(err instanceof Error ? err.message : "Posting check failed");
      setCheckProgress(null);
    } finally {
      checkingPostingsRef.current = false;
      setCheckingPostings(false);
    }
  }

  async function triageWatchJob(jobId: string, action: "approve" | "dismiss") {
    setTriagingId(jobId);
    setTriageError(null);
    try {
      if (action === "approve") {
        await api.approveWatchJob(jobId);
      } else {
        await api.dismissWatchJob(jobId);
      }
      await load({ quiet: true });
    } catch (err) {
      setTriageError(err instanceof Error ? err.message : `Failed to ${action} watch job`);
    } finally {
      setTriagingId(null);
    }
  }

  async function handleToggleFavorite(jobId: string) {
    setTogglingFavId(jobId);
    // Optimistic UI update
    setJobs((prev) =>
      prev.map((item) =>
        item.job.id === jobId
          ? { ...item, job: { ...item.job, isFavorite: !item.job.isFavorite } }
          : item,
      ),
    );
    try {
      const res = await api.toggleFavorite(jobId);
      setJobs((prev) =>
        prev.map((item) => (item.job.id === jobId ? { ...item, job: res.item.job } : item)),
      );
      setCounts((prev) => ({
        ...prev,
        favorites: Math.max(0, (prev.favorites ?? 0) + (res.item.job.isFavorite ? 1 : -1)),
      }));
    } catch (err) {
      // Revert on error
      await load({ quiet: true });
    } finally {
      setTogglingFavId(null);
    }
  }

  async function handleUpdateStatus(jobId: string, nextStatus: JobStatus) {
    // Optimistic update
    setJobs((prev) =>
      prev.map((item) =>
        item.job.id === jobId ? { ...item, job: { ...item.job, status: nextStatus } } : item,
      ),
    );
    try {
      await api.updateJob(jobId, { status: nextStatus });
      await load({ quiet: true });
    } catch {
      await load({ quiet: true });
    }
  }

  async function handleToggleArchive(jobId: string, currentStatus: JobStatus) {
    try {
      if (currentStatus === "archived") {
        await api.unarchiveJob(jobId);
      } else {
        await api.archiveJob(jobId);
      }
      await load({ quiet: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to update archive status");
    }
  }

  async function confirmDeleteJob() {
    if (!jobToDelete) return;
    setDeletingId(jobToDelete.id);
    try {
      await api.deleteJob(jobToDelete.id);
      setJobs((prev) => prev.filter((item) => item.job.id !== jobToDelete.id));
      setJobToDelete(null);
      await load({ quiet: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete job");
    } finally {
      setDeletingId(null);
    }
  }

  useEffect(() => {
    if (!jobToDelete) return;
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" && !deletingId) {
        setJobToDelete(null);
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [jobToDelete, deletingId]);

  function setView(nextView: "list" | "board") {
    const next = new URLSearchParams(searchParams);
    next.set("view", nextView);
    setSearchParams(next);
  }

  const activeStatus = jobStatuses.find((value) => value === status);
  const heading = isFavoriteFilter
    ? "Favorites Board"
    : isArchivedFilter
      ? "Archived Postings"
      : activeStatus
        ? jobStatusPresentation(activeStatus).label
        : "All active jobs";
  const subtitle = isFavoriteFilter
    ? jobs.length === 0
      ? "No starred roles yet."
      : `${jobs.length} ${jobs.length === 1 ? "priority role" : "priority roles"} on your favorite board.`
    : isArchivedFilter
      ? jobs.length === 0
        ? "No archived roles."
        : `${jobs.length} ${jobs.length === 1 ? "archived role" : "archived roles"} saved for reference.`
      : jobs.length === 0
        ? "Nothing here yet."
        : `${jobs.length} ${jobs.length === 1 ? "role" : "roles"} on your radar.`;

  const isFiltered = Boolean(status || companyId || postingState || search || isFavoriteFilter || isArchivedFilter);

  function handleFilterSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const next = new URLSearchParams();
    const nextSearch = String(form.get("search") ?? "").trim();
    const nextPosting = String(form.get("postingState") ?? "");
    if (isFavoriteFilter) next.set("favorites", "true");
    if (isArchivedFilter) next.set("archived", "true");
    if (status) next.set("status", status);
    if (companyId) next.set("companyId", companyId);
    if (nextSearch) next.set("search", nextSearch);
    if (nextPosting) next.set("postingState", nextPosting);
    if (viewMode) next.set("view", viewMode);
    setSearchParams(next);
  }

  if (loading && jobs.length === 0) {
    return <p className="text-sm text-[var(--muted)]">Loading jobs…</p>;
  }

  if (error && jobs.length === 0) {
    return (
      <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
        {error}
      </p>
    );
  }

  const activeJobsCount = Math.max(0, (counts.all ?? 0) - (counts.archivedTotal ?? 0));

  return (
    <>
      <div className="grid gap-8 lg:grid-cols-[240px_1fr]">
        <aside className="space-y-6">
          <section className="card p-5">
            <h2 className="mb-4 text-sm font-semibold text-[var(--muted)]">Pipeline</h2>
            <div className="space-y-1">
              <SidebarLink
                to="/"
                active={!status && !isFavoriteFilter && !isArchivedFilter}
                label="All active"
                count={activeJobsCount}
              />
              <SidebarLink
                to="/?favorites=true"
                active={isFavoriteFilter}
                label="Favorites"
                Icon={StarIcon}
                count={counts.favorites ?? 0}
              />
              {(["wishlist", "applied", "interviewing", "offer"] as const).map((key) => (
                <SidebarLink
                  key={key}
                  to={`/?status=${key}`}
                  active={status === key && !isFavoriteFilter && !isArchivedFilter}
                  label={jobStatusPresentation(key).label}
                  count={counts[key] ?? 0}
                />
              ))}
              <SidebarLink
                to="/?archived=true"
                active={isArchivedFilter}
                label="Archived"
                Icon={ArchiveIcon}
                count={counts.archivedTotal ?? (counts.archived ?? 0)}
              />
            </div>
          </section>

          <section className="card p-5">
            <div className="mb-4 flex items-center justify-between">
              <h2 className="text-sm font-semibold text-[var(--muted)]">Companies</h2>
              <Link to="/companies" className="text-xs font-medium text-[var(--accent)]">
                Manage
              </Link>
            </div>
            <div className="space-y-1">
              {companies.map((company) => (
                <SidebarLink
                  key={company.id}
                  to={`/?companyId=${company.id}`}
                  active={companyId === company.id}
                  label={company.name}
                />
              ))}
              {companies.length === 0 ? (
                <p className="text-sm text-[var(--faint)]">No companies yet.</p>
              ) : null}
            </div>
          </section>
        </aside>

        <section className="space-y-5">
          <div className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
            <div>
              <div className="flex items-center gap-2.5">
                {isFavoriteFilter ? (
                  <span className="flex h-8 w-8 items-center justify-center rounded-xl bg-amber-100 text-amber-600 dark:bg-amber-950/40 dark:text-amber-400">
                    <StarIcon size={18} filled />
                  </span>
                ) : isArchivedFilter ? (
                  <span className="flex h-8 w-8 items-center justify-center rounded-xl bg-[var(--surface-muted)] text-[var(--muted)]">
                    <ArchiveIcon size={18} />
                  </span>
                ) : null}
                <h1 className="font-display text-3xl font-semibold tracking-tight">{heading}</h1>
              </div>
              <p className="mt-1 text-sm text-[var(--muted)]">
                {subtitle}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2 self-start md:self-auto">
              {/* View Switcher: List vs Board */}
              <div className="inline-flex rounded-xl border border-[var(--border)] bg-[var(--surface)] p-1 shadow-[var(--shadow-sm)]">
                <button
                  type="button"
                  onClick={() => setView("list")}
                  className={`inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition-colors ${
                    viewMode === "list"
                      ? "bg-[var(--accent-soft)] text-[var(--accent-ink)]"
                      : "text-[var(--muted)] hover:text-[var(--foreground)]"
                  }`}
                  title="List view"
                >
                  <ListIcon size={14} />
                  List
                </button>
                <button
                  type="button"
                  onClick={() => setView("board")}
                  className={`inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition-colors ${
                    viewMode === "board"
                      ? "bg-[var(--accent-soft)] text-[var(--accent-ink)]"
                      : "text-[var(--muted)] hover:text-[var(--foreground)]"
                  }`}
                  title="Board view"
                >
                  <KanbanIcon size={14} />
                  Board
                </button>
              </div>

              <Link to="/jobs/new" className="btn btn-primary">
                + Add job
              </Link>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-3">
            <form onSubmit={handleFilterSubmit} className="flex flex-1 flex-wrap gap-2">
              <input
                type="search"
                name="search"
                defaultValue={search ?? ""}
                placeholder="Search jobs…"
                aria-label="Search jobs"
                className="field max-w-xs"
              />
              <select
                name="postingState"
                defaultValue={postingState ?? ""}
                aria-label="Posting status"
                className="field max-w-[160px]"
              >
                <option value="">All postings</option>
                <option value="active">Active</option>
                <option value="inactive">Inactive</option>
              </select>
              <button type="submit" className="btn btn-secondary">
                Filter
              </button>
              {isFiltered ? (
                <Link to="/" className="btn btn-ghost">
                  Clear
                </Link>
              ) : null}
            </form>

            <button
              type="button"
              onClick={() => void checkAllPostings()}
              disabled={checkingPostings}
              className="btn btn-secondary"
            >
              {checkingPostings ? <span className="spinner" /> : null}
              {checkingPostings ? "Checking…" : "Check postings"}
            </button>
          </div>

          {checkProgress && (checkingPostings || checkProgress.phase === "done") ? (
            <div
              role="status"
              aria-live="polite"
              className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--surface)] px-4 py-3 text-xs text-[var(--muted)] shadow-[var(--shadow-sm)]"
            >
              <span>{checkProgress.message}</span>
              {checkProgress.total && checkProgress.total > 0 ? (
                <span className="font-display font-medium text-[var(--foreground)]">
                  {checkProgress.current ?? 0} / {checkProgress.total}
                </span>
              ) : null}
            </div>
          ) : null}

          {checkError ? (
            <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
              {checkError}
            </p>
          ) : null}

          {!isFiltered ? (
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
              <StatCard
                label="Tracking"
                value={activeJobsCount}
                Icon={BriefcaseIcon}
                tint="bg-[var(--accent-soft)] text-[var(--accent-ink)]"
              />
              <StatCard
                label="Applied"
                value={counts.applied ?? 0}
                Icon={SendIcon}
                tint="bg-[var(--blue-soft)] text-[var(--blue-ink)]"
              />
              <StatCard
                label="Interviewing"
                value={counts.interviewing ?? 0}
                Icon={ChatIcon}
                tint="bg-[var(--amber-soft)] text-[var(--amber-ink)]"
              />
              <StatCard
                label="Offers"
                value={counts.offer ?? 0}
                Icon={TrophyIcon}
                tint="bg-[var(--green-soft)] text-[var(--green-ink)]"
              />
            </div>
          ) : null}

          {!isFiltered && activity ? <ActivityLine activity={activity} /> : null}

          {!isFiltered && newFromWatch.length > 0 ? (
            <section className="card p-5">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                  <h3 className="font-display text-lg font-medium">
                    {newFromWatch.length > 0
                      ? `${roleCountLabel(newFromWatch.length)} from your watches`
                      : "0 new roles from your watches"}
                  </h3>
                  <p className="mt-0.5 text-sm text-[var(--muted)]">
                    {newFromWatch.length > 0
                      ? "Not on your list yet. Save the ones worth tracking."
                      : "Your watches are up to date. No new matches found."}
                  </p>
                </div>
                {newFromWatch.length > WATCH_PREVIEW_COUNT ? (
                  <Link to="/companies" className="btn btn-secondary btn-sm">
                    Browse all {newFromWatch.length}
                  </Link>
                ) : null}
              </div>
              {triageError ? (
                <p
                  role="alert"
                  className="mt-3 rounded-lg bg-[var(--danger-soft)] px-2.5 py-1.5 text-sm text-[var(--danger)]"
                >
                  {triageError}
                </p>
              ) : null}
              <div className="mt-3">
                <NewRolesList
                  roles={newFromWatch.slice(0, WATCH_PREVIEW_COUNT)}
                  showCompany
                  onTriage={(jobId, action) =>
                    void triageWatchJob(jobId, action === "save" ? "approve" : "dismiss")
                  }
                  isPending={(jobId) => triagingId === jobId}
                />
              </div>
            </section>
          ) : null}

          {jobs.length === 0 ? (
            <div className="card flex flex-col items-center justify-center gap-3 p-12 text-center">
              <span className="flex h-12 w-12 items-center justify-center rounded-2xl bg-[var(--surface-muted)] text-[var(--muted)]">
                {isFavoriteFilter ? (
                  <StarIcon size={24} filled />
                ) : isArchivedFilter ? (
                  <ArchiveIcon size={24} />
                ) : (
                  <BriefcaseIcon size={24} />
                )}
              </span>
              {isFiltered ? (
                <>
                  <p className="font-display text-lg text-[var(--foreground)]">No matching jobs</p>
                  <p className="text-sm text-[var(--muted)]">
                    Try adjusting your search or clearing active filters.
                  </p>
                  <Link to="/" className="btn btn-secondary mt-1">
                    Clear filters
                  </Link>
                </>
              ) : isFavoriteFilter ? (
                <>
                  <p className="font-display text-lg text-[var(--foreground)]">No favorites yet</p>
                  <p className="max-w-sm text-sm text-[var(--muted)]">
                    Click the star on any job to pin it here and track your highest-priority applications across a dedicated Kanban board.
                  </p>
                  <Link to="/" className="btn btn-secondary mt-1">
                    Browse all jobs
                  </Link>
                </>
              ) : isArchivedFilter ? (
                <>
                  <p className="font-display text-lg text-[var(--foreground)]">No archived jobs</p>
                  <p className="max-w-sm text-sm text-[var(--muted)]">
                    Jobs that you archive or close will be saved here for reference.
                  </p>
                  <Link to="/" className="btn btn-secondary mt-1">
                    Browse active jobs
                  </Link>
                </>
              ) : (
                <>
                  <p className="font-display text-lg text-[var(--foreground)]">Your board is empty</p>
                  <p className="max-w-sm text-sm text-[var(--muted)]">
                    Paste a posting URL and Job Tracker will keep an eye on it for you.
                  </p>
                  <Link to="/jobs/new" className="btn btn-primary mt-1">
                    Add your first job
                  </Link>
                </>
              )}
            </div>
          ) : viewMode === "board" ? (
            <JobsBoardView
              jobs={jobs}
              onToggleFavorite={handleToggleFavorite}
              onUpdateStatus={handleUpdateStatus}
              onToggleArchive={handleToggleArchive}
              onDeleteJob={setJobToDelete}
              isPendingFavorite={(id) => togglingFavId === id}
            />
          ) : (
            <ul className="space-y-3">
              {jobs.map(({ job, companyName }) => {
                const statusInfo = jobStatusPresentation(job.status);
                const postingInfo = postingStateMatters(job.status)
                  ? postingStatePresentation(job.postingState)
                  : null;
                const source = jobSourceLabel(job.source);
                const StageIcon = statusIcons[job.status];
                return (
                  <li key={job.id}>
                    <Link
                      to={`/jobs/${job.id}`}
                      className="card group block p-5 transition-shadow hover:shadow-[var(--shadow-md)]"
                    >
                      <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
                        <div className="space-y-2">
                          <div className="flex flex-wrap items-center gap-2">
                            <span className={`pill ${toneClasses[statusInfo.tone]}`}>
                              <StageIcon size={12} />
                              {statusInfo.label}
                            </span>
                            {postingInfo ? (
                              <span className={`pill ${toneClasses[postingInfo.tone]}`}>
                                <span className="pill-dot" />
                                {postingInfo.label}
                              </span>
                            ) : null}
                            {job.isNewFromWatch ? (
                              <span className="pill bg-[var(--accent-soft)] text-[var(--accent-ink)]">
                                New
                              </span>
                            ) : null}
                          </div>
                          <p className="font-display text-lg font-medium leading-snug">{job.title}</p>
                          <p className="text-sm text-[var(--muted)]">
                            {companyName}
                            {job.appliedAt
                              ? ` · Applied ${formatDistanceToNow(new Date(job.appliedAt), {
                                  addSuffix: true,
                                })}`
                              : ""}
                          </p>
                          {source ? <p className="text-xs text-[var(--faint)]">{source}</p> : null}
                        </div>
                        <div className="flex items-center gap-2 shrink-0 self-start md:self-auto">
                          <button
                            type="button"
                            onClick={(e) => {
                              e.preventDefault();
                              e.stopPropagation();
                              void handleToggleArchive(job.id, job.status);
                            }}
                            className="rounded-lg p-1.5 text-[var(--muted)] hover:bg-[var(--surface-muted)] hover:text-[var(--accent)] transition-colors"
                            title={job.status === "archived" ? "Restore to active" : "Archive role"}
                          >
                            <ArchiveIcon size={16} />
                          </button>
                          <button
                            type="button"
                            onClick={(e) => {
                              e.preventDefault();
                              e.stopPropagation();
                              setJobToDelete({ id: job.id, title: job.title, companyName });
                            }}
                            className="rounded-lg p-1.5 text-[var(--muted)] hover:bg-[var(--danger-soft)] hover:text-[var(--danger)] transition-colors"
                            title="Delete role"
                          >
                            <TrashIcon size={16} />
                          </button>
                          <FavoriteButton
                            isFavorite={job.isFavorite}
                            onToggle={() => handleToggleFavorite(job.id)}
                            disabled={togglingFavId === job.id}
                          />
                          <p className="text-xs text-[var(--faint)] ml-1">
                            Updated{" "}
                            {formatDistanceToNow(new Date(job.updatedAt), { addSuffix: true })}
                          </p>
                        </div>
                      </div>
                    </Link>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      </div>

      {jobToDelete ? (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-sm"
          role="presentation"
        >
          <section
            role="dialog"
            aria-modal="true"
            aria-labelledby="delete-job-dialog-title"
            className="card w-full max-w-md p-6 shadow-[var(--shadow-lg)] space-y-4"
          >
            <div className="flex items-center gap-3">
              <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-[var(--danger-soft)] text-[var(--danger)]">
                <TrashIcon size={20} />
              </span>
              <div>
                <h3
                  id="delete-job-dialog-title"
                  className="font-display text-lg font-semibold text-[var(--foreground)]"
                >
                  Delete job posting?
                </h3>
                <p className="text-xs text-[var(--muted)]">Permanent removal from database and CSV</p>
              </div>
            </div>

            <p className="text-sm text-[var(--muted)] leading-relaxed">
              Are you sure you want to delete <strong className="text-[var(--foreground)]">{jobToDelete.title}</strong> at{" "}
              <strong className="text-[var(--foreground)]">{jobToDelete.companyName}</strong>?
            </p>
            <p className="text-xs text-[var(--faint)]">
              This will permanently delete this job, its timeline history, and document attachment links from your local database and the synchronized CSV file. This cannot be undone.
            </p>

            <div className="flex items-center justify-end gap-2 pt-2">
              <button
                type="button"
                onClick={() => setJobToDelete(null)}
                disabled={deletingId !== null}
                className="btn btn-secondary"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => void confirmDeleteJob()}
                disabled={deletingId !== null}
                className="btn bg-[var(--danger)] text-white hover:opacity-90 flex items-center gap-1.5"
              >
                {deletingId !== null ? <span className="spinner" /> : <TrashIcon size={14} />}
                <span>{deletingId !== null ? "Deleting…" : "Delete permanently"}</span>
              </button>
            </div>
          </section>
        </div>
      ) : null}
    </>
  );
}

function ActivityLine({ activity }: { activity: WeeklyActivity }) {
  const peak = Math.max(1, ...activity.days.map((day) => day.count));
  return (
    <div className="card flex items-center justify-between gap-6 px-5 py-3">
      <div className="flex items-baseline gap-2">
        <span className="text-sm text-[var(--muted)]">This week</span>
        <span className="font-display text-lg leading-none">{activity.total}</span>
        <span className="text-xs text-[var(--faint)]">
          {activity.total === 1 ? "update" : "updates"}
        </span>
      </div>
      <div className="flex h-9 items-end gap-1.5">
        {activity.days.map((day, index) => {
          const height = day.count === 0 ? 4 : 8 + Math.round((day.count / peak) * 20);
          return (
            <span
              key={day.key}
              className="flex flex-col items-center gap-1"
              title={`${day.count} ${day.count === 1 ? "update" : "updates"}`}
            >
              <span
                className={`w-1.5 rounded-full ${
                  day.count > 0 ? "bg-[var(--accent)]" : "bg-[var(--border)]"
                } ${day.isToday && day.count === 0 ? "bg-[var(--accent-soft)]" : ""}`}
                style={{ height }}
              />
              <span
                className={`text-[9px] leading-none ${
                  day.isToday
                    ? "font-semibold text-[var(--accent-ink)]"
                    : "text-[var(--faint)]"
                }`}
              >
                {day.label}
                <span className="sr-only"> day {index + 1} of 7</span>
              </span>
            </span>
          );
        })}
      </div>
    </div>
  );
}

function StatCard({
  label,
  value,
  Icon,
  tint,
}: {
  label: string;
  value: number;
  Icon: (props: { size?: number; className?: string }) => React.JSX.Element;
  tint: string;
}) {
  return (
    <div className="card flex items-center gap-3 p-4">
      <span
        className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-xl ${tint}`}
      >
        <Icon size={18} />
      </span>
      <div className="min-w-0">
        <div className="font-display text-2xl leading-none">{value}</div>
        <div className="mt-1 truncate text-xs text-[var(--muted)]">{label}</div>
      </div>
    </div>
  );
}

function SidebarLink({
  to,
  label,
  count,
  active,
  Icon,
}: {
  to: string;
  label: string;
  count?: number;
  active?: boolean;
  Icon?: (props: { size?: number; className?: string; filled?: boolean }) => React.JSX.Element;
}) {
  return (
    <Link
      to={to}
      className={`flex items-center justify-between rounded-lg px-3 py-2 text-sm transition-colors ${
        active
          ? "bg-[var(--accent-soft)] font-medium text-[var(--accent-ink)]"
          : "text-[var(--foreground)] hover:bg-[var(--surface-muted)]"
      }`}
    >
      <span className="flex items-center gap-2">
        {Icon ? <Icon size={14} className={active ? "text-[var(--accent-ink)]" : "text-[var(--faint)]"} filled={active} /> : null}
        <span>{label}</span>
      </span>
      {typeof count === "number" ? (
        <span
          className={`font-display text-sm ${
            active ? "text-[var(--accent-ink)]" : "text-[var(--faint)]"
          }`}
        >
          {count}
        </span>
      ) : null}
    </Link>
  );
}
