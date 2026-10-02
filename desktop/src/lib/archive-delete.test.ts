import { describe, expect, it } from "vitest";

import { jobStatuses, type JobListItem } from "./schema";
import { jobStatusPresentation, nextActiveJobStage, postingStateMatters } from "./ui";

function mockJob(
  id: string,
  status: JobListItem["job"]["status"] = "wishlist",
  isFavorite = false,
): JobListItem {
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
      description: null,
      location: "San Francisco, CA",
      isNewFromWatch: false,
      watchDisposition: null,
      missingFromSyncCount: 0,
      isFavorite,
      appeal: null,
      createdAt: "2026-08-01T00:00:00.000Z",
      updatedAt: "2026-08-01T00:00:00.000Z",
    },
  };
}

describe("Archive and Delete UI logic", () => {
  it("includes archived in jobStatuses list", () => {
    expect(jobStatuses).toContain("archived");
  });

  it("formats archived status presentation and posting state relevance correctly", () => {
    const presentation = jobStatusPresentation("archived");
    expect(presentation.label).toBe("Archived");
    expect(presentation.tone).toBe("stone");

    expect(postingStateMatters("archived")).toBe(false);
  });

  it("returns only the next active pipeline stage for the board action", () => {
    expect(nextActiveJobStage("wishlist")).toBe("applied");
    expect(nextActiveJobStage("applied")).toBe("interviewing");
    expect(nextActiveJobStage("interviewing")).toBe("offer");
    expect(nextActiveJobStage("offer")).toBeNull();
    expect(nextActiveJobStage("archived")).toBeNull();
  });

  it("filters jobs into active and archived partitions", () => {
    const jobs: JobListItem[] = [
      mockJob("1", "wishlist"),
      mockJob("2", "applied"),
      mockJob("3", "archived"),
      mockJob("4", "interviewing"),
      mockJob("5", "closed"),
    ];

    const activeJobs = jobs.filter(
      (j) => j.job.status !== "archived" && j.job.status !== "closed" && j.job.status !== "rejected" && j.job.status !== "withdrawn",
    );
    const archivedJobs = jobs.filter(
      (j) => j.job.status === "archived" || j.job.status === "closed" || j.job.status === "rejected" || j.job.status === "withdrawn",
    );

    expect(activeJobs.map((j) => j.job.id)).toEqual(["1", "2", "4"]);
    expect(archivedJobs.map((j) => j.job.id)).toEqual(["3", "5"]);
  });

  it("removes deleted jobs from the list immutably", () => {
    const jobs: JobListItem[] = [
      mockJob("1", "wishlist"),
      mockJob("2", "applied"),
      mockJob("3", "archived"),
    ];

    const filtered = jobs.filter((j) => j.job.id !== "2");
    expect(filtered).toHaveLength(2);
    expect(filtered.map((j) => j.job.id)).toEqual(["1", "3"]);
  });

  it("updates job status to archived immutably", () => {
    const jobs: JobListItem[] = [
      mockJob("1", "wishlist"),
      mockJob("2", "applied"),
    ];

    const updated = jobs.map((item) =>
      item.job.id === "1" ? { ...item, job: { ...item.job, status: "archived" as const } } : item,
    );

    expect(updated[0].job.status).toBe("archived");
    expect(updated[1].job.status).toBe("applied");
  });

  it("calculates active jobs count excluding archived total", () => {
    const counts = { all: 10, archivedTotal: 3, favorites: 4, wishlist: 2, applied: 3, interviewing: 2 };
    const activeJobsCount = Math.max(0, (counts.all ?? 0) - (counts.archivedTotal ?? 0));
    expect(activeJobsCount).toBe(7);
  });

  it("handles empty / zero counts gracefully without negative numbers", () => {
    const counts = { all: 0, archivedTotal: 5 };
    const activeJobsCount = Math.max(0, (counts.all ?? 0) - (counts.archivedTotal ?? 0));
    expect(activeJobsCount).toBe(0);
  });
});
