import { describe, expect, it } from "vitest";

import { getDb } from "@/lib/db";
import {
  companies,
  companyWatches,
  jobEvents,
  jobs,
} from "@/lib/db/schema";
import { createId, nowIso } from "@/lib/utils";
import { eq } from "drizzle-orm";

/**
 * Exercises the two-successful-sync deactivation rule without network calls
 * by directly applying the same counter logic used in sync.
 */
function applyMissingSync(jobId: string) {
  const db = getDb();
  const job = db.select().from(jobs).where(eq(jobs.id, jobId)).get();
  if (!job) throw new Error("missing job");
  const syncedAt = nowIso();
  const nextMissing = job.missingFromSyncCount + 1;
  const updates: Partial<typeof jobs.$inferInsert> = {
    missingFromSyncCount: nextMissing,
    updatedAt: syncedAt,
  };
  if (nextMissing >= 2 && job.postingState !== "inactive") {
    updates.postingState = "inactive";
    db.insert(jobEvents)
      .values({
        id: createId(),
        jobId,
        type: "posting_state_changed",
        note: "Marked inactive after two successful syncs without this role",
        occurredAt: syncedAt,
      })
      .run();
  }
  db.update(jobs).set(updates).where(eq(jobs.id, jobId)).run();
  return db.select().from(jobs).where(eq(jobs.id, jobId)).get();
}

describe("sync deactivation rules", () => {
  it("only marks inactive after two successful missing syncs", () => {
    const db = getDb();
    const timestamp = nowIso();
    const companyId = createId();
    const jobId = createId();
    const externalId = `ext-${jobId}`;

    db.insert(companies)
      .values({
        id: companyId,
        name: `SyncCo ${Date.now()}`,
        careersUrl: null,
        createdAt: timestamp,
        updatedAt: timestamp,
      })
      .run();

    db.insert(companyWatches)
      .values({
        id: createId(),
        companyId,
        provider: "greenhouse",
        boardSlug: "example",
        lastSyncedAt: null,
        consecutiveSyncFailures: 0,
        lastSyncError: null,
        createdAt: timestamp,
        updatedAt: timestamp,
      })
      .run();

    db.insert(jobs)
      .values({
        id: jobId,
        companyId,
        title: "Temporary Role",
        url: `https://example.com/jobs/temp-${jobId}`,
        canonicalUrl: `https://example.com/jobs/temp-${jobId}`,
        sourceExternalId: externalId,
        status: "wishlist",
        postingState: "active",
        source: "greenhouse",
        missingFromSyncCount: 0,
        isNewFromWatch: false,
        createdAt: timestamp,
        updatedAt: timestamp,
      })
      .run();

    const afterOne = applyMissingSync(jobId);
    expect(afterOne?.missingFromSyncCount).toBe(1);
    expect(afterOne?.postingState).toBe("active");

    const afterTwo = applyMissingSync(jobId);
    expect(afterTwo?.missingFromSyncCount).toBe(2);
    expect(afterTwo?.postingState).toBe("inactive");
  });
});
