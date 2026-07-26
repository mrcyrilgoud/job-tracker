import { eq } from "drizzle-orm";

import { getDb } from "@/lib/db";
import { jobs, type PostingState } from "@/lib/db/schema";
import { createId, nowIso } from "@/lib/utils";
import { looksLikeClosedPosting, safeFetch } from "@/lib/jobs/safe-fetch";
import { jobEvents } from "@/lib/db/schema";

export type CheckPostingResult = {
  postingState: PostingState;
  lastCheckResult: string;
  lastCheckedAt: string;
};

export async function checkJobPosting(jobId: string): Promise<CheckPostingResult> {
  const db = getDb();
  const job = db.select().from(jobs).where(eq(jobs.id, jobId)).get();
  if (!job) {
    throw new Error("Job not found");
  }

  const result = await safeFetch(job.url, { method: "GET" });
  const checkedAt = nowIso();
  let postingState: PostingState = "unknown";
  let lastCheckResult = "";

  if (result.error) {
    postingState = "unknown";
    lastCheckResult = `error: ${result.error}`;
  } else if (looksLikeClosedPosting(result.bodyText, result.status)) {
    postingState = "inactive";
    lastCheckResult = `inactive: HTTP ${result.status}`;
  } else if (result.ok) {
    postingState = "active";
    lastCheckResult = `active: HTTP ${result.status}`;
  } else if (result.status >= 500) {
    postingState = "unknown";
    lastCheckResult = `unknown: HTTP ${result.status}`;
  } else {
    postingState = "unknown";
    lastCheckResult = `unknown: HTTP ${result.status}`;
  }

  const previousState = job.postingState;
  db.update(jobs)
    .set({
      postingState,
      lastCheckedAt: checkedAt,
      lastCheckResult,
      updatedAt: checkedAt,
    })
    .where(eq(jobs.id, jobId))
    .run();

  if (previousState !== postingState) {
    db.insert(jobEvents)
      .values({
        id: createId(),
        jobId,
        type: "posting_state_changed",
        note: `Posting state changed from ${previousState} to ${postingState} (${lastCheckResult})`,
        occurredAt: checkedAt,
      })
      .run();
  }

  return { postingState, lastCheckResult, lastCheckedAt: checkedAt };
}
