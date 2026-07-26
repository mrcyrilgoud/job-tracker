import { NextResponse } from "next/server";
import { desc, eq } from "drizzle-orm";

import { getDb } from "@/lib/db";
import { documents, jobDocuments, jobs, companies } from "@/lib/db/schema";
import { attachDocumentToJob, importDocument } from "@/lib/documents/store";
import type { DocumentKind } from "@/lib/db/schema";

export const runtime = "nodejs";

export async function GET() {
  const db = getDb();
  const allDocs = db.select().from(documents).orderBy(desc(documents.importedAt)).all();
  const attachments = db
    .select({
      attachment: jobDocuments,
      jobTitle: jobs.title,
      companyName: companies.name,
    })
    .from(jobDocuments)
    .innerJoin(jobs, eq(jobDocuments.jobId, jobs.id))
    .innerJoin(companies, eq(jobs.companyId, companies.id))
    .all();

  return NextResponse.json({
    documents: allDocs.map((doc) => ({
      ...doc,
      usedBy: attachments
        .filter((row) => row.attachment.documentId === doc.id)
        .map((row) => `${row.companyName}`),
      kinds: [
        ...new Set(
          attachments
            .filter((row) => row.attachment.documentId === doc.id)
            .map((row) => row.attachment.kind),
        ),
      ],
    })),
  });
}

export async function POST(request: Request) {
  try {
    const form = await request.formData();
    const file = form.get("file");
    if (!(file instanceof File)) {
      return NextResponse.json({ error: "file is required" }, { status: 400 });
    }

    const buffer = Buffer.from(await file.arrayBuffer());
    const document = importDocument({
      originalFilename: file.name,
      mimeType: file.type || "application/octet-stream",
      buffer,
    });

    const jobId = form.get("jobId");
    const kind = form.get("kind");
    if (typeof jobId === "string" && typeof kind === "string") {
      const attachment = attachDocumentToJob({
        jobId,
        documentId: document.id,
        kind: kind as DocumentKind,
      });
      return NextResponse.json({ document, attachment }, { status: 201 });
    }

    return NextResponse.json({ document }, { status: 201 });
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Import failed" },
      { status: 400 },
    );
  }
}
