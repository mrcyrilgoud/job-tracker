import { useCallback, useEffect, useState } from "react";

import { DocumentsClient } from "@/components/DocumentsClient";
import { api } from "@/lib/api";
import type { DocumentListItem } from "@/lib/schema";
import { formatLabel } from "@/lib/utils";

export function DocumentsPage() {
  const [documents, setDocuments] = useState<DocumentListItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.listDocuments();
      setDocuments(result.documents);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load documents");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">Documents</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Import a resume or cover letter once, then reuse it across applications.
        </p>
      </div>

      <DocumentsClient onImported={() => void load()} />

      {loading ? <p className="text-sm text-[var(--muted)]">Loading documents…</p> : null}
      {error ? (
        <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
          {error}
        </p>
      ) : null}

      {!loading && documents.length === 0 ? (
        <div className="card px-6 py-16 text-center">
          <p className="font-display text-lg">No documents yet</p>
          <p className="mt-1 text-sm text-[var(--muted)]">
            Import a PDF, DOCX, or TXT to get started.
          </p>
        </div>
      ) : null}

      {!loading && documents.length > 0 ? (
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
