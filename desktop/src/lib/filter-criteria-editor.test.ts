import { describe, expect, it } from "vitest";

import {
  addChip,
  commitCommaSeparated,
  isOverride,
  matchAllCriteria,
  previewLabel,
  removeChipAt,
  resetOverridePayload,
} from "./filter-criteria-editor";
import type { FilterCriteria } from "./schema";

describe("addChip", () => {
  it("drops empty and whitespace-only input, leaving tokens unchanged (Req 17.3)", () => {
    expect(addChip([], "")).toEqual([]);
    expect(addChip([], "  ")).toEqual([]);
    expect(addChip(["a"], "   ")).toEqual(["a"]);
  });

  it("trims the committed token", () => {
    expect(addChip([], "  qa ")).toEqual(["qa"]);
  });

  it("appends the trimmed token to the end", () => {
    expect(addChip(["a"], "b")).toEqual(["a", "b"]);
  });

  it("does not mutate the input array", () => {
    const tokens = ["a"];
    addChip(tokens, "b");
    expect(tokens).toEqual(["a"]);
  });
});

describe("removeChipAt", () => {
  it("removes the chip at the given index", () => {
    expect(removeChipAt(["a", "b", "c"], 1)).toEqual(["a", "c"]);
    expect(removeChipAt(["a", "b", "c"], 0)).toEqual(["b", "c"]);
    expect(removeChipAt(["a", "b", "c"], 2)).toEqual(["a", "b"]);
  });

  it("leaves the list unchanged for an out-of-range index", () => {
    expect(removeChipAt(["a", "b"], 5)).toEqual(["a", "b"]);
  });
});

describe("commitCommaSeparated", () => {
  it("passes the raw value through as the draft when there is no comma", () => {
    expect(commitCommaSeparated(["a"], "eng")).toEqual({ tokens: ["a"], draft: "eng" });
  });

  it("commits each part before the last comma and keeps the tail as the draft", () => {
    expect(commitCommaSeparated([], "qa, product,eng")).toEqual({
      tokens: ["qa", "product"],
      draft: "eng",
    });
  });

  it("drops empty parts between commas (Req 17.3)", () => {
    expect(commitCommaSeparated([], "qa,, ,eng,")).toEqual({
      tokens: ["qa", "eng"],
      draft: "",
    });
  });
});

describe("matchAllCriteria", () => {
  it("has the match-all shape used to seed editors", () => {
    expect(matchAllCriteria()).toEqual({
      version: 1,
      title: { include: [], exclude: [], matchMode: "word" },
      location: { country: null, include: [], exclude: [], matchMode: "word" },
      remote: "any",
    });
  });

  it("returns a fresh object each call so edits do not leak between editors", () => {
    const a = matchAllCriteria();
    const b = matchAllCriteria();
    expect(a).not.toBe(b);
    a.title.include.push("engineer");
    expect(b.title.include).toEqual([]);
  });
});

describe("isOverride", () => {
  it("is false for null or undefined (Using global filter, Req 13.2)", () => {
    expect(isOverride(null)).toBe(false);
    expect(isOverride(undefined)).toBe(false);
  });

  it("is true for a real criteria object (Custom filter, Req 13.2)", () => {
    const criteria: FilterCriteria = matchAllCriteria();
    expect(isOverride(criteria)).toBe(true);
  });
});

describe("resetOverridePayload", () => {
  it("is the null payload sent to reset a board to the global filter (Req 13.4)", () => {
    expect(resetOverridePayload).toBeNull();
    // Resetting produces no override, i.e. the board reverts to the global filter.
    expect(isOverride(resetOverridePayload)).toBe(false);
  });
});

describe("previewLabel", () => {
  it("maps an included outcome to Shown (Req 13.5)", () => {
    expect(previewLabel({ included: true })).toBe("Shown");
  });

  it("maps an excluded outcome to Hidden (Req 13.5)", () => {
    expect(previewLabel({ included: false })).toBe("Hidden");
  });

  it("treats a missing outcome as Hidden", () => {
    expect(previewLabel(null)).toBe("Hidden");
    expect(previewLabel(undefined)).toBe("Hidden");
  });
});
