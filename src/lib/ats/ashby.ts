import { safeFetch } from "@/lib/jobs/safe-fetch";
import type { AtsAdapter, AtsJob } from "@/lib/ats/types";

type AshbyJob = {
  id: string;
  title: string;
  jobUrl: string;
  location?: string;
};

export const ashbyAdapter: AtsAdapter = {
  provider: "ashby",
  async validateBoard(boardSlug) {
    const result = await safeFetch(
      `https://api.ashbyhq.com/posting-api/job-board/${encodeURIComponent(boardSlug)}`,
      { accept: "application/json" },
    );
    if (!result.ok) {
      throw new Error(
        result.error ?? `Ashby board "${boardSlug}" is unavailable (HTTP ${result.status})`,
      );
    }
    const parsed = JSON.parse(result.bodyText) as { jobs?: unknown };
    if (!Array.isArray(parsed.jobs)) {
      throw new Error("Unexpected Ashby response shape");
    }
  },
  async listJobs(boardSlug) {
    const result = await safeFetch(
      `https://api.ashbyhq.com/posting-api/job-board/${encodeURIComponent(boardSlug)}`,
      { accept: "application/json" },
    );
    if (!result.ok) {
      throw new Error(result.error ?? `Failed to fetch Ashby jobs (HTTP ${result.status})`);
    }

    const parsed = JSON.parse(result.bodyText) as { jobs?: AshbyJob[] };
    if (!Array.isArray(parsed.jobs)) {
      throw new Error("Unexpected Ashby response shape");
    }

    return parsed.jobs.map(
      (job): AtsJob => ({
        externalId: job.id,
        title: job.title,
        url: job.jobUrl,
        location: job.location,
      }),
    );
  },
};
