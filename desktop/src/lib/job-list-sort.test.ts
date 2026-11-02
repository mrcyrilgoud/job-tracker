import { describe, expect, it } from "vitest";

import {
  appliedSortFromUrl,
  defaultAppliedSortDirection,
  DEFAULT_APPLIED_SORT,
  DEFAULT_JOB_LIST_SORT,
  jobListSortFromUrl,
} from "@/lib/job-list-sort";

describe("Applied sort URL state", () => {
  it("defaults to newest date applied first", () => {
    expect(appliedSortFromUrl(new URLSearchParams(), null)).toEqual(DEFAULT_APPLIED_SORT);
  });

  it("uses the natural direction when the URL has a field without an order", () => {
    expect(appliedSortFromUrl(new URLSearchParams("appliedSort=companyName"), null)).toEqual({
      field: "companyName",
      direction: "ascending",
    });
    expect(defaultAppliedSortDirection("appliedAt")).toBe("descending");
    expect(defaultAppliedSortDirection("appeal")).toBe("descending");
  });

  it("translates existing sort URLs for the Applied tab", () => {
    for (const [legacySort, expected] of [
      ["newestPosting", { field: "postingDate", direction: "descending" }],
      ["oldest", { field: "appliedAt", direction: "ascending" }],
      ["appealHighToLow", { field: "appeal", direction: "descending" }],
      ["appealLowToHigh", { field: "appeal", direction: "ascending" }],
      ["companyAz", { field: "companyName", direction: "ascending" }],
    ] as const) {
      expect(appliedSortFromUrl(new URLSearchParams(), legacySort)).toEqual(expected);
    }
  });

  it("lets the explicit Applied parameters override legacy sort values", () => {
    expect(appliedSortFromUrl(
      new URLSearchParams("sort=oldest&appliedSort=postingDate&appliedOrder=ascending"),
      "oldest",
    )).toEqual({ field: "postingDate", direction: "ascending" });
  });

  it("uses context defaults and rejects date applied outside Applied", () => {
    expect(jobListSortFromUrl(new URLSearchParams(), false, null)).toEqual(DEFAULT_JOB_LIST_SORT);
    expect(jobListSortFromUrl(new URLSearchParams("sortBy=appliedAt&sortOrder=ascending"), false, null))
      .toEqual(DEFAULT_JOB_LIST_SORT);
    expect(jobListSortFromUrl(new URLSearchParams("sortBy=invalid&sort=companyAz"), false, "companyAz"))
      .toEqual(DEFAULT_JOB_LIST_SORT);
  });

  it("uses shared URL parameters and translates legacy dates by context", () => {
    expect(jobListSortFromUrl(new URLSearchParams("sortBy=appeal&sortOrder=ascending&appliedSort=companyName"), false, null))
      .toEqual({ field: "appeal", direction: "ascending" });
    expect(jobListSortFromUrl(new URLSearchParams("sort=oldest"), false, "oldest"))
      .toEqual({ field: "postingDate", direction: "ascending" });
    expect(jobListSortFromUrl(new URLSearchParams("sort=oldest"), true, "oldest"))
      .toEqual({ field: "appliedAt", direction: "ascending" });
  });
});

describe("Company stability sort URL state", () => {
  // Validates: Requirements 6.2, 6.7, 6.8
  it.each([
    [true, "sortBy=companyStability&sortOrder=ascending", "ascending"],
    [true, "sortBy=companyStability&sortOrder=descending", "descending"],
    [true, "sortBy=companyStability", "descending"],
    [true, "sortBy=companyStability&sortOrder=sideways", "descending"],
    [false, "sortBy=companyStability&sortOrder=ascending", "ascending"],
    [false, "sortBy=companyStability&sortOrder=descending", "descending"],
    [false, "sortBy=companyStability", "descending"],
    [false, "sortBy=companyStability&sortOrder=sideways", "descending"],
  ] as const)("applied tab %s: %s sorts %s", (appliedTab, query, direction) => {
    expect(jobListSortFromUrl(new URLSearchParams(query), appliedTab, null))
      .toEqual({ field: "companyStability", direction });
  });
});
