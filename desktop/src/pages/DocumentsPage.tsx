import { useCallback, useEffect, useRef, useState } from "react";

import { DocumentsClient } from "@/components/DocumentsClient";
import { api } from "@/lib/api";
import type { DocumentListItem } from "@/lib/schema";
import { formatLabel } from "@/lib/utils";

export function DocumentsPage() {
  const [documents, setDocuments] = useState<DocumentListItem[]>([]);
  const [initialLoading, setInitialLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sequenceRef = useRef(0);

  const load = useCallback(async (opts?: { quiet?: boolean }) => {
    const sequence = ++sequenceRef.current;
    const quiet = Boolean(opts?.quiet);
    if (quiet) {
      setRefreshing(true);
    } else {
      setInitialLoading(true);
    }
    setError(null);
    try {
      const result = await api.listDocuments();
      if (sequence !== sequenceRef.current) return;
      setDocuments(result.documents);
    } catch (err) {
      if (sequence !== sequenceRef.current) return;
      setError(err instanceof Error ? err.message : "Failed to load documents");
    } finally {
      if (sequence === sequenceRef.current) {
        setInitialLoading(false);
        setRefreshing(false);
      }
    }
  }, []);

  useEffect(() => {
    void load();
    // Initial mount only; subsequent reloads go through onImported.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">Documents</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Import a resume or cover letter once, then reuse it across applications.
        </p>
      </div>

      <DocumentsClient onImported={() => void load({ quiet: true })} />

      {initialLoading ? <p className="text-sm text-[var(--muted)]">Loading documents…</p> : null}
      {refreshing ? (
        <p className="sr-only" aria-live="polite">
          Refreshing documents…
        </p>
      ) : null}
      {error ? (
        <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
          {error}
        </p>
      ) : null}

      {!initialLoading && documents.length === 0 ? (
        <div className="card px-6 py-16 text-center">
          <p className="font-display text-lg">No documents yet</p>
          <p className="mt-1 text-sm text-[var(--muted)]">
            Import a PDF, DOCX, or TXT to get started.
          </p>
        </div>
      ) : null}

      {!initialLoading && documents.length > 0 ? (
        <ul className="space-y-2">
          {documents.map((doc) => {
            const kinds = doc.kinds ?? [];
            const usedBy = doc.usedBy ?? [];
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
                  <button
                    type="button"
                    onClick={() => void api.openDocument(doc.id)}
                    className="shrink-0 text-sm font-medium text-[var(--accent)] hover:underline"
                  >
                    Open
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      ) : null}
    </div>
  );
}
