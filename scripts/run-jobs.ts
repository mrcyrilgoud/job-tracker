import { checkAllCareersPages } from "../src/lib/ats/careers-page";
import { syncAllWatches } from "../src/lib/ats/sync";
import { getDb } from "../src/lib/db";
import { jobs } from "../src/lib/db/schema";
import { checkJobPosting } from "../src/lib/jobs/check-active";
import { syncJobsCsvWithDisk } from "../src/lib/jobs/csv-sync";
import { isGmailConnected, pollGmailMatches } from "../src/lib/gmail/client";

async function main() {
  getDb();
  const allJobs = getDb().select().from(jobs).all();
  console.log(`Checking ${allJobs.length} job postings...`);
  for (const job of allJobs) {
    try {
      const result = await checkJobPosting(job.id);
      console.log(`- ${job.title}: ${result.postingState}`);
    } catch (error) {
      console.error(
        `- ${job.title}:`,
        error instanceof Error ? error.message : "check failed",
      );
    }
  }

  console.log("Syncing ATS watches...");
  const watchResults = await syncAllWatches();
  for (const result of watchResults) {
    console.log(`- watch ${result.watchId}:`, result);
  }

  console.log("Checking careers pages...");
  const careers = await checkAllCareersPages();
  for (const result of careers) {
    console.log(`- company ${result.companyId}:`, result);
  }

  if (await isGmailConnected()) {
    console.log("Polling Gmail...");
    const gmail = await pollGmailMatches();
    console.log("- gmail:", gmail);
  } else {
    console.log("Gmail not connected; skipping.");
  }

  console.log("Syncing jobs CSV...");
  const csv = await syncJobsCsvWithDisk();
  console.log("- csv:", {
    imported: csv.imported?.summary ?? null,
    exported: csv.exported,
  });
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
