/**
 * Dev-only fixture backend for the design preview entry.
 *
 * Mirrors the shapes the Rust commands return (see `src-tauri/src/commands`)
 * closely enough that the real components render as they do in the app. State
 * is in-memory, so mutations (sync, approve, dismiss) behave plausibly during a
 * preview session and reset on reload.
 */
import type {
  CareersPageReview,
  Company,
  CompanyRow,
  CompanyWatch,
  Job,
  JobListItem,
  JobStatus,
  WatchProvider,
} from "@/lib/schema";

const now = Date.now();
const ago = (ms: number) => new Date(now - ms).toISOString();
const minutes = 60_000;
const hours = 60 * minutes;
const days = 24 * hours;

function company(id: string, name: string, careersUrl: string | null): Company {
  return {
    id,
    name,
    careersUrl,
    createdAt: ago(30 * days),
    updatedAt: ago(2 * days),
  };
}

function watch(
  id: string,
  companyId: string,
  provider: WatchProvider,
  boardSlug: string,
  overrides: Partial<CompanyWatch> = {},
): CompanyWatch {
  return {
    id,
    companyId,
    provider,
    boardSlug,
    lastSyncedAt: ago(2 * hours),
    consecutiveSyncFailures: 0,
    lastSyncError: null,
    createdAt: ago(20 * days),
    updatedAt: ago(2 * hours),
    ...overrides,
  };
}

let jobSeq = 0;
function job(
  companyId: string,
  title: string,
  location: string | null,
  opts: { isNewFromWatch?: boolean; status?: JobStatus; source?: Job["source"] } = {},
): Job {
  jobSeq += 1;
  const id = `job-${jobSeq}`;
  const url = `https://boards.greenhouse.io/example/jobs/${4000000 + jobSeq}`;
  return {
    id,
    companyId,
    title,
    url,
    canonicalUrl: url,
    sourceExternalId: `ext-${jobSeq}`,
    status: opts.status ?? "wishlist",
    appliedAt: null,
    postingState: "active",
    lastCheckedAt: ago(3 * hours),
    lastCheckResult: null,
    source: opts.source ?? "greenhouse",
    notes: null,
    location,
    isNewFromWatch: opts.isNewFromWatch ?? false,
    missingFromSyncCount: 0,
    createdAt: ago(5 * days),
    updatedAt: ago(1 * hours),
  };
}

export function createFixtureBackend() {
  const companies: Company[] = [
    company("c-anthropic", "Anthropic", "https://www.anthropic.com/careers"),
    company("c-stripe", "Stripe", "https://stripe.com/jobs"),
    company("c-figma", "Figma", "https://www.figma.com/careers/"),
    company("c-linear", "Linear", null),
    company("c-notion", "Notion", null),
  ];

  const watches: CompanyWatch[] = [
    watch("w-1", "c-anthropic", "greenhouse", "anthropic"),
    watch("w-2", "c-stripe", "greenhouse", "stripe", { lastSyncedAt: ago(6 * hours) }),
    watch("w-3", "c-stripe", "lever", "stripe", {
      consecutiveSyncFailures: 3,
      lastSyncError:
        "error sending request for url (https://api.lever.co/v0/postings/stripe?mode=json): operation timed out",
      lastSyncedAt: ago(4 * days),
    }),
    watch("w-4", "c-linear", "ashby", "linear", {
      consecutiveSyncFailures: 1,
      lastSyncError: "unexpected status 404 Not Found",
      lastSyncedAt: null,
    }),
  ];

  const reviews: CareersPageReview[] = [
    {
      id: "r-1",
      companyId: "c-figma",
      previousHash: "a1b2c3",
      currentHash: "d4e5f6",
      summary: "Careers page content changed since the last check",
      status: "pending",
      createdAt: ago(18 * hours),
    },
  ];

  const jobs: Job[] = [
    // Tracked roles already on the user's board.
    job("c-anthropic", "Senior Product Engineer", "San Francisco, CA", {
      status: "applied",
    }),
    job("c-stripe", "Staff Frontend Engineer", "Remote (US)", { status: "interviewing" }),

    // Discovered by a watch, awaiting triage. These are the roles the user
    // currently has no way to browse from the Companies tab.
    job("c-anthropic", "Product Engineer, Growth", "San Francisco, CA", {
      isNewFromWatch: true,
    }),
    job("c-anthropic", "Software Engineer, Inference", "Seattle, WA", {
      isNewFromWatch: true,
    }),
    job("c-anthropic", "Engineering Manager, Product", "San Francisco, CA", {
      isNewFromWatch: true,
    }),
    job("c-anthropic", "Design Engineer", "Remote (US)", { isNewFromWatch: true }),
    job("c-anthropic", "Technical Program Manager, Safety", "New York, NY", {
      isNewFromWatch: true,
    }),
    job("c-anthropic", "Data Scientist, Product Analytics", "San Francisco, CA", {
      isNewFromWatch: true,
    }),
    job("c-anthropic", "Security Engineer, Corporate", "Remote (US)", {
      isNewFromWatch: true,
    }),
    job("c-stripe", "Frontend Engineer, Checkout", "Seattle, WA", {
      isNewFromWatch: true,
    }),
    job("c-stripe", "Product Manager, Billing", "New York, NY", { isNewFromWatch: true }),
    job("c-stripe", "Staff Engineer, Payments Infrastructure", "Remote (US)", {
      isNewFromWatch: true,
    }),
  ];

  function companyRows(): CompanyRow[] {
    return companies.map((c) => ({
      company: c,
      watches: watches.filter((w) => w.companyId === c.id),
      reviews: reviews.filter((r) => r.companyId === c.id && r.status === "pending"),
    }));
  }

  function jobListItems(filters?: Record<string, unknown> | null): JobListItem[] {
    const companyId = filters?.companyId as string | undefined;
    const newFromWatch = filters?.newFromWatch === true;
    const search = (filters?.search as string | undefined)?.toLowerCase();
    return jobs
      .filter((j) => (companyId ? j.companyId === companyId : true))
      .filter((j) => j.isNewFromWatch === newFromWatch)
      .filter((j) => (search ? j.title.toLowerCase().includes(search) : true))
      .map((j) => ({
        job: j,
        companyName: companies.find((c) => c.id === j.companyId)?.name ?? "Unknown",
      }));
  }

  const handlers: Record<string, (args?: Record<string, unknown>) => unknown> = {
    list_companies: () => ({ companies: companyRows() }),

    list_jobs_cmd: (args) => {
      const filters = (args?.filters ?? null) as Record<string, unknown> | null;
      return {
        jobs: jobListItems(filters),
        counts: { wishlist: 6, applied: 1, interviewing: 1, offer: 0 },
        weeklyActivity: {
          total: 4,
          days: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map((label, i) => ({
            key: `d${i}`,
            label,
            count: i % 3,
            isToday: i === 6,
          })),
        },
        dataDir: "/Users/preview/Library/Application Support/job-tracker",
      };
    },

    create_company: (args) => {
      const input = (args?.input ?? {}) as { name: string; careersUrl: string | null };
      const existing = companies.find((c) => c.name === input.name);
      if (existing) {
        existing.careersUrl = input.careersUrl ?? existing.careersUrl;
        return { company: existing };
      }
      const created = company(`c-${Date.now()}`, input.name, input.careersUrl ?? null);
      companies.push(created);
      return { company: created };
    },

    create_watch: (args) => {
      const input = (args?.input ?? {}) as {
        companyId: string;
        provider: WatchProvider;
        boardSlug: string;
      };
      const created = watch(
        `w-${Date.now()}`,
        input.companyId,
        input.provider,
        input.boardSlug,
        { lastSyncedAt: null },
      );
      watches.push(created);
      return { watch: created };
    },

    delete_watch: (args) => {
      const index = watches.findIndex((w) => w.id === args?.watchId);
      if (index >= 0) watches.splice(index, 1);
      return { ok: true };
    },

    sync_watch: (args) => {
      const found = watches.find((w) => w.id === args?.watchId);
      if (found) {
        found.lastSyncedAt = new Date().toISOString();
        found.consecutiveSyncFailures = 0;
        found.lastSyncError = null;
      }
      return { ok: true, created: 2 };
    },

    check_careers: () => ({ ok: true, changed: false }),

    dismiss_review: (args) => {
      const found = reviews.find((r) => r.id === args?.reviewId);
      if (found) found.status = "dismissed";
      return { ok: true };
    },

    approve_watch_job_cmd: (args) => {
      const found = jobs.find((j) => j.id === args?.jobId);
      if (found) found.isNewFromWatch = false;
      return { job: found };
    },

    dismiss_watch_job_cmd: (args) => {
      const index = jobs.findIndex((j) => j.id === args?.jobId);
      if (index >= 0) jobs.splice(index, 1);
      return { job: null };
    },

    preview_job_url: (args) => {
      const url = String(args?.url ?? "");
      return {
        title: "Product Engineer, Growth",
        companyName: "Anthropic",
        board: url.includes("greenhouse")
          ? {
              provider: "greenhouse",
              boardSlug: "anthropic",
              boardUrl: "https://boards.greenhouse.io/anthropic",
              postingId: "4012345",
            }
          : null,
        careersUrl: url.includes("greenhouse") ? null : "https://example.com/careers",
      };
    },

    run_jobs_cycle_cmd: () => ({ ok: true }),
    check_all_postings_cmd: () => ({ ok: true }),
    list_documents: () => ({ documents: [] }),
    gmail_status: () => ({
      connected: false,
      configured: false,
      redirectUri: "http://127.0.0.1:8765/callback",
      pending: [],
    }),
  };

  async function invoke(cmd: string, args?: Record<string, unknown>) {
    // Event plugin calls (used by RunJobsButton) resolve to a no-op listener id.
    if (cmd.startsWith("plugin:")) return 0;

    const handler = handlers[cmd];
    if (!handler) {
      throw new Error(`[design-preview] no fixture for command "${cmd}"`);
    }
    // A touch of latency so loading states are observable.
    await new Promise((resolve) => setTimeout(resolve, 60));
    return handler(args);
  }

  return { invoke };
}
