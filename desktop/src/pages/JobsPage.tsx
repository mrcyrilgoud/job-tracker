import { formatDistanceToNow } from "date-fns";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";

import { APPEAL_SCALE_LABEL, AppealSelect } from "@/components/AppealSelect";
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
import { api } from "@/lib/api";
import { filterCompaniesBySearch, filterDraftFromUrl } from "@/lib/job-filters";
import { jobStatuses, type JobListSummary, type JobPageCursor, type JobStatus, type WeeklyActivity } from "@/lib/schema";
import { useRunMonitorActions } from "@/lib/RunMonitorContext";
import {
  jobSourceLabel,
  jobStatusPresentation,
  postingStateMatters,
  postingStatePresentation,
  toneClasses,
} from "@/lib/ui";

const SEARCH_DEBOUNCE_MS = 250;
const SUGGESTION_MIN_LENGTH = 3;
const MAX_SEARCH_SUGGESTIONS = 5;

export function JobsPage() {
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const status = searchParams.get("status") ?? undefined;
  const companyId = searchParams.get("companyId") ?? undefined;
  const postingState = searchParams.get("postingState") ?? undefined;
  const search = searchParams.get("search") ?? undefined;
  const salaryMinParam = searchParams.get("salaryMin");
  const salaryMaxParam = searchParams.get("salaryMax");
  const parseSalaryBound = (value: string | null) => {
    if (value === null || !/^\d+$/.test(value)) return undefined;
    const parsed = Number(value);
    return Number.isSafeInteger(parsed) ? parsed : undefined;
  };
  const salaryMin = parseSalaryBound(salaryMinParam);
  const salaryMax = parseSalaryBound(salaryMaxParam);
  const isFavoriteFilter = searchParams.get("favorites") === "true";
  const isArchivedFilter = searchParams.get("archived") === "true";
  const viewMode = (searchParams.get("view") as "list" | "board" | null) ?? (isFavoriteFilter ? "board" : "list");

  const [jobs, setJobs] = useState<JobListSummary[]>([]);
  const [nextCursor, setNextCursor] = useState<JobPageCursor | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [counts, setCounts] = useState<Record<string, number>>({ all: 0, favorites: 0 });
  const [activity, setActivity] = useState<WeeklyActivity | null>(null);
  const [companies, setCompanies] = useState<Array<{ id: string; name: string }>>([]);
  const [companySidebarSearch, setCompanySidebarSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filterDraft, setFilterDraft] = useState(() => filterDraftFromUrl(search ?? null, postingState ?? null, salaryMinParam, salaryMaxParam));
  const [liveSearch, setLiveSearch] = useState(search ?? "");
  const [appliedSearch, setAppliedSearch] = useState(search ?? "");
  const [searchMenuOpen, setSearchMenuOpen] = useState(false);
  const [filterPopoverOpen, setFilterPopoverOpen] = useState(false);
  const [activeSuggestionIndex, setActiveSuggestionIndex] = useState(-1);
  const [loadedRequestKey, setLoadedRequestKey] = useState("");
  const [togglingFavId, setTogglingFavId] = useState<string | null>(null);
  const [selectedJobIds, setSelectedJobIds] = useState<Set<string>>(() => new Set());
  const [matchingJobIds, setMatchingJobIds] = useState<string[] | null>(null);
  const [selectingAll, setSelectingAll] = useState(false);
  const [bulkAction, setBulkAction] = useState<"restore" | null>(null);
  const [deleteConfirmation, setDeleteConfirmation] = useState<{
    ids: string[];
    job?: { id: string; title: string; companyName: string };
    isBulk: boolean;
  } | null>(null);
  const [deleting, setDeleting] = useState(false);
  const filterButtonRef = useRef<HTMLButtonElement>(null);
  const filterPopoverRef = useRef<HTMLDivElement>(null);
  const salaryMinInputRef = useRef<HTMLInputElement>(null);
  const selectAllRef = useRef<HTMLInputElement>(null);
  const loadSequenceRef = useRef(0);
  const selectAllSequenceRef = useRef(0);
  const appliedSearchRef = useRef(appliedSearch);
  appliedSearchRef.current = appliedSearch;
  const activeRequestKeyRef = useRef("");
  const jobsQueueRef = useRef<{
    running: boolean;
    activeKey: string | null;
    activeSequence: number | null;
    pending: {
      key: string;
      sequence: number;
      pageKey: string;
      search: string;
      quiet: boolean;
      cursor: JobPageCursor | null;
      filters: {
        status?: string;
        companyId?: string;
        postingState?: string;
        salaryMin?: number;
        salaryMax?: number;
        isFavorite?: boolean;
        isArchived?: boolean;
      };
    } | null;
  }>({ running: false, activeKey: null, activeSequence: null, pending: null });
  const lastPageFiltersKeyRef = useRef("");
  const { onRunSettled, reportRefreshFailed } = useRunMonitorActions();

  useEffect(() => {
    setFilterDraft(filterDraftFromUrl(search ?? null, postingState ?? null, salaryMinParam, salaryMaxParam));
    setLiveSearch(search ?? "");
  }, [postingState, search, salaryMinParam, salaryMaxParam]);

  useEffect(() => {
    if (!filterPopoverOpen) return;

    setSearchMenuOpen(false);
    salaryMinInputRef.current?.focus();

    function handlePointerDown(event: PointerEvent) {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (filterPopoverRef.current?.contains(target) || filterButtonRef.current?.contains(target)) return;
      setFilterPopoverOpen(false);
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setFilterPopoverOpen(false);
      filterButtonRef.current?.focus();
    }

    document.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [filterPopoverOpen]);

  useEffect(() => {
    const timer = window.setTimeout(() => setAppliedSearch(liveSearch.trim()), SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [liveSearch]);

  const filtersKey = JSON.stringify({
      status,
      companyId,
      postingState,
      salaryMin,
      salaryMax,
      isFavorite: isFavoriteFilter,
      isArchived: isArchivedFilter,
  });
  const selectionScopeKey = JSON.stringify({
    isArchived: isArchivedFilter,
    filtersKey,
    liveSearch: liveSearch.trim(),
    appliedSearch,
  });
  const requestKeyFor = (searchText: string) => JSON.stringify({ filtersKey, search: searchText });
  activeRequestKeyRef.current = requestKeyFor(appliedSearch);
  const selectionReady = isArchivedFilter && loadedRequestKey === activeRequestKeyRef.current;
  const selectedCount = selectedJobIds.size;
  const allMatchingSelected = Boolean(
    matchingJobIds?.length && matchingJobIds.every((id) => selectedJobIds.has(id)),
  );
  const someVisibleSelected = selectedCount > 0 && !allMatchingSelected;

  const loadJobs = useCallback((searchText: string, quiet = true, force = false, cursor: JobPageCursor | null = null) => {
    const queue = jobsQueueRef.current;
    const key = requestKeyFor(searchText);
    const pageKey = `${key}|${cursor ? `${cursor.updatedAt}:${cursor.id}` : "first"}`;
    if (!force && queue.activeKey === pageKey) {
      queue.pending = null;
      if (queue.activeSequence !== null) loadSequenceRef.current = queue.activeSequence;
      if (cursor) setLoadingMore(false);
      return;
    }
    if (!force && queue.pending?.pageKey === pageKey) {
      if (cursor) setLoadingMore(false);
      return;
    }
    if (!cursor) {
      setNextCursor(null);
      setLoadingMore(false);
    }
    const sequence = ++loadSequenceRef.current;
    queue.pending = {
      key,
      pageKey,
      sequence,
      search: searchText,
      quiet,
      cursor,
      filters: {
        status,
        companyId,
        postingState,
        salaryMin,
        salaryMax,
        isFavorite: isFavoriteFilter ? true : undefined,
        isArchived: isArchivedFilter
          ? true
          : !status && !isFavoriteFilter
            ? false
            : undefined,
      },
    };
    if (!quiet && !queue.running) setLoading(true);
    setError(null);
    if (queue.running) return;

    queue.running = true;
    const drain = async () => {
      try {
        while (queue.pending) {
          const request = queue.pending;
          queue.pending = null;
          queue.activeKey = request.pageKey;
          queue.activeSequence = request.sequence;
          try {
            const result = await api.listJobsPage(
              { ...request.filters, search: request.search || undefined },
              request.cursor,
            );
            if (request.sequence === loadSequenceRef.current && request.key === activeRequestKeyRef.current) {
              setJobs((previous) => {
                if (!request.cursor) return result.jobs;
                const seen = new Set(previous.map((item) => item.job.id));
                return [...previous, ...result.jobs.filter((item) => !seen.has(item.job.id))];
              });
              setNextCursor(result.nextCursor);
              setLoadedRequestKey(request.key);
            }
          } catch (err) {
            if (request.sequence === loadSequenceRef.current && request.key === activeRequestKeyRef.current) {
              const message = err instanceof Error ? err.message : "Failed to load jobs";
              setError(message);
              if (request.quiet) reportRefreshFailed(message);
            }
          } finally {
            queue.activeKey = null;
            queue.activeSequence = null;
            if (request.cursor) setLoadingMore(false);
          }
        }
      } finally {
        queue.running = false;
        if (!queue.pending) setLoading(false);
      }
    };
    void drain();
  }, [status, companyId, postingState, salaryMin, salaryMax, isFavoriteFilter, isArchivedFilter, filtersKey, reportRefreshFailed]);

  const loadMore = useCallback(() => {
    if (!nextCursor || loadingMore || loading) return;
    setLoadingMore(true);
    loadJobs(appliedSearch, true, false, nextCursor);
  }, [nextCursor, loadingMore, loading, loadJobs, appliedSearch]);

  const load = useCallback(async (opts?: { quiet?: boolean }) => {
    const searchText = appliedSearchRef.current;
    loadJobs(searchText, opts?.quiet ?? false, true);
    try {
      const [dashboardResult, companiesResult] = await Promise.all([
        api.getJobsDashboard(),
        api.listCompanies(),
      ]);
      setCounts(dashboardResult.counts);
      setActivity(dashboardResult.weeklyActivity);
      setCompanies(companiesResult.companies.map((row) => row.company));
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to load jobs";
      setError(message);
      if (opts?.quiet) reportRefreshFailed(message);
    }
  }, [loadJobs, reportRefreshFailed]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (lastPageFiltersKeyRef.current !== filtersKey) {
      lastPageFiltersKeyRef.current = filtersKey;
      return;
    }
    loadJobs(appliedSearch, true);
  }, [appliedSearch, loadJobs, filtersKey]);

  useEffect(() => {
    selectAllSequenceRef.current += 1;
    setSelectedJobIds(new Set());
    setMatchingJobIds(null);
    setSelectingAll(false);
  }, [selectionScopeKey]);

  useEffect(() => {
    if (selectAllRef.current) {
      selectAllRef.current.indeterminate = someVisibleSelected;
    }
  }, [someVisibleSelected]);

  useEffect(() => setActiveSuggestionIndex(-1), [appliedSearch, filtersKey]);

  useEffect(() => onRunSettled(() => { void load({ quiet: true }); }), [load, onRunSettled]);

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
        prev.map((item) => item.job.id === jobId
          ? {
              ...item,
              job: {
                ...item.job,
                isFavorite: res.item.job.isFavorite,
                updatedAt: res.item.job.updatedAt,
              },
            }
          : item),
      );
      setCounts((prev) => ({
        ...prev,
        favorites: Math.max(0, (prev.favorites ?? 0) + (res.item.job.isFavorite ? 1 : -1)),
      }));
    } catch (err) {
      // Revert on error
      await load({ quiet: true });
      setError(err instanceof Error ? err.message : "Failed to update favorite");
    } finally {
      setTogglingFavId(null);
    }
  }

  async function handleSetAppeal(jobId: string, appeal: number | null) {
    const previous = jobs.find((item) => item.job.id === jobId)?.job.appeal ?? null;
    setJobs((prev) =>
      prev.map((item) =>
        item.job.id === jobId ? { ...item, job: { ...item.job, appeal } } : item,
      ),
    );
    try {
      const res = await api.updateJob(jobId, { appeal });
      setJobs((prev) =>
        prev.map((item) =>
          item.job.id === jobId
            ? { ...item, job: { ...item.job, appeal: res.detail.job.appeal, updatedAt: res.detail.job.updatedAt } }
            : item,
        ),
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to update appeal score");
      setJobs((prev) =>
        prev.map((item) =>
          item.job.id === jobId ? { ...item, job: { ...item.job, appeal: previous } } : item,
        ),
      );
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
    } catch (err) {
      await load({ quiet: true });
      setError(err instanceof Error ? err.message : "Failed to update job status");
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

  function toggleJobSelection(jobId: string) {
    if (!selectionReady || bulkAction || deleting) return;
    setSelectedJobIds((current) => {
      const next = new Set(current);
      if (next.has(jobId)) next.delete(jobId);
      else next.add(jobId);
      return next;
    });
  }

  function toggleAllVisibleJobs() {
    if (!selectionReady || selectingAll || bulkAction || deleting) return;
    const requestSequence = ++selectAllSequenceRef.current;
    setSelectingAll(true);
    const filters = {
      status,
      companyId,
      postingState,
      salaryMin,
      salaryMax,
      search: appliedSearch || undefined,
      isFavorite: isFavoriteFilter ? true : undefined,
      isArchived: true,
    };
    void api.listJobIds(filters).then(({ ids }) => {
      if (requestSequence !== selectAllSequenceRef.current) return;
      setMatchingJobIds(ids);
      setSelectedJobIds((current) => {
        if (ids.length > 0 && ids.every((id) => current.has(id))) return new Set();
        return new Set(ids);
      });
    }).catch((err) => {
      if (requestSequence !== selectAllSequenceRef.current) return;
      setError(err instanceof Error ? err.message : "Failed to select matching postings");
    }).finally(() => {
      if (requestSequence === selectAllSequenceRef.current) setSelectingAll(false);
    });
  }

  function clearJobSelection() {
    setSelectedJobIds(new Set());
  }

  async function restoreSelectedJobs() {
    const ids = [...selectedJobIds];
    if (ids.length === 0 || !selectionReady || bulkAction || deleting) return;
    setBulkAction("restore");
    setError(null);
    try {
      await api.unarchiveJobs(ids);
      const restoredIds = new Set(ids);
      setJobs((previous) => previous.filter((item) => !restoredIds.has(item.job.id)));
      setSelectedJobIds(new Set());
      await load({ quiet: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to restore selected postings");
    } finally {
      setBulkAction(null);
    }
  }

  function requestDeleteJob(job: { id: string; title: string; companyName: string }) {
    setDeleteConfirmation({ ids: [job.id], job, isBulk: false });
  }

  function requestDeleteSelectedJobs() {
    const ids = [...selectedJobIds];
    if (ids.length === 0 || !selectionReady || bulkAction || deleting) return;
    setDeleteConfirmation({ ids, isBulk: true });
  }

  async function confirmDeleteJob() {
    const confirmation = deleteConfirmation;
    if (!confirmation || deleting) return;
    setDeleting(true);
    setError(null);
    try {
      if (confirmation.isBulk) {
        await api.deleteJobs(confirmation.ids);
      } else {
        await api.deleteJob(confirmation.ids[0]);
      }
      const deletedIds = new Set(confirmation.ids);
      setJobs((prev) => prev.filter((item) => !deletedIds.has(item.job.id)));
      setDeleteConfirmation(null);
      if (confirmation.isBulk) setSelectedJobIds(new Set());
      await load({ quiet: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete selected postings");
    } finally {
      setDeleting(false);
    }
  }

  useEffect(() => {
    if (!deleteConfirmation) return;
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" && !deleting) {
        setDeleteConfirmation(null);
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [deleteConfirmation, deleting]);

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
      : nextCursor
        ? `Showing 100+ priority roles on your favorite board.`
        : `${jobs.length} ${jobs.length === 1 ? "priority role" : "priority roles"} on your favorite board.`
    : isArchivedFilter
      ? jobs.length === 0
        ? "No archived roles."
        : nextCursor
          ? "Showing 100+ archived roles saved for reference."
          : `${jobs.length} ${jobs.length === 1 ? "archived role" : "archived roles"} saved for reference.`
      : jobs.length === 0
        ? "Nothing here yet."
        : nextCursor
          ? "Showing 100+ roles on your radar."
          : `${jobs.length} ${jobs.length === 1 ? "role" : "roles"} on your radar.`;

  const isFiltered = Boolean(status || companyId || postingState || liveSearch.trim() || salaryMinParam || salaryMaxParam || isFavoriteFilter || isArchivedFilter);
  const hasDraftFilters = Boolean(
    filterDraft.search.trim()
    || filterDraft.postingState
    || filterDraft.salaryMin.trim()
    || filterDraft.salaryMax.trim()
    || status
    || companyId
    || isFavoriteFilter
    || isArchivedFilter,
  );
  const activeFilterCount = Number(salaryMin !== undefined || salaryMax !== undefined) + Number(Boolean(postingState));

  function handleFilterSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const next = new URLSearchParams();
    const nextSearch = filterDraft.search.trim();
    const nextPosting = filterDraft.postingState;
    const nextSalaryMin = filterDraft.salaryMin.trim();
    const nextSalaryMax = filterDraft.salaryMax.trim();
    if (isFavoriteFilter) next.set("favorites", "true");
    if (isArchivedFilter) next.set("archived", "true");
    if (status) next.set("status", status);
    if (companyId) next.set("companyId", companyId);
    if (nextSearch) next.set("search", nextSearch);
    if (nextPosting) next.set("postingState", nextPosting);
    if (nextSalaryMin) next.set("salaryMin", nextSalaryMin);
    if (nextSalaryMax) next.set("salaryMax", nextSalaryMax);
    if (viewMode) next.set("view", viewMode);
    setSearchParams(next);
    if (filterPopoverOpen) {
      setFilterPopoverOpen(false);
      filterButtonRef.current?.focus();
    }
  }

  const activeJobsCount = Math.max(0, (counts.all ?? 0) - (counts.archivedTotal ?? 0));
  const visibleCompanies = useMemo(
    () => filterCompaniesBySearch(companies, companySidebarSearch),
    [companies, companySidebarSearch],
  );
  const normalizedLiveSearch = liveSearch.trim();
  const suggestionsReady = normalizedLiveSearch.length >= SUGGESTION_MIN_LENGTH
    && normalizedLiveSearch === appliedSearch
    && loadedRequestKey === activeRequestKeyRef.current;
  const suggestionMenuExpanded = searchMenuOpen && suggestionsReady;
  const suggestions = suggestionsReady ? jobs.slice(0, MAX_SEARCH_SUGGESTIONS) : [];

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
            {companies.length > 0 ? (
              <input
                type="search"
                value={companySidebarSearch}
                onChange={(event) => setCompanySidebarSearch(event.target.value)}
                placeholder="Search companies…"
                aria-label="Search companies"
                className="field mb-3 py-2 text-xs"
              />
            ) : null}
            <div className="max-h-80 space-y-1 overflow-y-auto pr-1">
              {visibleCompanies.map((company) => (
                <SidebarLink
                  key={company.id}
                  to={`/?companyId=${company.id}`}
                  active={companyId === company.id}
                  label={company.name}
                />
              ))}
              {companies.length === 0 ? (
                <p className="text-sm text-[var(--faint)]">No companies yet.</p>
              ) : visibleCompanies.length === 0 ? (
                <p className="px-3 py-2 text-sm text-[var(--faint)]">No matching companies.</p>
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

          <div className="flex items-center">
            <form onSubmit={handleFilterSubmit} className="flex w-full flex-wrap items-center gap-2">
              <div
                className="relative min-w-0 w-full max-w-sm flex-1"
                onBlur={(event) => {
                  if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
                    setSearchMenuOpen(false);
                  }
                }}
              >
                <input
                  type="search"
                  name="search"
                  value={filterDraft.search}
                  onChange={(event) => {
                    const value = event.target.value;
                    setFilterDraft((current) => ({ ...current, search: value }));
                    setLiveSearch(value);
                    setActiveSuggestionIndex(-1);
                    setSearchMenuOpen(true);
                  }}
                  onFocus={() => {
                    setFilterPopoverOpen(false);
                    setSearchMenuOpen(true);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") {
                      event.preventDefault();
                      setSearchMenuOpen(false);
                      setActiveSuggestionIndex(-1);
                    } else if (suggestions.length > 0 && event.key === "ArrowDown") {
                      event.preventDefault();
                      setSearchMenuOpen(true);
                      setActiveSuggestionIndex((current) => (current + 1) % suggestions.length);
                    } else if (suggestions.length > 0 && event.key === "ArrowUp") {
                      event.preventDefault();
                      setSearchMenuOpen(true);
                      setActiveSuggestionIndex((current) => current <= 0 ? suggestions.length - 1 : current - 1);
                    } else if (event.key === "Enter" && searchMenuOpen && suggestionsReady) {
                      event.preventDefault();
                      const selected = suggestions[Math.max(activeSuggestionIndex, 0)];
                      if (selected) navigate(`/jobs/${selected.job.id}`);
                    }
                  }}
                  placeholder="Search title, company, or posting URL…"
                  aria-label="Search jobs"
                  aria-autocomplete="list"
                  aria-expanded={suggestionMenuExpanded}
                  aria-controls="job-search-suggestions"
                  aria-activedescendant={suggestionMenuExpanded && activeSuggestionIndex >= 0 ? `job-search-option-${activeSuggestionIndex}` : undefined}
                  className="field jobs-search-field w-full"
                />
                <svg
                  aria-hidden="true"
                  className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--muted)]"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <circle cx="10.8" cy="10.8" r="6.3" />
                  <path d="m16 16 4.2 4.2" />
                </svg>
                {suggestionMenuExpanded ? (
                  <div className="absolute left-0 right-0 top-full z-30 mt-1 overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--surface)] shadow-[var(--shadow-md)]">
                    <ul id="job-search-suggestions" role="listbox" aria-label="Matching jobs" className="max-h-80 overflow-y-auto py-1">
                      {suggestions.map((item, index) => (
                        <li key={item.job.id}>
                          <button
                            id={`job-search-option-${index}`}
                            type="button"
                            role="option"
                            aria-selected={index === activeSuggestionIndex}
                            className={`block w-full px-3 py-2 text-left ${index === activeSuggestionIndex ? "bg-[var(--accent-soft)]" : "hover:bg-[var(--surface-muted)]"}`}
                            onMouseDown={(event) => event.preventDefault()}
                            onMouseEnter={() => setActiveSuggestionIndex(index)}
                            onClick={() => navigate(`/jobs/${item.job.id}`)}
                          >
                            <span className="block truncate text-sm font-medium text-[var(--foreground)]">{item.job.title}</span>
                            <span className="block truncate text-xs text-[var(--muted)]">{item.companyName}</span>
                          </button>
                        </li>
                      ))}
                    </ul>
                    {suggestions.length === 0 ? (
                      <p role="status" className="px-3 py-2 text-sm text-[var(--muted)]">No matching jobs</p>
                    ) : null}
                  </div>
                ) : null}
              </div>
              <div className="relative shrink-0">
                <button
                  ref={filterButtonRef}
                  type="button"
                  className={`btn btn-secondary btn-sm ${filterPopoverOpen || activeFilterCount > 0 ? "border-[var(--accent)] text-[var(--accent-ink)]" : ""}`}
                  aria-label={activeFilterCount > 0 ? `Filters, ${activeFilterCount} active` : "Filters"}
                  aria-haspopup="dialog"
                  aria-expanded={filterPopoverOpen}
                  aria-controls="jobs-filter-popover"
                  onClick={() => setFilterPopoverOpen((open) => !open)}
                >
                  Filters
                  {activeFilterCount > 0 ? (
                    <span aria-hidden="true" className="ml-0.5 inline-flex min-w-5 items-center justify-center rounded-full bg-[var(--accent-soft)] px-1.5 py-0.5 text-[0.6875rem] font-semibold leading-none text-[var(--accent-ink)]">
                      {activeFilterCount}
                    </span>
                  ) : null}
                </button>
                {filterPopoverOpen ? (
                  <div
                    id="jobs-filter-popover"
                    ref={filterPopoverRef}
                    role="dialog"
                    aria-modal="false"
                    aria-labelledby="jobs-filter-heading"
                    className="jobs-filter-popover absolute right-0 top-full z-30 mt-2 rounded-xl border border-[var(--border)] bg-[var(--surface)] p-4 shadow-[var(--shadow-md)]"
                  >
                    <h2 id="jobs-filter-heading" className="mb-4 text-sm font-semibold text-[var(--foreground)]">Filters</h2>
                    <fieldset>
                      <legend className="field-label">Salary per year (USD)</legend>
                      <div className="grid grid-cols-2 gap-3">
                        <label htmlFor="salary-min" className="text-xs font-medium text-[var(--muted)]">
                          Minimum
                          <input
                            ref={salaryMinInputRef}
                            id="salary-min"
                            type="number"
                            min="0"
                            step="1000"
                            inputMode="numeric"
                            value={filterDraft.salaryMin}
                            onChange={(event) => setFilterDraft((current) => ({ ...current, salaryMin: event.target.value }))}
                            placeholder="No minimum"
                            className="field mt-1.5 text-sm"
                          />
                        </label>
                        <label htmlFor="salary-max" className="text-xs font-medium text-[var(--muted)]">
                          Maximum
                          <input
                            id="salary-max"
                            type="number"
                            min="0"
                            step="1000"
                            inputMode="numeric"
                            value={filterDraft.salaryMax}
                            onChange={(event) => setFilterDraft((current) => ({ ...current, salaryMax: event.target.value }))}
                            placeholder="No maximum"
                            className="field mt-1.5 text-sm"
                          />
                        </label>
                      </div>
                    </fieldset>
                    <label htmlFor="posting-state" className="field-label mt-4">Posting status</label>
                    <select
                      id="posting-state"
                      name="postingState"
                      value={filterDraft.postingState}
                      onChange={(event) => setFilterDraft((current) => ({ ...current, postingState: event.target.value }))}
                      className="field text-sm"
                    >
                      <option value="">All postings</option>
                      <option value="active">Active</option>
                      <option value="inactive">Inactive</option>
                    </select>
                    <div className="mt-4 flex items-center justify-between gap-3 border-t border-[var(--border)] pt-3">
                      {isFiltered || hasDraftFilters ? (
                        <Link
                          to="/"
                          className="btn btn-ghost btn-sm"
                          onClick={() => setFilterPopoverOpen(false)}
                        >
                          Clear all
                        </Link>
                      ) : <span />}
                      <button type="submit" className="btn btn-secondary btn-sm">
                        Apply
                      </button>
                    </div>
                  </div>
                ) : null}
              </div>
            </form>

          </div>

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
          ) : (
            <PipelineSummary
              activeJobsCount={activeJobsCount}
              appliedCount={counts.applied ?? 0}
              interviewingCount={counts.interviewing ?? 0}
              offerCount={counts.offer ?? 0}
            />
          )}

          {!isFiltered && activity ? <ActivityLine activity={activity} /> : null}

          {isArchivedFilter && jobs.length > 0 ? (
            <div className={`card archived-selection-toolbar flex flex-wrap items-center gap-x-4 gap-y-3 px-4 py-3 ${
              selectedCount > 0 ? "has-selection" : ""
            }`}>
              <label className="flex min-h-8 cursor-pointer items-center gap-2 text-sm text-[var(--muted)]">
                <input
                  ref={selectAllRef}
                  type="checkbox"
                  checked={allMatchingSelected}
                  disabled={!selectionReady || selectingAll || bulkAction !== null || deleting}
                  onChange={toggleAllVisibleJobs}
                  aria-label="Select all matching archived postings"
                  className="archived-selection-checkbox h-4 w-4 cursor-pointer accent-[var(--accent)] disabled:cursor-wait"
                />
                <span>{selectingAll ? "Selecting matching postings…" : "Select all matching postings"}</span>
              </label>
              {selectedCount > 0 ? (
                <div className="ml-auto flex flex-wrap items-center gap-2">
                  <span role="status" aria-live="polite" className="mr-1 text-xs font-medium text-[var(--muted)]">
                    {selectedCount} selected
                  </span>
                  <button
                    type="button"
                    onClick={() => void restoreSelectedJobs()}
                    disabled={!selectionReady || bulkAction !== null || deleting}
                    className="btn btn-secondary btn-sm inline-flex items-center gap-1.5"
                  >
                    {bulkAction === "restore" ? <span className="spinner" /> : <ArchiveIcon size={14} />}
                    <span>{bulkAction === "restore" ? "Restoring…" : "Restore selected"}</span>
                  </button>
                  <button
                    type="button"
                    onClick={requestDeleteSelectedJobs}
                    disabled={!selectionReady || bulkAction !== null || deleting}
                    className="btn btn-sm inline-flex items-center gap-1.5 bg-[var(--danger-soft)] text-[var(--danger)] hover:opacity-80"
                  >
                    <TrashIcon size={14} />
                    <span>Delete selected</span>
                  </button>
                  <button
                    type="button"
                    onClick={clearJobSelection}
                    disabled={bulkAction !== null || deleting}
                    className="btn btn-ghost btn-sm"
                  >
                    Clear
                  </button>
                </div>
              ) : null}
            </div>
          ) : null}

          {error ? (
            <p role="alert" className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
              {error}
            </p>
          ) : null}

          {loading && jobs.length === 0 ? (
            <div className="space-y-3">
              {[1, 2, 3].map((i) => (
                <div
                  key={i}
                  className="card flex h-24 items-center justify-between p-4 animate-pulse bg-[var(--surface-muted)]/50"
                />
              ))}
            </div>
          ) : jobs.length === 0 ? (
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
              onDeleteJob={requestDeleteJob}
              selection={isArchivedFilter ? {
                selectedIds: selectedJobIds,
                disabled: !selectionReady || bulkAction !== null || deleting,
                onToggle: toggleJobSelection,
              } : undefined}
              isPendingFavorite={(id) => togglingFavId === id}
            />
          ) : (
            <ul className="space-y-3">
              {jobs.map(({ job, companyName }) => {
                const statusInfo = jobStatusPresentation(job.status);
                const postingInfo = postingStateMatters(job.status)
                      ? postingStatePresentation(job.postingState, job.lastCheckedAt)
                  : null;
                const source = jobSourceLabel(job.source);
                const StageIcon = statusIcons[job.status];
                const selected = selectedJobIds.has(job.id);
                return (
                  <li
                    key={job.id}
                    className={`archived-selection-row relative ${selected ? "is-selected" : ""}`}
                  >
                    <Link
                      to={`/jobs/${job.id}`}
                      className={`card group block p-5 hover:shadow-[var(--shadow-md)] ${
                        isArchivedFilter ? "archived-job-card pl-12" : ""
                      } ${selected ? "is-selected" : ""}`}
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
                          <div
                            className="flex items-center gap-1.5"
                            onMouseDown={(event) => event.stopPropagation()}
                            onClick={(event) => {
                              event.preventDefault();
                              event.stopPropagation();
                            }}
                          >
                            <label
                              htmlFor={`appeal-${job.id}`}
                              className="text-xs text-[var(--faint)]"
                              title={APPEAL_SCALE_LABEL}
                            >
                              Appeal
                            </label>
                            <AppealSelect
                              id={`appeal-${job.id}`}
                              value={job.appeal}
                              compact
                              onChange={(appeal) => void handleSetAppeal(job.id, appeal)}
                            />
                          </div>
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
                              requestDeleteJob({ id: job.id, title: job.title, companyName });
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
                    {isArchivedFilter ? (
                      <input
                        type="checkbox"
                        checked={selected}
                        disabled={!selectionReady || bulkAction !== null || deleting}
                        onChange={() => toggleJobSelection(job.id)}
                        aria-label={`Select ${job.title} at ${companyName}`}
                        className="archived-selection-checkbox absolute left-5 top-6 z-10 h-4 w-4 cursor-pointer accent-[var(--accent)] disabled:cursor-wait"
                      />
                    ) : null}
                  </li>
                );
              })}
            </ul>
          )}
          {nextCursor ? (
            <div className="flex justify-center pt-2">
              <button
                type="button"
                onClick={loadMore}
                disabled={loadingMore || loading}
                className="btn btn-secondary"
              >
                {loadingMore ? "Loading…" : "Load more"}
              </button>
            </div>
          ) : null}
        </section>
      </div>

      {deleteConfirmation ? (
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
                  {deleteConfirmation.isBulk
                    ? `Delete ${deleteConfirmation.ids.length} job ${deleteConfirmation.ids.length === 1 ? "posting" : "postings"}?`
                    : "Delete job posting?"}
                </h3>
                <p className="text-xs text-[var(--muted)]">Permanent removal from database and CSV</p>
              </div>
            </div>

            {deleteConfirmation.isBulk ? (
              <p className="text-sm text-[var(--muted)] leading-relaxed">
                Permanently delete {deleteConfirmation.ids.length} selected {deleteConfirmation.ids.length === 1 ? "posting" : "postings"}?
              </p>
            ) : deleteConfirmation.job ? (
              <p className="text-sm text-[var(--muted)] leading-relaxed">
                Are you sure you want to delete <strong className="text-[var(--foreground)]">{deleteConfirmation.job.title}</strong> at{" "}
                <strong className="text-[var(--foreground)]">{deleteConfirmation.job.companyName}</strong>?
              </p>
            ) : null}
            <p className="text-xs text-[var(--faint)]">
              This will permanently delete {deleteConfirmation.isBulk ? "these postings and their" : "this posting, its"} timeline history and document attachment links from your local database and the synchronized CSV file. This cannot be undone.
            </p>

            <div className="flex items-center justify-end gap-2 pt-2">
              <button
                type="button"
                onClick={() => setDeleteConfirmation(null)}
                disabled={deleting}
                className="btn btn-secondary"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => void confirmDeleteJob()}
                disabled={deleting}
                className="btn bg-[var(--danger)] text-white hover:opacity-90 flex items-center gap-1.5"
              >
                {deleting ? <span className="spinner" /> : <TrashIcon size={14} />}
                <span>{deleting ? "Deleting…" : "Delete permanently"}</span>
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

function PipelineSummary({
  activeJobsCount,
  appliedCount,
  interviewingCount,
  offerCount,
}: {
  activeJobsCount: number;
  appliedCount: number;
  interviewingCount: number;
  offerCount: number;
}) {
  const metrics = [
    { label: "Tracking", value: activeJobsCount },
    { label: "Applied", value: appliedCount },
    { label: "Interviewing", value: interviewingCount },
    { label: "Offers", value: offerCount },
  ];

  return (
    <section
      aria-label="Pipeline totals across all jobs"
      className="card flex flex-wrap items-center gap-x-6 gap-y-3 px-4 py-3"
    >
      <div>
        <h2 className="text-sm font-semibold text-[var(--muted)]">Pipeline</h2>
        <p className="text-xs text-[var(--faint)]">All jobs</p>
      </div>
      <dl className="grid flex-1 grid-cols-2 gap-x-5 gap-y-2 sm:grid-cols-4">
        {metrics.map((metric) => (
          <div key={metric.label} className="flex items-baseline justify-between gap-2 sm:block">
            <dt className="text-xs text-[var(--muted)]">{metric.label}</dt>
            <dd className="font-display text-lg leading-none text-[var(--foreground)]">{metric.value}</dd>
          </div>
        ))}
      </dl>
    </section>
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
