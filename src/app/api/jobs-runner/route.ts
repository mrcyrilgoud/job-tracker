import { NextResponse } from "next/server";

import { checkAllCareersPages } from "@/lib/ats/careers-page";
import { syncAllWatches } from "@/lib/ats/sync";
import { getDb } from "@/lib/db";
import { jobs } from "@/lib/db/schema";
import { isGmailConnected, pollGmailMatches } from "@/lib/gmail/client";
import { checkJobPosting } from "@/lib/jobs/check-active";
import { syncJobsCsvWithDisk } from "@/lib/jobs/csv-sync";

export const runtime = "nodejs";

export async function POST() {
  const db = getDb();
  const allJobs = db.select().from(jobs).all();
  const checkResults = [];
  for (const job of allJobs) {
    try {
      checkResults.push({ jobId: job.id, ...(await checkJobPosting(job.id)) });
    } catch (error) {
      checkResults.push({
        jobId: job.id,
        error: error instanceof Error ? error.message : "check failed",
      });
    }
  }

  const watchResults = await syncAllWatches();
  const careersResults = await checkAllCareersPages();

  let gmailResult = null;
  if (await isGmailConnected()) {
    try {
      gmailResult = await pollGmailMatches();
    } catch (error) {
      gmailResult = {
        error: error instanceof Error ? error.message : "gmail poll failed",
      };
    }
  }

  let csvResult = null;
  try {
    csvResult = await syncJobsCsvWithDisk();
  } catch (error) {
    csvResult = {
      error: error instanceof Error ? error.message : "csv sync failed",
    };
  }

  return NextResponse.json({
    checks: checkResults,
    watches: watchResults,
    careers: careersResults,
    gmail: gmailResult,
    csv: csvResult,
  });
}
