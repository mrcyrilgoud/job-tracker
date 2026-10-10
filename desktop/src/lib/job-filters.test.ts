import { describe, expect, it } from "vitest";

import { filterCompaniesBySearch, filterDraftFromUrl, validateSalaryRange } from "./job-filters";

describe("job filter draft", () => {
  it("hydrates both controls from the active URL filters", () => {
    expect(filterDraftFromUrl("OpenAI", "active")).toEqual({
      search: "OpenAI",
      postingState: "active",
      salaryMin: "",
      salaryMax: "",
    });
  });

  it("clears control values when URL navigation removes the filters", () => {
    expect(filterDraftFromUrl(null, null)).toEqual({
      search: "",
      postingState: "",
      salaryMin: "",
      salaryMax: "",
    });
  });

  it("hydrates annual salary bounds from the URL", () => {
    expect(filterDraftFromUrl("role", "active", "120000", "180000")).toEqual({
      search: "role",
      postingState: "active",
      salaryMin: "120000",
      salaryMax: "180000",
    });
  });
});

describe("company sidebar filter", () => {
  const companies = [
    { id: "1", name: "OpenAI" },
    { id: "2", name: "Anthropic" },
    { id: "3", name: "Acme Labs" },
  ];

  it("matches company names case-insensitively after trimming the query", () => {
    expect(filterCompaniesBySearch(companies, "  AI ")).toEqual([
      { id: "1", name: "OpenAI" },
    ]);
  });

  it("keeps every company when the query is blank", () => {
    expect(filterCompaniesBySearch(companies, "  ")).toEqual(companies);
  });
});

describe("annual income range validation", () => {
  it("allows either open bound and valid whole-dollar amounts", () => {
    expect(validateSalaryRange("", "")).toBeNull();
    expect(validateSalaryRange("120000", "")).toBeNull();
    expect(validateSalaryRange("", "180000")).toBeNull();
    expect(validateSalaryRange("120000", "180000")).toBeNull();
  });

  it("rejects negative, fractional, and unsafe amounts", () => {
    expect(validateSalaryRange("-1", "")).toBe("Enter whole-dollar amounts of 0 or more.");
    expect(validateSalaryRange("100.50", "")).toBe("Enter whole-dollar amounts of 0 or more.");
    expect(validateSalaryRange("999999999999999999999", "")).toBe("Enter whole-dollar amounts of 0 or more.");
  });

  it("rejects a minimum greater than the maximum", () => {
    expect(validateSalaryRange("180000", "120000")).toBe("Minimum income must not exceed maximum income.");
  });
});
