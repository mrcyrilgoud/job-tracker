import { desc, eq } from "drizzle-orm";

import { getAtsAdapter } from "@/lib/ats";
import { getDb } from "@/lib/db";
import {
  careersPageReviews,
  companies,
  companyWatches,
  type WatchProvider,
} from "@/lib/db/schema";
import { createId, nowIso } from "@/lib/utils";

export function listCompanies() {
  const db = getDb();
  return db.select().from(companies).orderBy(companies.name).all();
}

export function createCompany(input: { name: string; careersUrl?: string | null }) {
  const db = getDb();
  const timestamp = nowIso();
  const existing = db
    .select()
    .from(companies)
    .where(eq(companies.name, input.name.trim()))
    .get();
  if (existing) {
    if (input.careersUrl) {
      db.update(companies)
        .set({ careersUrl: input.careersUrl, updatedAt: timestamp })
        .where(eq(companies.id, existing.id))
        .run();
      return { ...existing, careersUrl: input.careersUrl, updatedAt: timestamp };
    }
    return existing;
  }

  const company = {
    id: createId(),
    name: input.name.trim(),
    careersUrl: input.careersUrl ?? null,
    createdAt: timestamp,
    updatedAt: timestamp,
  };
  db.insert(companies).values(company).run();
  return company;
}

export async function createWatch(input: {
  companyId: string;
  provider: WatchProvider;
  boardSlug: string;
}) {
  const db = getDb();
  const company = db
    .select()
    .from(companies)
    .where(eq(companies.id, input.companyId))
    .get();
  if (!company) {
    throw new Error("Company not found");
  }

  const adapter = getAtsAdapter(input.provider);
  await adapter.validateBoard(input.boardSlug.trim());

  const timestamp = nowIso();
  const watch = {
    id: createId(),
    companyId: input.companyId,
    provider: input.provider,
    boardSlug: input.boardSlug.trim(),
    lastSyncedAt: null,
    consecutiveSyncFailures: 0,
    lastSyncError: null,
    createdAt: timestamp,
    updatedAt: timestamp,
  };
  db.insert(companyWatches).values(watch).run();
  return watch;
}

export function listCompaniesWithWatches() {
  const db = getDb();
  const allCompanies = listCompanies();
  const watches = db.select().from(companyWatches).all();
  const reviews = db
    .select()
    .from(careersPageReviews)
    .where(eq(careersPageReviews.status, "pending"))
    .orderBy(desc(careersPageReviews.createdAt))
    .all();

  return allCompanies.map((company) => ({
    company,
    watches: watches.filter((watch) => watch.companyId === company.id),
    reviews: reviews.filter((review) => review.companyId === company.id),
  }));
}

export function dismissCareersReview(reviewId: string) {
  const db = getDb();
  db.update(careersPageReviews)
    .set({ status: "dismissed" })
    .where(eq(careersPageReviews.id, reviewId))
    .run();
}

export function deleteWatch(watchId: string) {
  const db = getDb();
  db.delete(companyWatches).where(eq(companyWatches.id, watchId)).run();
}
