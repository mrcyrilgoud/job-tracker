import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

import { eq } from "drizzle-orm";

import { getDb, getDocumentsDir } from "@/lib/db";
import { documents, jobDocuments, type DocumentKind } from "@/lib/db/schema";
import { createId, extensionForMime, nowIso } from "@/lib/utils";

const MAX_DOCUMENT_BYTES = 10 * 1024 * 1024;

const ALLOWED_MIME = new Map<string, string[]>([
  ["application/pdf", [".pdf"]],
  [
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    [".docx"],
  ],
  ["text/plain", [".txt"]],
]);

export function validateDocumentUpload(filename: string, mimeType: string, size: number) {
  if (size > MAX_DOCUMENT_BYTES) {
    throw new Error("File exceeds the 10MB size limit");
  }

  const allowedExts = ALLOWED_MIME.get(mimeType);
  if (!allowedExts) {
    throw new Error("Only PDF, DOCX, and plain text files are allowed");
  }

  const ext = path.extname(filename).toLowerCase();
  if (!allowedExts.includes(ext)) {
    throw new Error(`File extension ${ext || "(none)"} does not match MIME type ${mimeType}`);
  }
}

export function importDocument(input: {
  originalFilename: string;
  mimeType: string;
  buffer: Buffer;
}) {
  validateDocumentUpload(input.originalFilename, input.mimeType, input.buffer.byteLength);

  const checksum = createHash("sha256").update(input.buffer).digest("hex");
  const db = getDb();
  const existing = db
    .select()
    .from(documents)
    .where(eq(documents.checksum, checksum))
    .get();

  if (existing) {
    return existing;
  }

  const id = createId();
  const ext = extensionForMime(input.mimeType, input.originalFilename);
  const storedFilename = `${id}${ext}`;
  const dest = path.join(getDocumentsDir(), storedFilename);
  fs.writeFileSync(dest, input.buffer);

  const importedAt = nowIso();
  const row = {
    id,
    originalFilename: input.originalFilename,
    storedFilename,
    mimeType: input.mimeType,
    checksum,
    sizeBytes: input.buffer.byteLength,
    importedAt,
  };

  db.insert(documents).values(row).run();
  return row;
}

export function attachDocumentToJob(input: {
  jobId: string;
  documentId: string;
  kind: DocumentKind;
}) {
  const db = getDb();
  const document = db
    .select()
    .from(documents)
    .where(eq(documents.id, input.documentId))
    .get();
  if (!document) {
    throw new Error("Document not found");
  }

  const usedAt = nowIso();
  const row = {
    id: createId(),
    jobId: input.jobId,
    documentId: input.documentId,
    kind: input.kind,
    usedAt,
  };
  db.insert(jobDocuments).values(row).run();
  return row;
}

export function getDocumentFilePath(documentId: string) {
  const db = getDb();
  const document = db.select().from(documents).where(eq(documents.id, documentId)).get();
  if (!document) {
    throw new Error("Document not found");
  }

  const filePath = path.join(getDocumentsDir(), document.storedFilename);
  if (!fs.existsSync(filePath)) {
    throw new Error("Document file is missing from disk");
  }

  return { document, filePath };
}
