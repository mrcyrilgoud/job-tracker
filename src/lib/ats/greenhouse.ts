import { safeFetch } from "@/lib/jobs/safe-fetch";
import type { AtsAdapter, AtsJob } from "@/lib/ats/types";

export const greenhouseAdapter: AtsAdapter = {
  provider: "greenhouse",
  async validateBoard(boardSlug) {
    const result = await safeFetch(
      `https://boards-api.greenhouse.io/v1/boards/${encodeURIComponent(boardSlug)}/jobs`,
      { accept: "application/json" },
    );
    if (!result.ok) {
      throw new Error(
        result.error ?? `Greenhouse board "${boardSlug}" is unavailable (HTTP ${result.status})`,
      );
    }
    const parsed = JSON.parse(result.bodyText) as { jobs?: unknown };
    if (!Array.isArray(parsed.jobs)) {
      throw new Error("Unexpected Greenhouse response shape");
    }
  },
  async listJobs(boardSlug) {
    const result = await safeFetch(
      `https://boards-api.greenhouse.io/v1/boards/${encodeURIComponent(boardSlug)}/jobs`,
      { accept: "application/json" },
    );
    if (!result.ok) {
      throw new Error(
        result.error ?? `Failed to fetch Greenhouse jobs (HTTP ${result.status})`,
      );
    }

    const parsed = JSON.parse(result.bodyText) as {
      jobs?: Array<{
        id: number | string;
        title: string;
        absolute_url: string;
        location?: { name?: string };
      }>;
    };

    if (!Array.isArray(parsed.jobs)) {
      throw new Error("Unexpected Greenhouse response shape");
    }

    return parsed.jobs.map(
      (job): AtsJob => ({
        externalId: String(job.id),
        title: job.title,
        url: job.absolute_url,
        location: job.location?.name,
      }),
    );
  },
};
