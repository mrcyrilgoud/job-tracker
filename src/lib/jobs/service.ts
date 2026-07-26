import { and, desc, eq, gte, like } from "drizzle-orm";

import { getDb } from "@/lib/db";
import {
  companies,
  documents,
  jobDocuments,
  jobEvents,
  jobs,
  type JobSource,
  type JobStatus,
  type PostingState,
} from "@/lib/db/schema";
import {
  createId,
  guessTitleFromUrl,
  normalizeCanonicalUrl,
  nowIso,
} from "@/lib/utils";
import { extractHtmlTitle, safeFetch } from "@/lib/jobs/safe-fetch";

export async function createJobFromUrl(input: {
  url: string;
  title?: string;
  companyName?: string;
  status?: JobStatus;
  appliedAt?: string | null;
  notes?: string | null;
  location?: string | null;
}) {
  const db = getDb();
  const canonicalUrl = normalizeCanonicalUrl(input.url);
  const existing = db
    .select()
    .from(jobs)
    .where(eq(jobs.canonicalUrl, canonicalUrl))
    .get();
  if (existing) {
    throw new Error("A job with this URL is already tracked");
  }

  let title = input.title?.trim() || guessTitleFromUrl(input.url);
  if (!input.title) {
    const fetched = await safeFetch(input.url);
    if (fetched.ok) {
      title = extractHtmlTitle(fetched.bodyText) ?? title;
    }
  }

  const companyName = input.companyName?.trim() || "Unknown company";
  let company = db.select().from(companies).where(eq(companies.name, companyName)).get();
  const timestamp = nowIso();
  if (!company) {
    company = {
      id: createId(),
      name: companyName,
      careersUrl: null,
      createdAt: timestamp,
      updatedAt: timestamp,
    };
    db.insert(companies).values(company).run();
  }

  const status = input.status ?? "wishlist";
  const jobId = createId();
  const job = {
    id: jobId,
    companyId: company.id,
    title,
    url: input.url,
    canonicalUrl,
    sourceExternalId: null,
    status,
    appliedAt: input.appliedAt ?? (status === "applied" ? timestamp : null),
    postingState: "unknown" as PostingState,
    lastCheckedAt: null,
    lastCheckResult: null,
    source: "manual" as JobSource,
    notes: input.notes ?? null,
    location: input.location ?? null,
    isNewFromWatch: false,
    missingFromSyncCount: 0,
    createdAt: timestamp,
    updatedAt: timestamp,
  };

  db.insert(jobs).values(job).run();
  db.insert(jobEvents)
    .values({
      id: createId(),
      jobId,
      type: "created",
      note: `Added from URL with status ${status}`,
      occurredAt: timestamp,
    })
    .run();

  return { job, company };
}

export function listJobs(filters: {
  status?: string;
  companyId?: string;
  postingState?: string;
  search?: string;
  newFromWatch?: boolean;
} = {}) {
  const db = getDb();
  const conditions = [];

  if (filters.status) {
    conditions.push(eq(jobs.status, filters.status as JobStatus));
  }
  if (filters.companyId) {
    conditions.push(eq(jobs.companyId, filters.companyId));
  }
  if (filters.postingState) {
    conditions.push(eq(jobs.postingState, filters.postingState as PostingState));
  }
  if (filters.newFromWatch) {
    conditions.push(eq(jobs.isNewFromWatch, true));
  }
  if (filters.search) {
    conditions.push(like(jobs.title, `%${filters.search}%`));
  }

  const rows = db
    .select({
      job: jobs,
      companyName: companies.name,
    })
    .from(jobs)
    .innerJoin(companies, eq(jobs.companyId, companies.id))
    .where(conditions.length ? and(...conditions) : undefined)
    .orderBy(desc(jobs.updatedAt))
    .all();

  return rows;
}

export function getJobDetail(jobId: string) {
  const db = getDb();
  const row = db
    .select({
      job: jobs,
      company: companies,
    })
    .from(jobs)
    .innerJoin(companies, eq(jobs.companyId, companies.id))
    .where(eq(jobs.id, jobId))
    .get();

  if (!row) {
    return null;
  }

  const events = db
    .select()
    .from(jobEvents)
    .where(eq(jobEvents.jobId, jobId))
    .orderBy(desc(jobEvents.occurredAt))
    .all();

  const attached = db
    .select({
      attachment: jobDocuments,
      document: documents,
    })
    .from(jobDocuments)
    .innerJoin(documents, eq(jobDocuments.documentId, documents.id))
    .where(eq(jobDocuments.jobId, jobId))
    .orderBy(desc(jobDocuments.usedAt))
    .all();

  return { ...row, events, attached };
}

export function updateJob(
  jobId: string,
  updates: {
    title?: string;
    companyName?: string;
    status?: JobStatus;
    appliedAt?: string | null;
    notes?: string | null;
    location?: string | null;
    url?: string;
    isNewFromWatch?: boolean;
  },
) {
  const db = getDb();
  const existing = db.select().from(jobs).where(eq(jobs.id, jobId)).get();
  if (!existing) {
    throw new Error("Job not found");
  }

  const timestamp = nowIso();
  let companyId = existing.companyId;
  let nextUrl = existing.url;
  let nextCanonicalUrl = existing.canonicalUrl;

  if (updates.url !== undefined) {
    const trimmedUrl = updates.url.trim();
    if (!trimmedUrl) {
      throw new Error("URL cannot be empty");
    }
    nextUrl = trimmedUrl;
    nextCanonicalUrl = normalizeCanonicalUrl(trimmedUrl);
    const duplicate = db
      .select()
      .from(jobs)
      .where(eq(jobs.canonicalUrl, nextCanonicalUrl))
      .get();
    if (duplicate && duplicate.id !== jobId) {
      throw new Error("A job with this URL is already tracked");
    }
  }

  if (updates.companyName) {
    let company = db
      .select()
      .from(companies)
      .where(eq(companies.name, updates.companyName.trim()))
      .get();
    if (!company) {
      company = {
        id: createId(),
        name: updates.companyName.trim(),
        careersUrl: null,
        createdAt: timestamp,
        updatedAt: timestamp,
      };
      db.insert(companies).values(company).run();
    }
    companyId = company.id;
  }

  const nextStatus = updates.status ?? existing.status;
  const nextAppliedAt =
    updates.appliedAt !== undefined
      ? updates.appliedAt
      : nextStatus === "applied" && !existing.appliedAt
        ? timestamp
        : existing.appliedAt;

  db.update(jobs)
    .set({
      title: updates.title ?? existing.title,
      companyId,
      url: nextUrl,
      canonicalUrl: nextCanonicalUrl,
      status: nextStatus,
      appliedAt: nextAppliedAt,
      notes: updates.notes !== undefined ? updates.notes : existing.notes,
      location:
        updates.location !== undefined ? updates.location : existing.location,
      isNewFromWatch:
        updates.isNewFromWatch !== undefined
          ? updates.isNewFromWatch
          : existing.isNewFromWatch,
      updatedAt: timestamp,
    })
    .where(eq(jobs.id, jobId))
    .run();

  if (updates.status && updates.status !== existing.status) {
    db.insert(jobEvents)
      .values({
        id: createId(),
        jobId,
        type: "status_changed",
        note: `Status changed from ${existing.status} to ${updates.status}`,
        occurredAt: timestamp,
      })
      .run();
  }

  return getJobDetail(jobId);
}

export function addJobEvent(
  jobId: string,
  type: string,
  note: string | null = null,
) {
  const db = getDb();
  const existing = db.select().from(jobs).where(eq(jobs.id, jobId)).get();
  if (!existing) {
    throw new Error("Job not found");
  }

  const event = {
    id: createId(),
    jobId,
    type,
    note,
    occurredAt: nowIso(),
  };
  db.insert(jobEvents).values(event).run();
  return event;
}

export function getPipelineCounts() {
  const db = getDb();
  const all = db.select().from(jobs).all();
  const counts: Record<string, number> = {
    all: all.length,
    wishlist: 0,
    applied: 0,
    interviewing: 0,
    offer: 0,
    rejected: 0,
    withdrawn: 0,
    closed: 0,
  };

  for (const job of all) {
    counts[job.status] = (counts[job.status] ?? 0) + 1;
  }

  return counts;
}

export type WeeklyActivity = {
  total: number;
  days: Array<{ key: string; label: string; count: number; isToday: boolean }>;
};

const WEEKDAY_INITIALS = ["S", "M", "T", "W", "T", "F", "S"];

function localDayKey(date: Date): string {
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${year}-${month}-${day}`;
}

/** Per-day event counts for the trailing 7 days (index 0 = 6 days ago, 6 = today). */
export function getWeeklyActivity(): WeeklyActivity {
  const db = getDb();

  const start = new Date();
  start.setHours(0, 0, 0, 0);
  start.setDate(start.getDate() - 6);

  const todayKey = localDayKey(new Date());
  const buckets = new Map<string, number>();
  const days = Array.from({ length: 7 }, (_, i) => {
    const date = new Date(start);
    date.setDate(start.getDate() + i);
    const key = localDayKey(date);
    buckets.set(key, 0);
    return { key, label: WEEKDAY_INITIALS[date.getDay()], isToday: key === todayKey };
  });

  const events = db
    .select({ occurredAt: jobEvents.occurredAt })
    .from(jobEvents)
    .where(gte(jobEvents.occurredAt, start.toISOString()))
    .all();

  let total = 0;
  for (const event of events) {
    const key = localDayKey(new Date(event.occurredAt));
    if (buckets.has(key)) {
      buckets.set(key, (buckets.get(key) ?? 0) + 1);
      total += 1;
    }
  }

  return {
    total,
    days: days.map((day) => ({ ...day, count: buckets.get(day.key) ?? 0 })),
  };
}
