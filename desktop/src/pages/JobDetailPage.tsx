import { formatDistanceToNow } from "date-fns";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useParams } from "react-router-dom";

import { AttachDocumentForm } from "@/components/AttachDocumentForm";
import {
  JobDetailClient,
  type JobDetailUpdateMode,
} from "@/components/JobDetailClient";
import { api } from "@/lib/api";
import type { DocumentListItem, JobDetail } from "@/lib/schema";
import { formatLabel } from "@/lib/utils";

export function JobDetailPage() {
  const { id } = useParams<{ id: string }>();
  const [detail, setDetail] = useState<JobDetail | null>(null);
  const [library, setLibrary] = useState<DocumentListItem[]>([]);
  const [initialLoading, setInitialLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sequenceRef = useRef(0);
  const idRef = useRef(id);
  idRef.current = id;

  const loadDetail = useCallback(async (opts?: { quiet?: boolean }) => {
    if (!id) return;
    const requestId = id;
    const sequence = ++sequenceRef.current;
    const quiet = Boolean(opts?.quiet);
    if (quiet) {
      setRefreshing(true);
    } else {
      setInitialLoading(true);
    }
    setError(null);
    try {
      const jobResult = await api.getJob(requestId);
      if (sequence !== sequenceRef.current || idRef.current !== requestId) return;
      setDetail(jobResult.detail);
    } catch (err) {
      if (sequence !== sequenceRef.current || idRef.current !== requestId) return;
      setError(err instanceof Error ? err.message : "Failed to load job");
    } finally {
      if (sequence === sequenceRef.current) {
        setInitialLoading(false);
        setRefreshing(false);
      }
    }
  }, [id]);

  const loadLibrary = useCallback(async () => {
    if (!id) return;
    const requestId = id;
    try {
      const docsResult = await api.listDocuments();
      if (idRef.current !== requestId) return;
      setLibrary(docsResult.documents);
    } catch {
      // Library is secondary; detail error handling covers the page shell.
    }
  }, [id]);

  useEffect(() => {
    void loadDetail();
    void loadLibrary();
  }, [loadDetail, loadLibrary]);

  const onUpdated = useCallback(
    (payload: { detail?: JobDetail; mode: JobDetailUpdateMode }) => {
      if (payload.mode === "save" && payload.detail) {
        setDetail((current) => {
          if (!current) return payload.detail ?? null;
          return {
            ...payload.detail!,
            attached: current.attached,
            events: payload.detail!.events.length
              ? payload.detail!.events
              : current.events,
          };
        });
        return;
      }
      if (payload.mode === "check") {
        void loadDetail({ quiet: true });
        return;
      }
      if (payload.mode === "attachment") {
        void loadDetail({ quiet: true });
        void loadLibrary();
        return;
      }
      void loadDetail({ quiet: true });
      void loadLibrary();
    },
    [loadDetail, loadLibrary],
  );

  if (initialLoading && !detail) {
    return <p className="text-sm text-[var(--muted)]">Loading job…</p>;
  }

  if ((error && !detail) || !detail) {
    return (
      <div className="mx-auto max-w-4xl space-y-4">
        <Link to="/" className="text-sm text-[var(--muted)] hover:text-[var(--accent)]">
          ← All jobs
        </Link>
        <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
          {error ?? "Job not found"}
        </p>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <Link
        to="/"
        className="text-sm text-[var(--muted)] transition-colors hover:text-[var(--accent)]"
      >
        ← All jobs
      </Link>

      {refreshing ? (
        <p className="sr-only" aria-live="polite">
          Refreshing job…
        </p>
      ) : null}

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
          isFavorite: detail.job.isFavorite,
        }}
        onUpdated={onUpdated}
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
                      {formatDistanceToNow(new Date(document.importedAt), { addSuffix: true })}
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => void api.openDocument(document.id)}
                    className="shrink-0 text-sm font-medium text-[var(--accent)] hover:underline"
                  >
                    Open
                  </button>
                </div>
              ))
            )}
            <AttachDocumentForm
              jobId={detail.job.id}
              library={library.map((doc) => ({
                id: doc.id,
                originalFilename: doc.originalFilename,
              }))}
              onAttached={() => onUpdated({ mode: "attachment" })}
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
