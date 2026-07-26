import { and, eq } from "drizzle-orm";

import { getAtsAdapter } from "@/lib/ats";
import { getDb } from "@/lib/db";
import {
  companyWatches,
  jobEvents,
  jobs,
  type WatchProvider,
} from "@/lib/db/schema";
import { createId, normalizeCanonicalUrl, nowIso } from "@/lib/utils";

export async function syncCompanyWatch(watchId: string) {
  const db = getDb();
  const watch = db
    .select()
    .from(companyWatches)
    .where(eq(companyWatches.id, watchId))
    .get();

  if (!watch) {
    throw new Error("Watch not found");
  }

  const adapter = getAtsAdapter(watch.provider as WatchProvider);
  const syncedAt = nowIso();

  try {
    const remoteJobs = await adapter.listJobs(watch.boardSlug);
    const remoteIds = new Set(remoteJobs.map((job) => job.externalId));

    const existing = db
      .select()
      .from(jobs)
      .where(
        and(eq(jobs.companyId, watch.companyId), eq(jobs.source, watch.provider)),
      )
      .all();

    let created = 0;
    let reactivated = 0;

    for (const remote of remoteJobs) {
      const canonicalUrl = normalizeCanonicalUrl(remote.url);
      const byExternal = existing.find(
        (job) => job.sourceExternalId === remote.externalId,
      );
      const byUrl = db
        .select()
        .from(jobs)
        .where(eq(jobs.canonicalUrl, canonicalUrl))
        .get();

      if (byExternal) {
        const updates: Partial<typeof jobs.$inferInsert> = {
          title: remote.title,
          url: remote.url,
          location: remote.location ?? null,
          missingFromSyncCount: 0,
          updatedAt: syncedAt,
        };

        if (byExternal.postingState === "inactive") {
          updates.postingState = "active";
          reactivated += 1;
          db.insert(jobEvents)
            .values({
              id: createId(),
              jobId: byExternal.id,
              type: "posting_state_changed",
              note: "Role reappeared in a successful ATS sync",
              occurredAt: syncedAt,
            })
            .run();
        }

        db.update(jobs).set(updates).where(eq(jobs.id, byExternal.id)).run();
        continue;
      }

      if (byUrl) {
        db.update(jobs)
          .set({
            source: watch.provider,
            sourceExternalId: remote.externalId,
            companyId: watch.companyId,
            title: remote.title,
            location: remote.location ?? byUrl.location,
            missingFromSyncCount: 0,
            updatedAt: syncedAt,
          })
          .where(eq(jobs.id, byUrl.id))
          .run();
        continue;
      }

      const jobId = createId();
      db.insert(jobs)
        .values({
          id: jobId,
          companyId: watch.companyId,
          title: remote.title,
          url: remote.url,
          canonicalUrl,
          sourceExternalId: remote.externalId,
          status: "wishlist",
          postingState: "active",
          source: watch.provider,
          location: remote.location,
          isNewFromWatch: true,
          missingFromSyncCount: 0,
          createdAt: syncedAt,
          updatedAt: syncedAt,
        })
        .run();

      db.insert(jobEvents)
        .values({
          id: createId(),
          jobId,
          type: "discovered_from_watch",
          note: `Discovered via ${watch.provider} watch`,
          occurredAt: syncedAt,
        })
        .run();

      created += 1;
    }

    let deactivated = 0;
    for (const local of existing) {
      if (!local.sourceExternalId) {
        continue;
      }
      if (remoteIds.has(local.sourceExternalId)) {
        continue;
      }

      const nextMissing = local.missingFromSyncCount + 1;
      const updates: Partial<typeof jobs.$inferInsert> = {
        missingFromSyncCount: nextMissing,
        updatedAt: syncedAt,
      };

      if (nextMissing >= 2 && local.postingState !== "inactive") {
        updates.postingState = "inactive";
        deactivated += 1;
        db.insert(jobEvents)
          .values({
            id: createId(),
            jobId: local.id,
            type: "posting_state_changed",
            note: "Marked inactive after two successful syncs without this role",
            occurredAt: syncedAt,
          })
          .run();
      }

      db.update(jobs).set(updates).where(eq(jobs.id, local.id)).run();
    }

    db.update(companyWatches)
      .set({
        lastSyncedAt: syncedAt,
        consecutiveSyncFailures: 0,
        lastSyncError: null,
        updatedAt: syncedAt,
      })
      .where(eq(companyWatches.id, watchId))
      .run();

    return {
      ok: true as const,
      created,
      reactivated,
      deactivated,
      totalRemote: remoteJobs.length,
      syncedAt,
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : "Sync failed";
    db.update(companyWatches)
      .set({
        consecutiveSyncFailures: watch.consecutiveSyncFailures + 1,
        lastSyncError: message,
        updatedAt: syncedAt,
      })
      .where(eq(companyWatches.id, watchId))
      .run();

    return {
      ok: false as const,
      error: message,
      syncedAt,
    };
  }
}

export async function syncAllWatches() {
  const db = getDb();
  const watches = db.select().from(companyWatches).all();
  const results = [];
  for (const watch of watches) {
    results.push({ watchId: watch.id, ...(await syncCompanyWatch(watch.id)) });
  }
  return results;
}
