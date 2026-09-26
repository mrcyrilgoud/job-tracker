import { describe, expect, it } from "vitest";

import { filterDraftFromUrl } from "./job-filters";

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
