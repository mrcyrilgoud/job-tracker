import { filterCompaniesBySearch } from "./job-filters";
import type { CompanyRow } from "./schema";

/**
 * Pure logic for the Jobs page Companies sidebar: sort options, filter toggles,
 * helpers over `CompanyRow[]`, and the view-state codec. No React, no URL state;
 * storage is always injected by the caller.
 */

export const COMPANY_SORT_OPTIONS = [
  { value: "nameAsc", label: "Name A–Z" },
  { value: "nameDesc", label: "Name Z–A" },
  { value: "openPositionsDesc", label: "Most open positions" },
  { value: "trackedJobsDesc", label: "Most tracked jobs" },
  { value: "recentlyAdded", label: "Recently added" },
] as const;

export type CompanySortOption = (typeof COMPANY_SORT_OPTIONS)[number]["value"];

export const DEFAULT_COMPANY_SORT: CompanySortOption = "nameAsc";

export const COMPANY_FILTERS = [
  { value: "watched", label: "Watched" },
  { value: "hasOpenPositions", label: "Has open positions" },
  { value: "hasTrackedJobs", label: "Has tracked jobs" },
  { value: "needsAttention", label: "Needs attention" },
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

/** Parsed `createdAt` in ms; unparseable values sort as the oldest. */
function createdAtMs(row: CompanyRow): number {
  const ms = Date.parse(row.company.createdAt);
  return Number.isNaN(ms) ? -Infinity : ms;
}

/** Descending numeric key for the sort, or null for name-only sorts. */
function sortKey(row: CompanyRow, sort: CompanySortOption): number | null {
  switch (sort) {
    case "openPositionsDesc":
      return row.openPositionCount;
    case "trackedJobsDesc":
      return row.trackedJobCount;
    case "recentlyAdded":
      return createdAtMs(row);
    default:
      return null;
  }
}

/** Returns a new sorted array; never mutates `rows`. Total order via the `id` tie-break. */
export function sortCompanies(rows: readonly CompanyRow[], sort: CompanySortOption): CompanyRow[] {
  if (sort === "nameAsc") return [...rows].sort(compareByNameOrder);
  if (sort === "nameDesc") return [...rows].sort((a, b) => compareByNameOrder(b, a));

  // Compute each key once so large lists don't re-parse dates in the comparator.
  const decorated = rows.map((row) => ({ row, key: sortKey(row, sort) as number }));
  decorated.sort((a, b) => sign(b.key, a.key) || compareByNameOrder(a.row, b.row));
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

/** Count shown next to a company name, or null when the sort has no count. */
export function sortCountFor(row: CompanyRow, sort: CompanySortOption): number | null {
  if (sort === "openPositionsDesc") return row.openPositionCount;
  if (sort === "trackedJobsDesc") return row.trackedJobCount;
  return null;
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
