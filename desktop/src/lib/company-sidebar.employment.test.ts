import fc from "fast-check";
import { describe, expect, it } from "vitest";
import {
  COMPANY_SORT_OPTIONS,
  deriveCompanyList,
  parseCompanyViewState,
  serializeCompanyViewState,
  sortAnnotationFor,
  type CompanyFilter,
} from "./company-sidebar";
import type { CompanyRow } from "./schema";
import { arbMaybeMalformedRows, pipelineRow } from "./__fixtures__/company-row-arbitraries";

const PBT = { numRuns: 100 } as const;
const SCORE_POOL: ReadonlyArray<unknown> = [null, 1, 2, 3, 4, 5, undefined, "4", 0, 5.5, Number.NaN, Infinity, -Infinity];
const FILTERS: readonly CompanyFilter[] = [
  "watched",
  "hasOpenPositions",
  "hasTrackedJobs",
  "needsAttention",
  "hasWishlistJobs",
  "hasAppliedJobs",
];
const SCORE_SORTS = ["mostStable", "mostStableEmployment"] as const;
const ids = (rows: readonly CompanyRow[]) => rows.map((row) => row.company.id);

function score(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && Number.isInteger(value) && value >= 1 && value <= 5
    ? value
    : null;
}

function refNameOrder(a: CompanyRow, b: CompanyRow): number {
  const byName = new Intl.Collator(undefined, { sensitivity: "accent" }).compare(a.company.name, b.company.name);
  if (byName !== 0) return byName < 0 ? -1 : 1;
  return a.company.id < b.company.id ? -1 : a.company.id > b.company.id ? 1 : 0;
}

function refScore(row: CompanyRow, sort: (typeof SCORE_SORTS)[number]): number | null {
  return score(sort === "mostStable" ? row.company.companyStability : row.company.employmentStability);
}

function refSort(rows: readonly CompanyRow[], sort: (typeof SCORE_SORTS)[number]): CompanyRow[] {
  return [...rows].sort((a, b) => {
    const sa = refScore(a, sort);
    const sb = refScore(b, sort);
    if ((sa === null) !== (sb === null)) return sa === null ? 1 : -1;
    if (sa !== null && sb !== null && sa !== sb) return sb - sa;
    return refNameOrder(a, b);
  });
}

function sanitizedCount(value: unknown): number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 ? value : 0;
}

function refMatchesFilter(row: CompanyRow, filter: CompanyFilter): boolean {
  switch (filter) {
    case "watched":
      return row.watches.length > 0;
    case "hasOpenPositions":
      return sanitizedCount(row.openPositionCount) > 0;
    case "hasTrackedJobs":
      return sanitizedCount(row.trackedJobCount) > 0;
    case "needsAttention":
      return row.watches.some((watch) => watch.consecutiveSyncFailures > 0) || row.reviews.length > 0;
    case "hasWishlistJobs":
      return sanitizedCount(row.wishlistJobCount) >= 1;
    case "hasAppliedJobs":
      return sanitizedCount(row.appliedJobCount) >= 1;
  }
}

function withEmploymentScores(rows: CompanyRow[], values: unknown[]): CompanyRow[] {
  return rows.map((row, index) => ({
    ...row,
    company: { ...row.company, employmentStability: values[index] as CompanyRow["company"]["employmentStability"] },
  }));
}

describe("company sidebar company-owned stability sorts", () => {
  it("keeps the legacy mostStable saved sort and round-trips the new employment sort", () => {
    const legacy = parseCompanyViewState(JSON.stringify({ version: 1, sort: "mostStable", filters: ["watched"] }));
    expect(legacy.sort).toBe("mostStable");
    expect([...legacy.filters]).toEqual(["watched"]);

    const current = parseCompanyViewState(
      serializeCompanyViewState({ sort: "mostStableEmployment", filters: new Set(["hasAppliedJobs"]) }),
    );
    expect(current.sort).toBe("mostStableEmployment");
    expect([...current.filters]).toEqual(["hasAppliedJobs"]);

    const companyIndex = COMPANY_SORT_OPTIONS.findIndex((option) => option.value === "mostStable");
    const employmentIndex = COMPANY_SORT_OPTIONS.findIndex((option) => option.value === "mostStableEmployment");
    expect(employmentIndex).toBe(companyIndex + 1);
    expect(COMPANY_SORT_OPTIONS[companyIndex]?.group).toBe(COMPANY_SORT_OPTIONS[employmentIndex]?.group);
  });

  it("uses whole-number target annotations and suppresses annotations for unscored targets", () => {
    const companyScored = pipelineRow("company", "Company", { companyStability: 4 });
    expect(sortAnnotationFor(companyScored, "mostStable", { now: new Date("2026-01-01T00:00:00Z") })).toEqual({
      text: "Company 4",
      label: "Company stability 4",
    });

    const employmentScored = pipelineRow("employment", "Employment", { companyStability: 4 });
    employmentScored.company.employmentStability = 5;
    expect(
      sortAnnotationFor(employmentScored, "mostStableEmployment", { now: new Date("2026-01-01T00:00:00Z") }),
    ).toEqual({ text: "Employment 5", label: "Employment stability 5" });

    const companyUnscored = pipelineRow("unscored-company", "Unscored company", { companyStability: null });
    companyUnscored.company.employmentStability = 5;
    expect(sortAnnotationFor(companyUnscored, "mostStable", { now: new Date() })).toBeNull();

    const employmentUnscored = pipelineRow("unscored-employment", "Unscored employment", { companyStability: 5 });
    employmentUnscored.company.employmentStability = null;
    expect(sortAnnotationFor(employmentUnscored, "mostStableEmployment", { now: new Date() })).toBeNull();
  });

  // Feature: company-employment-stability, Property 14: Sidebar sorting sanitizes malformed scores and suppresses irrelevant annotations
  it("Property 14: sanitizes malformed scores, filters rows, and suppresses irrelevant annotations", () => {
    fc.assert(
      fc.property(
        arbMaybeMalformedRows().chain((rows) => fc.tuple(...rows.map(() => fc.constantFrom(...SCORE_POOL))).map((scores) => [rows, scores] as const)),
        fc.subarray([...FILTERS]),
        fc.constantFrom(...SCORE_SORTS),
        ([rawRows, employmentScores], activeFilters, sort) => {
          const rows = withEmploymentScores(rawRows, employmentScores);
          const filters = new Set<CompanyFilter>(activeFilters);
          const expected = refSort(
            rows.filter((row) => activeFilters.every((filter) => refMatchesFilter(row, filter))),
            sort,
          );
          const actual = deriveCompanyList(rows, "", { sort, filters }).visible;
          expect(ids(actual)).toEqual(ids(expected));

          for (const row of rows) {
            const target = refScore(row, sort);
            const annotation = sortAnnotationFor(row, sort, { now: new Date("2026-01-01T00:00:00Z") });
            if (target === null) {
              expect(annotation).toBeNull();
            } else if (sort === "mostStable") {
              expect(annotation).toEqual({ text: `Company ${target}`, label: `Company stability ${target}` });
            } else {
              expect(annotation).toEqual({ text: `Employment ${target}`, label: `Employment stability ${target}` });
            }
          }

          // Keep the filter set live in this property so its generated value is part of the contract.
          expect(filters.size).toBe(activeFilters.length);
        },
      ),
      PBT,
    );
  });
});
