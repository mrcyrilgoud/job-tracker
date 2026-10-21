import type {
  JobListSort,
  JobListSortDirection,
  JobListSortField,
  JobSort,
} from "@/lib/schema";

export const DEFAULT_APPLIED_SORT: JobListSort = {
  field: "appliedAt",
  direction: "descending",
};

export const DEFAULT_JOB_LIST_SORT: JobListSort = {
  field: "postingDate",
  direction: "descending",
};

const SORT_FIELDS: readonly JobListSortField[] = [
  "appliedAt",
  "postingDate",
  "appeal",
  "companyName",
];

const DIRECTIONS: readonly JobListSortDirection[] = ["ascending", "descending"];

const LEGACY_SORTS: Partial<Record<JobSort, JobListSort>> = {
  newestPosting: { field: "postingDate", direction: "descending" },
  oldest: { field: "postingDate", direction: "ascending" },
  appealHighToLow: { field: "appeal", direction: "descending" },
  appealLowToHigh: { field: "appeal", direction: "ascending" },
  companyAz: { field: "companyName", direction: "ascending" },
};

export function defaultJobListSortDirection(field: JobListSortField): JobListSortDirection {
  return field === "companyName" ? "ascending" : "descending";
}

function isSortField(value: string | null): value is JobListSortField {
  return SORT_FIELDS.includes(value as JobListSortField);
}

function isSortDirection(value: string | null): value is JobListSortDirection {
  return DIRECTIONS.includes(value as JobListSortDirection);
}

export function jobListSortFromUrl(
  searchParams: URLSearchParams,
  appliedTab: boolean,
  legacySort: JobSort | null,
): JobListSort {
  const requestedField = searchParams.has("sortBy")
    ? searchParams.get("sortBy")
    : searchParams.get("appliedSort");
  const fieldAvailable = (field: JobListSortField) => appliedTab || field !== "appliedAt";
  if (requestedField !== null) {
    if (!isSortField(requestedField) || !fieldAvailable(requestedField)) {
      return appliedTab ? DEFAULT_APPLIED_SORT : DEFAULT_JOB_LIST_SORT;
    }
    const directionValue = searchParams.has("sortOrder")
      ? searchParams.get("sortOrder")
      : searchParams.get("appliedOrder");
    const direction = isSortDirection(directionValue)
      ? directionValue
      : defaultJobListSortDirection(requestedField);
    return { field: requestedField, direction };
  }

  if (legacySort && LEGACY_SORTS[legacySort]) {
    const legacy = LEGACY_SORTS[legacySort]!;
    if (appliedTab && legacySort === "oldest") return { field: "appliedAt", direction: "ascending" };
    return legacy;
  }
  return appliedTab ? DEFAULT_APPLIED_SORT : DEFAULT_JOB_LIST_SORT;
}

/** Preserve compatibility for existing imports while sorting is shared by all Jobs views. */
export const appliedSortFromUrl = (searchParams: URLSearchParams, legacySort: JobSort | null) =>
  jobListSortFromUrl(searchParams, true, legacySort);
export const defaultAppliedSortDirection = defaultJobListSortDirection;
