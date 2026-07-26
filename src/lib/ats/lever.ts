import { safeFetch } from "@/lib/jobs/safe-fetch";
import type { AtsAdapter, AtsJob } from "@/lib/ats/types";

export const leverAdapter: AtsAdapter = {
  provider: "lever",
  async validateBoard(boardSlug) {
    const result = await safeFetch(
      `https://api.lever.co/v0/postings/${encodeURIComponent(boardSlug)}?mode=json`,
      { accept: "application/json" },
    );
    if (!result.ok) {
      throw new Error(
        result.error ?? `Lever board "${boardSlug}" is unavailable (HTTP ${result.status})`,
      );
    }
    const parsed = JSON.parse(result.bodyText) as unknown;
    if (!Array.isArray(parsed)) {
      throw new Error("Unexpected Lever response shape");
    }
  },
  async listJobs(boardSlug) {
    const result = await safeFetch(
      `https://api.lever.co/v0/postings/${encodeURIComponent(boardSlug)}?mode=json`,
      { accept: "application/json" },
    );
    if (!result.ok) {
      throw new Error(result.error ?? `Failed to fetch Lever jobs (HTTP ${result.status})`);
    }

    const parsed = JSON.parse(result.bodyText) as Array<{
      id: string;
      text: string;
      hostedUrl: string;
      categories?: { location?: string };
    }>;

    if (!Array.isArray(parsed)) {
      throw new Error("Unexpected Lever response shape");
    }

    return parsed.map(
      (job): AtsJob => ({
        externalId: job.id,
        title: job.text,
        url: job.hostedUrl,
        location: job.categories?.location,
      }),
    );
  },
};
