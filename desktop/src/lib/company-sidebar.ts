import { calendarDateOf, formatShortDate, parseInstant, type Instant } from "./instant";
import { filterCompaniesBySearch } from "./job-filters";
import type { CompanyRow } from "./schema";

/**
 * Pure logic for the Jobs page Companies sidebar: sort options, filter toggles,
 * helpers over `CompanyRow[]`, and the view-state codec. No React, no URL state;
 * storage is always injected by the caller.
 */

/**
 * Sort_Menu entries, top to bottom. `group` is internal: the menu renders a separator
 * wherever it changes between adjacent entries (name | count | activity).
 */
export const COMPANY_SORT_OPTIONS = [
  { value: "nameAsc", label: "Name A–Z", group: "name" },
  { value: "nameDesc", label: "Name Z–A", group: "name" },
  { value: "recentlyAdded", label: "Recently added company", group: "name" },
  { value: "openPositionsDesc", label: "Most open positions", group: "count" },
  { value: "trackedJobsDesc", label: "Most tracked jobs", group: "count" },
  { value: "wishlistJobsDesc", label: "Most wishlist jobs", group: "count" },
  { value: "wishlistJobsAsc", label: "Fewest wishlist jobs", group: "count" },
  { value: "appliedJobsDesc", label: "Most applied jobs", group: "count" },
  { value: "appliedJobsAsc", label: "Fewest applied jobs", group: "count" },
  { value: "interviewingJobsDesc", label: "Most interviews", group: "count" },
  { value: "recentActivity", label: "Recently updated job", group: "activity" },
  { value: "recentlyApplied", label: "Recently applied", group: "activity" },
  { value: "easiestWishlist", label: "Easiest wishlist to apply", group: "activity" },
] as const;

export type CompanySortOption = (typeof COMPANY_SORT_OPTIONS)[number]["value"];

export type CompanySortGroup = (typeof COMPANY_SORT_OPTIONS)[number]["group"];

export const DEFAULT_COMPANY_SORT: CompanySortOption = "nameAsc";

export const COMPANY_FILTERS = [
  { value: "watched", label: "Watched" },
  { value: "hasOpenPositions", label: "Has open positions" },
  { value: "hasTrackedJobs", label: "Has tracked jobs" },
  { value: "needsAttention", label: "Needs attention" },
  // Appended (not inserted) so pre-feature serialized states stay byte-identical.
  { value: "hasWishlistJobs", label: "Has wishlist jobs" },
  { value: "hasAppliedJobs", label: "Has applied jobs" },
] as const;

export type CompanyFilter = (typeof COMPANY_FILTERS)[number]["value"];

export type CompanyViewState = {
  sort: CompanySortOption;
  filters: ReadonlySet<CompanyFilter>;
};

export const DEFAULT_COMPANY_VIEW_STATE: CompanyViewState = Object.freeze({
  sort: DEFAULT_COMPANY_SORT,
  filters: new Set<CompanyFilter>(),
});

// One shared collator: "accent" sensitivity treats A/a as equal but keeps a/á distinct.
const nameCollator = new Intl.Collator(undefined, { sensitivity: "accent" });

/** -1, 0, or 1. Safe for ±Infinity (never NaN). */
function sign(a: number, b: number): number {
  if (a < b) return -1;
  if (a > b) return 1;
  return 0;
}

function compareCodeUnits(a: string, b: string): number {
  if (a < b) return -1;
  if (a > b) return 1;
  return 0;
}

/** Name_Order: locale-aware, case-insensitive name, then `id` by code unit. */
export function compareByNameOrder(a: CompanyRow, b: CompanyRow): number {
  const byName = nameCollator.compare(a.company.name, b.company.name);
  if (byName !== 0) return byName < 0 ? -1 : 1;
  return compareCodeUnits(a.company.id, b.company.id);
}

// ---------------------------------------------------------------------------
// Sanitizers and accessors for the pipeline aggregates (Req 3.10, 5.5).
// The backend contract types these fields, but rows from an older backend or a
// malformed payload may omit them or carry bad values, so every read goes through here.
// ---------------------------------------------------------------------------

/** Whole number ≥ 0, else 0. Rejects NaN, ±Infinity, fractions, negatives, non-numbers. */
export function sanitizeCount(value: unknown): number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 ? value : 0;
}

/** Finite number in [1, 5], else null. */
export function sanitizeEase(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 1 && value <= 5 ? value : null;
}

/** Sanitized Wishlist_Job_Count. */
export function wishlistCountOf(row: CompanyRow): number {
  return sanitizeCount((row as Partial<CompanyRow>).wishlistJobCount);
}

/** Sanitized Applied_Job_Count. */
export function appliedCountOf(row: CompanyRow): number {
  return sanitizeCount((row as Partial<CompanyRow>).appliedJobCount);
}

/** Sanitized Interviewing_Job_Count. */
export function interviewingCountOf(row: CompanyRow): number {
  return sanitizeCount((row as Partial<CompanyRow>).interviewingJobCount);
}

/** Sanitized Wishlist_Ease_Average (unrounded), or null. */
export function easeOf(row: CompanyRow): number | null {
  return sanitizeEase((row as Partial<CompanyRow>).wishlistEaseAverage);
}

/** Last_Activity_At as an Instant, or null when missing or not a Valid_Timestamp. */
export function lastActivityOf(row: CompanyRow): Instant | null {
  return parseInstant((row as Partial<CompanyRow>).lastActivityAt);
}

/** Last_Applied_At as an Instant, or null when missing or not a Valid_Timestamp. */
export function lastAppliedOf(row: CompanyRow): Instant | null {
  return parseInstant((row as Partial<CompanyRow>).lastAppliedAt);
}

/** Parsed `createdAt` in ms; unparseable values sort as the oldest. */
function createdAtMs(row: CompanyRow): number {
  const ms = Date.parse(row.company.createdAt);
  return Number.isNaN(ms) ? -Infinity : ms;
}

/** Sort_Options ordered by a key vector (everything except the two pure name sorts). */
type KeyedSortOption = Exclude<CompanySortOption, "nameAsc" | "nameDesc">;

/**
 * Key vector for a row under `sort`; smaller sorts first, compared element-wise,
 * then Name_Order. A leading `1` marks a trailing group (Req 8.5) whose remaining
 * elements are all 0, so Name_Order alone orders it.
 */
function sortKeyVector(row: CompanyRow, sort: KeyedSortOption): readonly number[] {
  switch (sort) {
    case "recentlyAdded":
      return [-createdAtMs(row)];
    case "openPositionsDesc":
      return [-row.openPositionCount];
    case "trackedJobsDesc":
      return [-row.trackedJobCount];
    case "wishlistJobsDesc":
      return [-wishlistCountOf(row)];
    case "appliedJobsDesc":
      return [-appliedCountOf(row)];
    case "interviewingJobsDesc":
      return [-interviewingCountOf(row)];
    case "wishlistJobsAsc": {
      const w = wishlistCountOf(row);
      return w === 0 ? [1, 0] : [0, w];
    }
    case "appliedJobsAsc": {
      const a = appliedCountOf(row);
      return a === 0 ? [1, 0] : [0, a];
    }
    case "recentActivity":
      return instantKey(lastActivityOf(row));
    case "recentlyApplied":
      return instantKey(lastAppliedOf(row));
    case "easiestWishlist": {
      const e = easeOf(row);
      return e === null ? [1, 0, 0] : [0, -e, -wishlistCountOf(row)];
    }
    default: {
      const unreachable: never = sort;
      throw new Error(`Unhandled company sort option: ${String(unreachable)}`);
    }
  }
}

/** Newest instant first; null/invalid timestamps trail. */
function instantKey(t: Instant | null): readonly number[] {
  return t === null ? [1, 0, 0] : [0, -t.ms, -t.subMsNanos];
}

function compareKeyVectors(a: readonly number[], b: readonly number[]): number {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    const c = sign(a[i], b[i]);
    if (c !== 0) return c;
  }
  return 0;
}

/** Returns a new sorted array; never mutates `rows`. Total order via the `id` tie-break. */
export function sortCompanies(rows: readonly CompanyRow[], sort: CompanySortOption): CompanyRow[] {
  if (sort === "nameAsc") return [...rows].sort(compareByNameOrder);
  if (sort === "nameDesc") return [...rows].sort((a, b) => compareByNameOrder(b, a));

  // Compute each key once so large lists don't re-parse dates in the comparator.
  const decorated = rows.map((row) => ({ row, key: sortKeyVector(row, sort) }));
  decorated.sort((a, b) => compareKeyVectors(a.key, b.key) || compareByNameOrder(a.row, b.row));
  return decorated.map((entry) => entry.row);
}

export function matchesCompanyFilter(row: CompanyRow, filter: CompanyFilter): boolean {
  switch (filter) {
    case "watched":
      return row.watches.length > 0;
    case "hasOpenPositions":
      return row.openPositionCount > 0;
    case "hasTrackedJobs":
      return row.trackedJobCount > 0;
    case "needsAttention":
      // `list_companies` returns only pending reviews.
      return row.watches.some((watch) => watch.consecutiveSyncFailures > 0) || row.reviews.length > 0;
    case "hasWishlistJobs":
      return wishlistCountOf(row) >= 1;
    case "hasAppliedJobs":
      return appliedCountOf(row) >= 1;
  }
}

/** Keeps rows that satisfy every active filter, in input order. Empty set → same rows, same order. */
export function applyCompanyFilters(
  rows: readonly CompanyRow[],
  filters: ReadonlySet<CompanyFilter>,
): CompanyRow[] {
  if (filters.size === 0) return [...rows];
  const active = [...filters];
  return rows.filter((row) => active.every((filter) => matchesCompanyFilter(row, filter)));
}

export type SortAnnotation = {
  /** Visible text, e.g. "3", "Mar 4", "4.5". */
  text: string;
  /** Accessible label naming measure and value, e.g. "3 wishlist jobs". */
  label: string;
};

export type AnnotationContext = { now: Date; locale?: string; timeZone?: string };

/** One decimal, halves away from zero: 4.25 → "4.3", 4 → "4.0". Inputs lie in [1, 5]. */
export function formatEase(value: number): string {
  // The 1e-9 nudge rounds non-representable halves (23/20 = 1.1499…) up as the exact mean would.
  return (Math.round(value * 10 + 1e-9) / 10).toFixed(1);
}

function countAnnotation(n: number, singular: string, plural: string): SortAnnotation {
  const text = String(n);
  return { text, label: `${text} ${n === 1 ? singular : plural}` };
}

function dateAnnotation(value: unknown, prefix: string, ctx: AnnotationContext): SortAnnotation | null {
  if (typeof value !== "string") return null;
  const date = calendarDateOf(value, ctx.timeZone);
  if (!date) return null;
  const text = formatShortDate(date, ctx.now, ctx.locale, ctx.timeZone);
  return { text, label: `${prefix} ${text}` };
}

/** Annotation for the active sort, or null when the sort has none or the value is null/invalid. */
export function sortAnnotationFor(
  row: CompanyRow,
  sort: CompanySortOption,
  ctx: AnnotationContext,
): SortAnnotation | null {
  switch (sort) {
    case "nameAsc":
    case "nameDesc":
    case "recentlyAdded":
      return null;
    case "openPositionsDesc":
      // Same visible number as before this feature (Req 6.8).
      return countAnnotation(row.openPositionCount, "open position", "open positions");
    case "trackedJobsDesc":
      return countAnnotation(row.trackedJobCount, "tracked job", "tracked jobs");
    case "wishlistJobsDesc":
    case "wishlistJobsAsc":
      return countAnnotation(wishlistCountOf(row), "wishlist job", "wishlist jobs");
    case "appliedJobsDesc":
    case "appliedJobsAsc":
      return countAnnotation(appliedCountOf(row), "applied job", "applied jobs");
    case "interviewingJobsDesc":
      return countAnnotation(interviewingCountOf(row), "interview", "interviews");
    case "recentActivity":
      return dateAnnotation((row as Partial<CompanyRow>).lastActivityAt, "Last activity", ctx);
    case "recentlyApplied":
      return dateAnnotation((row as Partial<CompanyRow>).lastAppliedAt, "Last applied", ctx);
    case "easiestWishlist": {
      const e = easeOf(row);
      if (e === null) return null;
      const text = formatEase(e);
      return { text, label: `Average ease ${text}` };
    }
    default: {
      const unreachable: never = sort;
      throw new Error(`Unhandled company sort option: ${String(unreachable)}`);
    }
  }
}

/** Flips one filter; keeps the sort and every other filter. */
export function toggleCompanyFilter(view: CompanyViewState, filter: CompanyFilter): CompanyViewState {
  const filters = new Set(view.filters);
  if (filters.has(filter)) filters.delete(filter);
  else filters.add(filter);
  return { sort: view.sort, filters };
}

/** Deactivates every filter; keeps the sort. */
export function clearCompanyFilters(view: CompanyViewState): CompanyViewState {
  return { sort: view.sort, filters: new Set<CompanyFilter>() };
}

/** Search step over rows, reusing `filterCompaniesBySearch` (which matches on a top-level `name`). */
function searchCompanyRows(rows: readonly CompanyRow[], search: string): CompanyRow[] {
  const entries = rows.map((row) => ({ name: row.company.name, row }));
  return filterCompaniesBySearch(entries, search).map((entry) => entry.row);
}

export type DerivedCompanyList = {
  /** Rows after search, then filters, then sort. */
  visible: CompanyRow[];
  /** Total rows loaded (M). */
  total: number;
  /** True when any filter is active or the trimmed search is non-empty. */
  isRefined: boolean;
};

/** search → filters → sort. Search and filters are row-local predicates, so their order doesn't matter. */
export function deriveCompanyList(
  rows: readonly CompanyRow[],
  search: string,
  view: CompanyViewState,
): DerivedCompanyList {
  const searched = searchCompanyRows(rows, search);
  const filtered = applyCompanyFilters(searched, view.filters);
  return {
    visible: sortCompanies(filtered, view.sort),
    total: rows.length,
    isRefined: view.filters.size > 0 || search.trim() !== "",
  };
}

export type CompanyEmptyState = "loading" | "error" | "noCompanies" | "noSearchMatch" | "noFilterMatch" | null;

/** Precedence: loading → error → noCompanies → null (rows visible) → noSearchMatch → noFilterMatch. */
export function companyEmptyState(input: {
  load: "loading" | "ready" | "error";
  total: number;
  visibleCount: number;
  search: string;
  filterCount: number;
}): CompanyEmptyState {
  if (input.load === "loading") return "loading";
  if (input.load === "error") return "error";
  if (input.total === 0) return "noCompanies";
  if (input.visibleCount > 0) return null;
  if (input.search.trim() !== "") return "noSearchMatch";
  return "noFilterMatch";
}

// ---------------------------------------------------------------------------
// View-state persistence (sort + filters only; search text is never stored)
// ---------------------------------------------------------------------------

export const COMPANY_VIEW_STORAGE_KEY = "jobTracker.companiesSidebar.view.v1";

const SORT_VALUES: ReadonlySet<string> = new Set(COMPANY_SORT_OPTIONS.map((option) => option.value));

function isCompanySortOption(value: unknown): value is CompanySortOption {
  return typeof value === "string" && SORT_VALUES.has(value);
}

/** A fresh default state, so callers can never share (or mutate) the default Set. */
function defaultViewState(): CompanyViewState {
  return { sort: DEFAULT_COMPANY_SORT, filters: new Set<CompanyFilter>() };
}

/** `{"version":1,"sort":…,"filters":[…]}` with filters in `COMPANY_FILTERS` order, so equal states serialize identically. */
export function serializeCompanyViewState(view: CompanyViewState): string {
  const filters = COMPANY_FILTERS.map((filter) => filter.value).filter((value) => view.filters.has(value));
  return JSON.stringify({ version: 1, sort: view.sort, filters });
}

/** Never throws. Invalid sort → `nameAsc`; keeps only known filters, each once; any `version` is accepted. */
export function parseCompanyViewState(raw: string | null): CompanyViewState {
  if (raw === null) return defaultViewState();

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return defaultViewState();
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return defaultViewState();

  const record = parsed as Record<string, unknown>;
  const sort = isCompanySortOption(record.sort) ? record.sort : DEFAULT_COMPANY_SORT;

  const filters = new Set<CompanyFilter>();
  if (Array.isArray(record.filters)) {
    const stored = new Set<unknown>(record.filters);
    for (const { value } of COMPANY_FILTERS) {
      if (stored.has(value)) filters.add(value);
    }
  }
  return { sort, filters };
}

/** Never throws; missing storage or any storage error → default state. */
export function loadCompanyViewState(storage: Pick<Storage, "getItem"> | undefined): CompanyViewState {
  if (!storage) return defaultViewState();
  try {
    return parseCompanyViewState(storage.getItem(COMPANY_VIEW_STORAGE_KEY));
  } catch {
    return defaultViewState();
  }
}

/** Never throws; returns false when there is no storage or the write failed. */
export function saveCompanyViewState(
  storage: Pick<Storage, "setItem"> | undefined,
  view: CompanyViewState,
): boolean {
  if (!storage) return false;
  try {
    storage.setItem(COMPANY_VIEW_STORAGE_KEY, serializeCompanyViewState(view));
    return true;
  } catch {
    return false;
  }
}
