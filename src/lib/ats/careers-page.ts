import { createHash } from "node:crypto";

import { desc, eq } from "drizzle-orm";

import { getDb } from "@/lib/db";
import {
  careersPageReviews,
  careersPageSnapshots,
  companies,
} from "@/lib/db/schema";
import { safeFetch } from "@/lib/jobs/safe-fetch";
import { createId, nowIso } from "@/lib/utils";

function normalizeCareersText(html: string) {
  return html
    .replace(/<script[\s\S]*?<\/script>/gi, " ")
    .replace(/<style[\s\S]*?<\/style>/gi, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
}

export async function checkCareersPage(companyId: string) {
  const db = getDb();
  const company = db.select().from(companies).where(eq(companies.id, companyId)).get();
  if (!company?.careersUrl) {
    return { changed: false as const, reason: "No careers URL configured" };
  }

  const result = await safeFetch(company.careersUrl);
  if (!result.ok) {
    return {
      changed: false as const,
      reason: result.error ?? `HTTP ${result.status}`,
    };
  }

  const normalizedText = normalizeCareersText(result.bodyText);
  const contentHash = createHash("sha256").update(normalizedText).digest("hex");
  const previous = db
    .select()
    .from(careersPageSnapshots)
    .where(eq(careersPageSnapshots.companyId, companyId))
    .orderBy(desc(careersPageSnapshots.capturedAt))
    .get();

  const capturedAt = nowIso();
  db.insert(careersPageSnapshots)
    .values({
      id: createId(),
      companyId,
      contentHash,
      normalizedText: normalizedText.slice(0, 20_000),
      capturedAt,
    })
    .run();

  if (!previous) {
    return { changed: false as const, reason: "Initial snapshot captured" };
  }

  if (previous.contentHash === contentHash) {
    return { changed: false as const, reason: "No change detected" };
  }

  db.insert(careersPageReviews)
    .values({
      id: createId(),
      companyId,
      previousHash: previous.contentHash,
      currentHash: contentHash,
      summary: `Careers page content changed for ${company.name}. Review manually; no jobs were auto-created.`,
      status: "pending",
      createdAt: capturedAt,
    })
    .run();

  return { changed: true as const, currentHash: contentHash };
}

export async function checkAllCareersPages() {
  const db = getDb();
  const allCompanies = db.select().from(companies).all();
  const results = [];
  for (const company of allCompanies) {
    if (!company.careersUrl) continue;
    results.push({
      companyId: company.id,
      ...(await checkCareersPage(company.id)),
    });
  }
  return results;
}
