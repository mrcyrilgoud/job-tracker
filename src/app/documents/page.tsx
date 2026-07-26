import { desc, eq } from "drizzle-orm";

import { DocumentsClient } from "@/components/documents-client";
import { getDb } from "@/lib/db";
import { companies, documents, jobDocuments, jobs } from "@/lib/db/schema";
import { formatLabel } from "@/lib/utils";

export const dynamic = "force-dynamic";

export default async function DocumentsPage() {
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

  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">Documents</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Import a resume or cover letter once, then reuse it across applications.
        </p>
      </div>

      <DocumentsClient />

      {allDocs.length === 0 ? (
        <div className="card px-6 py-16 text-center">
          <p className="font-display text-lg">No documents yet</p>
          <p className="mt-1 text-sm text-[var(--muted)]">
            Import a PDF, DOCX, or TXT to get started.
          </p>
        </div>
      ) : (
        <ul className="space-y-2">
          {allDocs.map((doc) => {
            const linked = attachments.filter(
              (row) => row.attachment.documentId === doc.id,
            );
            const kinds = [...new Set(linked.map((row) => row.attachment.kind))];
            const usedBy = [...new Set(linked.map((row) => row.companyName))];
            return (
              <li key={doc.id}>
                <div className="card flex items-center justify-between gap-4 p-4">
                  <div className="min-w-0">
                    <p className="truncate font-medium">{doc.originalFilename}</p>
                    <p className="text-sm text-[var(--muted)]">
                      {kinds.length
                        ? kinds.map((kind) => formatLabel(kind)).join(", ")
                        : "In your library"}
                      {usedBy.length ? ` · Used at ${usedBy.join(", ")}` : ""}
                    </p>
                  </div>
                  <a
                    href={`/api/documents/${doc.id}`}
                    className="shrink-0 text-sm font-medium text-[var(--accent)] hover:underline"
                  >
                    Open
                  </a>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
