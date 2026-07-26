import { NextResponse } from "next/server";

import { attachDocumentToJob } from "@/lib/documents/store";
import type { DocumentKind } from "@/lib/db/schema";
import { getDb } from "@/lib/db";
import { jobDocuments, jobEvents } from "@/lib/db/schema";
import { createId, nowIso } from "@/lib/utils";
import { eq } from "drizzle-orm";

export const runtime = "nodejs";

export async function POST(request: Request) {
  try {
    const body = (await request.json()) as {
      jobId?: string;
      documentId?: string;
      kind?: DocumentKind;
    };

    if (!body.jobId || !body.documentId || !body.kind) {
      return NextResponse.json(
        { error: "jobId, documentId, and kind are required" },
        { status: 400 },
      );
    }

    const attachment = attachDocumentToJob({
      jobId: body.jobId,
      documentId: body.documentId,
      kind: body.kind,
    });

    const db = getDb();
    db.insert(jobEvents)
      .values({
        id: createId(),
        jobId: body.jobId,
        type: "document_attached",
        note: `Attached ${body.kind.replace("_", " ")}`,
        occurredAt: nowIso(),
      })
      .run();

    return NextResponse.json({ attachment }, { status: 201 });
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Attach failed" },
      { status: 400 },
    );
  }
}

export async function DELETE(request: Request) {
  try {
    const body = (await request.json()) as { attachmentId?: string };
    if (!body.attachmentId) {
      return NextResponse.json({ error: "attachmentId is required" }, { status: 400 });
    }
    const db = getDb();
    db.delete(jobDocuments).where(eq(jobDocuments.id, body.attachmentId)).run();
    return NextResponse.json({ ok: true });
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Detach failed" },
      { status: 400 },
    );
  }
}
