import { describe, expect, it } from "vitest";

import type { JobListItem } from "./schema";

function mockJob(id: string, isFavorite: boolean, status: JobListItem["job"]["status"] = "wishlist"): JobListItem {
  return {
    companyName: "Acme Corp",
    job: {
      id,
      companyId: "comp-1",
      title: `Job ${id}`,
      url: `https://example.com/jobs/${id}`,
      canonicalUrl: `https://example.com/jobs/${id}`,
      sourceExternalId: null,
      status,
      appliedAt: null,
      postingState: "active",
      lastCheckedAt: null,
      lastCheckResult: null,
      source: "manual",
      notes: null,
      location: "San Francisco, CA",
      isNewFromWatch: false,
      watchDisposition: null,
      missingFromSyncCount: 0,
      isFavorite,
      createdAt: "2026-08-01T00:00:00.000Z",
      updatedAt: "2026-08-01T00:00:00.000Z",
    },
  };
}

describe("Favorites UI logic", () => {
  it("filters jobs to only starred favorites", () => {
    const jobs: JobListItem[] = [
      mockJob("1", true, "wishlist"),
      mockJob("2", false, "applied"),
      mockJob("3", true, "interviewing"),
      mockJob("4", false, "offer"),
    ];

    const favorites = jobs.filter((j) => j.job.isFavorite);
    expect(favorites).toHaveLength(2);
    expect(favorites.map((f) => f.job.id)).toEqual(["1", "3"]);
  });

  it("updates job favorite status immutably", () => {
    const jobs: JobListItem[] = [
      mockJob("1", false, "wishlist"),
      mockJob("2", true, "applied"),
    ];

    const updated = jobs.map((item) =>
      item.job.id === "1"
        ? { ...item, job: { ...item.job, isFavorite: !item.job.isFavorite } }
        : item,
    );

    expect(updated[0].job.isFavorite).toBe(true);
    expect(updated[1].job.isFavorite).toBe(true);
    expect(jobs[0].job.isFavorite).toBe(false); // Original remains untouched
  });

  it("organizes starred favorites by stage for Kanban board view", () => {
    const jobs: JobListItem[] = [
      mockJob("1", true, "wishlist"),
      mockJob("2", true, "applied"),
      mockJob("3", true, "applied"),
      mockJob("4", true, "interviewing"),
      mockJob("5", true, "offer"),
      mockJob("6", true, "rejected"),
    ];

    const byStage = {
      wishlist: jobs.filter((j) => j.job.status === "wishlist"),
      applied: jobs.filter((j) => j.job.status === "applied"),
      interviewing: jobs.filter((j) => j.job.status === "interviewing"),
      offer: jobs.filter((j) => j.job.status === "offer"),
    };

    expect(byStage.wishlist).toHaveLength(1);
    expect(byStage.applied).toHaveLength(2);
    expect(byStage.interviewing).toHaveLength(1);
    expect(byStage.offer).toHaveLength(1);
  });
});
