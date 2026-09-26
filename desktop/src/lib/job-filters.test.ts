import { describe, expect, it } from "vitest";

import { filterCompaniesBySearch, filterDraftFromUrl } from "./job-filters";

describe("job filter draft", () => {
  it("hydrates both controls from the active URL filters", () => {
    expect(filterDraftFromUrl("OpenAI", "active")).toEqual({
      search: "OpenAI",
      postingState: "active",
    });
  });

  it("clears control values when URL navigation removes the filters", () => {
    expect(filterDraftFromUrl(null, null)).toEqual({
      search: "",
      postingState: "",
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
