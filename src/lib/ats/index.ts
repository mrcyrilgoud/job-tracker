import { ashbyAdapter } from "@/lib/ats/ashby";
import { greenhouseAdapter } from "@/lib/ats/greenhouse";
import { leverAdapter } from "@/lib/ats/lever";
import type { AtsAdapter } from "@/lib/ats/types";
import type { WatchProvider } from "@/lib/db/schema";

const adapters: Record<WatchProvider, AtsAdapter> = {
  greenhouse: greenhouseAdapter,
  lever: leverAdapter,
  ashby: ashbyAdapter,
};

export function getAtsAdapter(provider: WatchProvider): AtsAdapter {
  return adapters[provider];
}

export function parseAtsJobsFromJson(
  provider: WatchProvider,
  bodyText: string,
): ReturnType<AtsAdapter["listJobs"]> extends Promise<infer T> ? T : never {
  switch (provider) {
    case "greenhouse": {
      const parsed = JSON.parse(bodyText) as {
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
      return parsed.jobs.map((job) => ({
        externalId: String(job.id),
        title: job.title,
        url: job.absolute_url,
        location: job.location?.name,
      }));
    }
    case "lever": {
      const parsed = JSON.parse(bodyText) as Array<{
        id: string;
        text: string;
        hostedUrl: string;
        categories?: { location?: string };
      }>;
      if (!Array.isArray(parsed)) {
        throw new Error("Unexpected Lever response shape");
      }
      return parsed.map((job) => ({
        externalId: job.id,
        title: job.text,
        url: job.hostedUrl,
        location: job.categories?.location,
      }));
    }
    case "ashby": {
      const parsed = JSON.parse(bodyText) as {
        jobs?: Array<{ id: string; title: string; jobUrl: string; location?: string }>;
      };
      if (!Array.isArray(parsed.jobs)) {
        throw new Error("Unexpected Ashby response shape");
      }
      return parsed.jobs.map((job) => ({
        externalId: job.id,
        title: job.title,
        url: job.jobUrl,
        location: job.location,
      }));
    }
    default: {
      const _exhaustive: never = provider;
      throw new Error(`Unhandled provider: ${_exhaustive}`);
    }
  }
}
