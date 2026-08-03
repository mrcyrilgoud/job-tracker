import { describe, expect, it } from "vitest";

import {
  groupRolesByCompany,
  matchesRoleSearch,
  providerLabel,
  roleCountLabel,
  syncErrorNote,
  watchPresentation,
} from "./companies-ui";
import type { CompanyWatch, Job, JobListItem } from "./schema";

function watch(overrides: Partial<CompanyWatch> = {}): CompanyWatch {
  return {
    id: "watch-1",
    companyId: "company-1",
    provider: "greenhouse",
    boardSlug: "acme",
    lastSyncedAt: "2026-08-01T00:00:00.000Z",
    consecutiveSyncFailures: 0,
    lastSyncError: null,
    createdAt: "2026-07-01T00:00:00.000Z",
    updatedAt: "2026-08-01T00:00:00.000Z",
    ...overrides,
  };
}

function role(overrides: Partial<Job> & Pick<Job, "id">, companyName = "Acme"): JobListItem {
  return {
    companyName,
    job: {
      companyId: "company-1",
      title: "Software Engineer",
      url: "https://example.com/jobs/1",
      canonicalUrl: "https://example.com/jobs/1",
      sourceExternalId: null,
      status: "wishlist",
      appliedAt: null,
      postingState: "active",
      lastCheckedAt: null,
      lastCheckResult: null,
      source: "greenhouse",
      notes: null,
      location: null,
      isNewFromWatch: true,
      missingFromSyncCount: 0,
      createdAt: "2026-07-01T00:00:00.000Z",
      updatedAt: "2026-07-02T00:00:00.000Z",
      ...overrides,
    },
  };
}

describe("providerLabel", () => {
  it("uses the vendor's own capitalization", () => {
    expect(providerLabel("greenhouse")).toBe("Greenhouse");
    expect(providerLabel("lever")).toBe("Lever");
    expect(providerLabel("ashby")).toBe("Ashby");
  });

  it("falls back to capitalizing an unknown provider", () => {
    expect(providerLabel("workday")).toBe("Workday");
  });
});

describe("watchPresentation", () => {
  it("reads as watching once a healthy sync has happened", () => {
    expect(watchPresentation(watch())).toEqual({ label: "Watching", tone: "green" });
  });

  it("distinguishes a brand new watch from a healthy one", () => {
    expect(watchPresentation(watch({ lastSyncedAt: null }))).toEqual({
      label: "Not checked yet",
      tone: "stone",
    });
  });

  it("treats the first couple of failures as retrying, not broken", () => {
    expect(watchPresentation(watch({ consecutiveSyncFailures: 1 })).label).toBe("Retrying");
    expect(watchPresentation(watch({ consecutiveSyncFailures: 2 })).label).toBe("Retrying");
  });

  it("escalates to needs attention once failures are sustained", () => {
    expect(watchPresentation(watch({ consecutiveSyncFailures: 3 }))).toEqual({
      label: "Needs attention",
      tone: "danger",
    });
  });

  it("prefers failure state over a stale successful sync", () => {
    const stale = watch({ consecutiveSyncFailures: 4, lastSyncedAt: "2026-07-01T00:00:00.000Z" });
    expect(watchPresentation(stale).tone).toBe("danger");
  });
});

describe("syncErrorNote", () => {
  it("matches the timeout cause inside a transport wrapper", () => {
    // Both "error sending request" and "operation timed out" appear; the
    // specific cause has to win.
    const raw =
      "error sending request for url (https://api.lever.co/v0/postings/stripe?mode=json): operation timed out";
    expect(syncErrorNote(raw, "lever")).toBe(
      "Lever didn't respond in time. We'll try again on the next sync.",
    );
  });

  it("explains a missing board and what to do about it", () => {
    expect(syncErrorNote("unexpected status 404 Not Found", "ashby")).toBe(
      "Ashby no longer has a board at this address. Check the board name, or remove this watch.",
    );
  });

  it("explains a private board", () => {
    expect(syncErrorNote("unexpected status 403 Forbidden", "greenhouse")).toContain(
      "made private",
    );
  });

  it("explains rate limiting", () => {
    expect(syncErrorNote("unexpected status 429", "greenhouse")).toContain("slow down");
  });

  it("blames the vendor for a server error rather than the user", () => {
    expect(syncErrorNote("unexpected status 503 Service Unavailable", "lever")).toBe(
      "Lever is having trouble on their end. We'll try again later.",
    );
  });

  it("falls back to a connection problem for bare transport failures", () => {
    expect(syncErrorNote("error sending request for url (https://x)", "greenhouse")).toBe(
      "Couldn't reach Greenhouse. Check your connection and try again.",
    );
  });

  it("never returns the raw string", () => {
    const raw = "thread 'main' panicked at src/ats/mod.rs:41";
    expect(syncErrorNote(raw, "greenhouse")).not.toContain("src/ats");
  });
});

describe("roleCountLabel", () => {
  it("pluralizes", () => {
    expect(roleCountLabel(1)).toBe("1 new role");
    expect(roleCountLabel(7)).toBe("7 new roles");
    expect(roleCountLabel(0)).toBe("0 new roles");
  });
});

describe("groupRolesByCompany", () => {
  it("buckets roles by company id preserving order", () => {
    const grouped = groupRolesByCompany([
      role({ id: "a", companyId: "c1", title: "One" }),
      role({ id: "b", companyId: "c2", title: "Two" }),
      role({ id: "c", companyId: "c1", title: "Three" }),
    ]);
    expect(grouped.get("c1")?.map((r) => r.job.title)).toEqual(["One", "Three"]);
    expect(grouped.get("c2")?.map((r) => r.job.title)).toEqual(["Two"]);
  });

  it("returns an empty map for no roles", () => {
    expect(groupRolesByCompany([]).size).toBe(0);
  });
});

describe("matchesRoleSearch", () => {
  const item = role({ id: "a", title: "Staff Frontend Engineer", location: "Remote (US)" }, "Stripe");

  it("matches everything when the query is blank", () => {
    expect(matchesRoleSearch(item, "   ")).toBe(true);
  });

  it("matches on title, location and company, case-insensitively", () => {
    expect(matchesRoleSearch(item, "frontend")).toBe(true);
    expect(matchesRoleSearch(item, "REMOTE")).toBe(true);
    expect(matchesRoleSearch(item, "stripe")).toBe(true);
  });

  it("rejects a non-match", () => {
    expect(matchesRoleSearch(item, "designer")).toBe(false);
  });

  it("tolerates a missing location", () => {
    expect(matchesRoleSearch(role({ id: "b", location: null }), "engineer")).toBe(true);
  });
});
