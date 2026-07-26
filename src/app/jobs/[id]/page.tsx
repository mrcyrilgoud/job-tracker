import { formatDistanceToNow } from "date-fns";
import Link from "next/link";
import { notFound } from "next/navigation";

import { AttachDocumentForm } from "@/components/attach-document-form";
import { JobDetailClient } from "@/components/job-detail-client";
import { getDb } from "@/lib/db";
import { documents } from "@/lib/db/schema";
import { getJobDetail } from "@/lib/jobs/service";
import { formatLabel } from "@/lib/utils";
import { desc } from "drizzle-orm";

export const dynamic = "force-dynamic";

export default async function JobDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const detail = getJobDetail(id);
  if (!detail) {
    notFound();
  }

  const library = getDb().select().from(documents).orderBy(desc(documents.importedAt)).all();

  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <Link
        href="/"
        className="text-sm text-[var(--muted)] transition-colors hover:text-[var(--accent)]"
      >
        ← All jobs
      </Link>

      <JobDetailClient
        jobId={detail.job.id}
        initial={{
          title: detail.job.title,
          companyName: detail.company.name,
          status: detail.job.status,
          appliedAt: detail.job.appliedAt,
          notes: detail.job.notes,
          postingState: detail.job.postingState,
          lastCheckedAt: detail.job.lastCheckedAt,
          lastCheckResult: detail.job.lastCheckResult,
          url: detail.job.url,
          isNewFromWatch: detail.job.isNewFromWatch,
        }}
      />

      <div className="grid gap-4 lg:grid-cols-2">
        <section className="card p-6">
          <h2 className="mb-4 font-display text-lg font-medium">Documents</h2>
          <div className="space-y-3">
            {detail.attached.length === 0 ? (
              <p className="text-sm text-[var(--muted)]">
                Nothing attached yet. Add a resume or cover letter below.
              </p>
            ) : (
              detail.attached.map(({ attachment, document }) => (
                <div
                  key={attachment.id}
                  className="flex items-center justify-between gap-3 rounded-xl bg-[var(--surface-muted)] px-4 py-3"
                >
                  <div className="min-w-0">
                    <p className="text-xs font-medium text-[var(--accent-ink)]">
                      {formatLabel(attachment.kind)}
                    </p>
                    <p className="truncate font-medium">{document.originalFilename}</p>
                    <p className="text-xs text-[var(--faint)]">
                      Imported{" "}
                      {formatDistanceToNow(new Date(document.importedAt), {
                        addSuffix: true,
                      })}
                    </p>
                  </div>
                  <a
                    href={`/api/documents/${document.id}`}
                    className="shrink-0 text-sm font-medium text-[var(--accent)] hover:underline"
                  >
                    Open
                  </a>
                </div>
              ))
            )}
            <AttachDocumentForm
              jobId={detail.job.id}
              library={library.map((doc) => ({
                id: doc.id,
                originalFilename: doc.originalFilename,
              }))}
            />
          </div>
        </section>

        <section className="card p-6">
          <h2 className="mb-4 font-display text-lg font-medium">Timeline</h2>
          {detail.events.length === 0 ? (
            <p className="text-sm text-[var(--muted)]">No activity yet.</p>
          ) : (
            <ul className="space-y-4">
              {detail.events.map((event) => (
                <li key={event.id} className="relative pl-5">
                  <span className="absolute left-0 top-1.5 h-2 w-2 rounded-full bg-[var(--accent-soft)] ring-2 ring-[var(--accent)]" />
                  <p className="text-sm font-medium">{formatLabel(event.type)}</p>
                  <p className="text-xs text-[var(--faint)]">
                    {formatDistanceToNow(new Date(event.occurredAt), { addSuffix: true })}
                  </p>
                  {event.note ? (
                    <p className="mt-0.5 text-sm text-[var(--muted)]">{event.note}</p>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}
