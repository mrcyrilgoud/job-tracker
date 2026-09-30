import { describe, expect, it } from "vitest";

import { postingStatePresentation } from "./ui";

describe("postingStatePresentation", () => {
  it("distinguishes an unchecked unknown posting from a checked one", () => {
    expect(postingStatePresentation("unknown", null)).toEqual({
      label: "Not checked yet",
      tone: "stone",
    });
    expect(postingStatePresentation("unknown", "2026-09-30T00:00:00.000Z")).toEqual({
      label: "Couldn't confirm",
      tone: "stone",
    });
  });

  it("keeps active and inactive labels unchanged", () => {
    expect(postingStatePresentation("active", null).label).toBe("Open");
    expect(postingStatePresentation("inactive", "2026-09-30T00:00:00.000Z").label).toBe("Closed");
  });
});
