import { jobStatuses, type JobStatus, type RemoteMode } from "@/lib/schema";

/** The three job date fields a range filter may target. */
export type DateField = "appliedAt" | "postingDate" | "lastCheckedAt";

export type JobFilterDraft = {
  search: string;
  postingState: string;
  salaryMin: string;
  salaryMax: string;
  statuses: JobStatus[];
  dateField: "" | DateField;
  dateFrom: string;
  dateTo: string;
  location: string;
  remoteMode: RemoteMode;
};

const DATE_FIELDS: readonly DateField[] = ["appliedAt", "postingDate", "lastCheckedAt"];
const REMOTE_MODES: readonly RemoteMode[] = ["any", "remoteOnly", "onsiteOnly"];

const JOB_STATUS_SET = new Set<string>(jobStatuses);

/** Strict `YYYY-MM-DD` with a real calendar day (e.g. rejects 2026-02-30). */
export function isValidCalendarDate(value: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return false;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (month < 1 || month > 12 || day < 1 || day > 31) return false;
  const date = new Date(Date.UTC(year, month - 1, day));
  return (
    date.getUTCFullYear() === year &&
    date.getUTCMonth() === month - 1 &&
    date.getUTCDate() === day
  );
}

/**
 * Validate a date-range draft. Returns a human-readable message when invalid,
 * or `null` when the range is acceptable. Empty bounds are treated as unset.
 */
export function validateDateRange(from: string, to: string): string | null {
  const start = from.trim();
  const end = to.trim();

  if ((start && !isValidCalendarDate(start)) || (end && !isValidCalendarDate(end))) {
    return "Enter a valid date in YYYY-MM-DD form.";
  }
  if (start && end && start > end) {
    return "Start date must not be later than the end date.";
  }
  return null;
}

/**
 * Parse the `statuses` param into a deduplicated list of valid `JobStatus`
 * members in first-seen order. Non-members and duplicates are dropped. Total.
 */
export function parseStatusesFromUrl(params: URLSearchParams): JobStatus[] {
  const raw = params.get("statuses");
  if (!raw) return [];

  const seen = new Set<string>();
  const result: JobStatus[] = [];
  for (const token of raw.split(",")) {
    const value = token.trim();
    if (!value || seen.has(value) || !JOB_STATUS_SET.has(value)) continue;
    seen.add(value);
    result.push(value as JobStatus);
  }
  return result;
}

/**
 * Write the `statuses` param as a comma-joined list in canonical `jobStatuses`
 * order, each member at most once. Removes the param when the set is empty so
 * an inactive dimension encodes to the absence of its parameter.
 */
export function encodeStatusesToUrl(params: URLSearchParams, statuses: JobStatus[]): void {
  const selected = new Set<string>(statuses.filter((status) => JOB_STATUS_SET.has(status)));
  if (selected.size === 0) {
    params.delete("statuses");
    return;
  }
  const ordered = jobStatuses.filter((status) => selected.has(status));
  params.set("statuses", ordered.join(","));
}

/**
 * Parse the date-filter params. Drops an out-of-domain field and any date that
 * is not a valid calendar date. Total (never throws).
 */
export function parseDateFilterFromUrl(params: URLSearchParams): {
  field?: DateField;
  from: string;
  to: string;
} {
  const rawField = params.get("dateField");
  const field = rawField && (DATE_FIELDS as readonly string[]).includes(rawField)
    ? (rawField as DateField)
    : undefined;

  const rawFrom = params.get("dateFrom");
  const rawTo = params.get("dateTo");
  const from = rawFrom && isValidCalendarDate(rawFrom) ? rawFrom : "";
  const to = rawTo && isValidCalendarDate(rawTo) ? rawTo : "";

  return field ? { field, from, to } : { from, to };
}

/** Parse the `remote` param; any non-member value falls back to `any`. Total. */
export function parseRemoteModeFromUrl(params: URLSearchParams): RemoteMode {
  const raw = params.get("remote");
  return raw && (REMOTE_MODES as readonly string[]).includes(raw) ? (raw as RemoteMode) : "any";
}

export function parseSalaryBound(value: string | null): number | undefined {
  if (value === null || !/^\d+$/.test(value)) return undefined;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : undefined;
}

export function validateSalaryRange(salaryMin: string, salaryMax: string): string | null {
  const minimum = salaryMin.trim();
  const maximum = salaryMax.trim();
  const parsedMinimum = minimum ? parseSalaryBound(minimum) : undefined;
  const parsedMaximum = maximum ? parseSalaryBound(maximum) : undefined;

  if ((minimum && parsedMinimum === undefined) || (maximum && parsedMaximum === undefined)) {
    return "Enter whole-dollar amounts of 0 or more.";
  }
  if (parsedMinimum !== undefined && parsedMaximum !== undefined && parsedMinimum > parsedMaximum) {
    return "Minimum income must not exceed maximum income.";
  }
  return null;
}

export function filterDraftFromUrl(
  search: string | null,
  postingState: string | null,
  salaryMin: string | null = null,
  salaryMax: string | null = null,
  params: URLSearchParams | null = null,
): JobFilterDraft {
  const statuses = params ? parseStatusesFromUrl(params) : [];
  const dateFilter = params ? parseDateFilterFromUrl(params) : { from: "", to: "" };
  const remoteMode = params ? parseRemoteModeFromUrl(params) : "any";
  const location = params?.get("location")?.trim() ?? "";
  return {
    search: search ?? "",
    postingState: postingState ?? "",
    salaryMin: salaryMin ?? "",
    salaryMax: salaryMax ?? "",
    statuses,
    dateField: dateFilter.field ?? "",
    dateFrom: dateFilter.from,
    dateTo: dateFilter.to,
    location,
    remoteMode,
  };
}

/**
 * Count the number of active "new" filter dimensions (0–3):
 * +1 when the status set is non-empty, +1 when a date field is chosen together
 * with at least one bound, and +1 when location text is present or the remote
 * mode is not `any`. Pure so it can be property-tested.
 */
export function countActiveFilterDimensions(input: {
  statuses: JobStatus[];
  dateField: "" | DateField;
  dateFrom: string;
  dateTo: string;
  location: string;
  remoteMode: RemoteMode;
}): number {
  let count = 0;
  if (input.statuses.length > 0) count += 1;
  if (input.dateField && (input.dateFrom.trim() !== "" || input.dateTo.trim() !== "")) count += 1;
  if (input.location.trim() !== "" || input.remoteMode !== "any") count += 1;
  return count;
}

export function filterCompaniesBySearch<T extends { name: string }>(companies: T[], search: string): T[] {
  const query = search.trim().toLocaleLowerCase();
  if (!query) return companies;

  return companies.filter((company) => company.name.toLocaleLowerCase().includes(query));
}
