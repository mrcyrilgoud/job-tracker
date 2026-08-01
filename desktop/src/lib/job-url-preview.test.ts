import { describe, expect, it } from "vitest";

import type { JobUrlPreview } from "./api";
import type { Job, JobListItem } from "./schema";
import {
  applyJobUrlPreview,
  confirmJobUrlPreview,
  defaultStoredPostingUrl,
  formatInvokeError,
  formatJobSaveError,
  formatStoredPostingOption,
  isConfirmedJobDiscovery,
  resetJobUrlDiscovery,
  serializeConfirmedJobDiscovery,
  storedPostingLinksFromJobs,
} from "./job-url-preview";

function jobItem(overrides: Partial<Job> & Pick<Job, "id" | "url">): JobListItem {
  return {
    companyName: "Acme",
    job: {
      companyId: "company-1",
      title: "Software Engineer",
      canonicalUrl: overrides.url,
      sourceExternalId: null,
      status: "wishlist",
      appliedAt: null,
      postingState: "unknown",
      lastCheckedAt: null,
      lastCheckResult: null,
      source: "manual",
      notes: null,
      location: null,
      isNewFromWatch: false,
      missingFromSyncCount: 0,
      createdAt: "2026-07-01T00:00:00.000Z",
      updatedAt: "2026-07-02T00:00:00.000Z",
      ...overrides,
    },
  };
}

const boardPreview: JobUrlPreview = {
  title: "Software Engineer",
  companyName: "Acme",
  board: {
    provider: "ashby",
    boardSlug: "acme",
    boardUrl: "https://jobs.ashbyhq.com/acme",
    postingId: "posting-1",
  },
  careersUrl: null,
};

const careersPreview: JobUrlPreview = {
  title: null,
  companyName: "Onebrief",
  board: null,
  careersUrl: "https://www.onebrief.com/careers",
};

describe("applyJobUrlPreview", () => {
  it("fills both blank fields from a complete preview", () => {
    expect(
      applyJobUrlPreview(
        { title: "", companyName: "" },
        {
          title: "Software Engineer",
          companyName: "Acme",
          board: null,
          careersUrl: null,
        },
      ),
    ).toEqual({ title: "Software Engineer", companyName: "Acme" });
  });

  it("preserves non-empty manually entered values", () => {
    expect(
      applyJobUrlPreview(
        { title: "My title", companyName: "My company" },
        {
          title: "Detected title",
          companyName: "Detected company",
          board: null,
          careersUrl: null,
        },
      ),
    ).toEqual({ title: "My title", companyName: "My company" });
  });

  it("fills only the available blank field from a partial preview", () => {
    expect(
      applyJobUrlPreview(
        { title: "", companyName: "" },
        {
          title: "Product Designer",
          companyName: null,
          board: null,
          careersUrl: null,
        },
      ),
    ).toEqual({ title: "Product Designer", companyName: "" });
  });

  it("leaves current values unchanged for an empty preview", () => {
    expect(
      applyJobUrlPreview(
        { title: "", companyName: "Existing company" },
        { title: null, companyName: null, board: null, careersUrl: null },
      ),
    ).toEqual({ title: "", companyName: "Existing company" });
  });

  it("starts detected boards unconfirmed and confirms them explicitly", () => {
    const confirmed = confirmJobUrlPreview(boardPreview);

    expect(confirmed).toEqual({ provider: "ashby", boardSlug: "acme" });
    expect(isConfirmedJobDiscovery(boardPreview, null)).toBe(false);
    expect(isConfirmedJobDiscovery(boardPreview, confirmed)).toBe(true);
  });

  it("supports careers-only fallback without creating a board confirmation", () => {
    const confirmed = confirmJobUrlPreview(careersPreview);

    expect(confirmed).toEqual({ careersUrl: "https://www.onebrief.com/careers" });
    expect(isConfirmedJobDiscovery(careersPreview, confirmed)).toBe(true);
    expect(confirmed?.provider).toBeUndefined();
    expect(confirmed?.boardSlug).toBeUndefined();
  });

  it("serializes only an explicit confirmation for the current preview", () => {
    expect(serializeConfirmedJobDiscovery(boardPreview, null)).toBeNull();
    expect(
      serializeConfirmedJobDiscovery(boardPreview, {
        provider: "ashby",
        boardSlug: "acme",
      }),
    ).toEqual({ provider: "ashby", boardSlug: "acme" });
    expect(
      serializeConfirmedJobDiscovery(boardPreview, {
        provider: "greenhouse",
        boardSlug: "acme",
      }),
    ).toBeNull();
    expect(
      serializeConfirmedJobDiscovery(careersPreview, {
        careersUrl: careersPreview.careersUrl!,
      }),
    ).toEqual({ careersUrl: "https://www.onebrief.com/careers" });
  });

  it("clears stale discovery when the posting URL changes", () => {
    expect(resetJobUrlDiscovery()).toEqual({
      preview: null,
      confirmedDiscovery: null,
    });
  });

  it("turns board validation failures into an actionable watch warning", () => {
    expect(formatJobSaveError(new Error("Ashby board unavailable"))).toBe(
      "Ashby board unavailable The watch was not created. Retry or set it up manually from Companies.",
    );
    expect(formatJobSaveError(new Error("A job with this URL is already tracked"))).toBe(
      "A job with this URL is already tracked",
    );
  });

  it("reads Tauri string and object invoke failures", () => {
    expect(formatInvokeError("missing required key input", "fallback")).toBe(
      "missing required key input",
    );
    expect(formatInvokeError({ message: "Ashby board unavailable" }, "fallback")).toBe(
      "Ashby board unavailable",
    );
    expect(formatInvokeError(null, "Could not save this discovery.")).toBe(
      "Could not save this discovery.",
    );
  });
});

describe("storedPostingLinksFromJobs", () => {
  it("keeps newest-first unique http posting URLs", () => {
    const links = storedPostingLinksFromJobs([
      jobItem({
        id: "job-1",
        title: "Staff Engineer",
        url: "https://jobs.ashbyhq.com/acme/staff",
        updatedAt: "2026-07-10T00:00:00.000Z",
      }),
      jobItem({
        id: "job-2",
        title: "Staff Engineer duplicate",
        url: "https://jobs.ashbyhq.com/acme/staff",
        updatedAt: "2026-07-09T00:00:00.000Z",
      }),
      jobItem({
        id: "job-3",
        title: "Designer",
        url: "https://jobs.lever.co/acme/designer",
        updatedAt: "2026-07-08T00:00:00.000Z",
      }),
      jobItem({
        id: "job-4",
        title: "Bad link",
        url: "not-a-url",
        updatedAt: "2026-07-07T00:00:00.000Z",
      }),
    ]);

    expect(links).toEqual([
      {
        jobId: "job-1",
        title: "Staff Engineer",
        url: "https://jobs.ashbyhq.com/acme/staff",
        updatedAt: "2026-07-10T00:00:00.000Z",
      },
      {
        jobId: "job-3",
        title: "Designer",
        url: "https://jobs.lever.co/acme/designer",
        updatedAt: "2026-07-08T00:00:00.000Z",
      },
    ]);
    expect(defaultStoredPostingUrl(links)).toBe("https://jobs.ashbyhq.com/acme/staff");
  });

  it("falls back to canonical URL and formats select labels", () => {
    const links = storedPostingLinksFromJobs([
      jobItem({
        id: "job-5",
        title: "",
        url: "   ",
        canonicalUrl: "https://boards.greenhouse.io/acme/jobs/12345678901234567890",
      }),
    ]);

    expect(links).toHaveLength(1);
    expect(links[0]?.title).toBe("Untitled role");
    expect(links[0]?.url).toBe("https://boards.greenhouse.io/acme/jobs/12345678901234567890");
    expect(formatStoredPostingOption(links[0]!, 40)).toBe(
      "Untitled role — https://boards.greenhouse.io/acme/jobs/…",
    );
    expect(defaultStoredPostingUrl([])).toBe("");
  });
});
