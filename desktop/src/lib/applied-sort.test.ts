import { describe, expect, it } from "vitest";

import {
  appliedSortFromUrl,
  defaultAppliedSortDirection,
  DEFAULT_APPLIED_SORT,
} from "@/lib/applied-sort";

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
});
