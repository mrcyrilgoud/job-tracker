import type {
  AppliedJobSort,
  AppliedJobSortDirection,
  AppliedJobSortField,
  JobSort,
} from "@/lib/schema";

export const DEFAULT_APPLIED_SORT: AppliedJobSort = {
  field: "appliedAt",
  direction: "descending",
};

const APPLIED_SORT_FIELDS: readonly AppliedJobSortField[] = [
  "appliedAt",
  "postingDate",
  "appeal",
  "companyName",
];

const DIRECTIONS: readonly AppliedJobSortDirection[] = ["ascending", "descending"];

const LEGACY_SORTS: Partial<Record<JobSort, AppliedJobSort>> = {
  newestPosting: { field: "postingDate", direction: "descending" },
  oldest: { field: "appliedAt", direction: "ascending" },
  appealHighToLow: { field: "appeal", direction: "descending" },
  appealLowToHigh: { field: "appeal", direction: "ascending" },
  companyAz: { field: "companyName", direction: "ascending" },
};

export function defaultAppliedSortDirection(field: AppliedJobSortField): AppliedJobSortDirection {
  return field === "companyName" ? "ascending" : "descending";
}

export function appliedSortFromUrl(
  searchParams: URLSearchParams,
  legacySort: JobSort | null,
): AppliedJobSort {
  const requestedField = searchParams.get("appliedSort");
  if (APPLIED_SORT_FIELDS.includes(requestedField as AppliedJobSortField)) {
    const field = requestedField as AppliedJobSortField;
    const requestedDirection = searchParams.get("appliedOrder");
    const direction = DIRECTIONS.includes(requestedDirection as AppliedJobSortDirection)
      ? requestedDirection as AppliedJobSortDirection
      : defaultAppliedSortDirection(field);
    return { field, direction };
  }

  return (legacySort && LEGACY_SORTS[legacySort]) ?? DEFAULT_APPLIED_SORT;
}
