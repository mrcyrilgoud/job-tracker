import { useState } from "react";

import { api } from "@/lib/api";
import { documentKinds, type DocumentKind } from "@/lib/schema";
import { fileToBase64, formatLabel } from "@/lib/utils";

type LibraryDoc = {
  id: string;
  originalFilename: string;
};

export function AttachDocumentForm({
  jobId,
  library,
  onAttached,
}: {
  jobId: string;
  library: LibraryDoc[];
  onAttached: () => void;
}) {
  const [documentId, setDocumentId] = useState(library[0]?.id ?? "");
  const [kind, setKind] = useState<DocumentKind>("resume");
  const [file, setFile] = useState<File | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function attachExisting() {
    if (!documentId) return;
    setBusy(true);
    setError(null);
    try {
      await api.attachDocument(jobId, documentId, kind);
      onAttached();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Attach failed");
    } finally {
      setBusy(false);
    }
  }

  async function importAndAttach() {
    if (!file) return;
    setBusy(true);
    setError(null);
    try {
      const bytesBase64 = await fileToBase64(file);
      await api.importDocument({
        originalFilename: file.name,
        mimeType: file.type || "application/octet-stream",
        bytesBase64,
        jobId,
        kind,
      });
      setFile(null);
      onAttached();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Import failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3 rounded-xl border border-dashed border-[var(--border)] p-4">
      <p className="text-sm font-medium">Attach a document</p>
      <div className="flex flex-wrap gap-2">
        <select
          value={kind}
          onChange={(e) => setKind(e.target.value as DocumentKind)}
          className="field w-auto"
        >
          {documentKinds.map((value) => (
            <option key={value} value={value}>
              {formatLabel(value)}
            </option>
          ))}
        </select>
        {library.length > 0 ? (
          <>
            <select
              value={documentId}
              onChange={(e) => setDocumentId(e.target.value)}
              className="field min-w-[180px] flex-1"
            >
              {library.map((doc) => (
                <option key={doc.id} value={doc.id}>
                  {doc.originalFilename}
                </option>
              ))}
            </select>
            <button
              type="button"
              onClick={() => void attachExisting()}
              disabled={busy}
              className="btn btn-primary"
            >
              Attach
            </button>
          </>
        ) : null}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <label className="btn btn-secondary cursor-pointer">
          {file ? file.name : "Upload new"}
          <input
            type="file"
            accept=".pdf,.docx,.txt,application/pdf,text/plain,application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            onChange={(e) => setFile(e.target.files?.[0] ?? null)}
            className="hidden"
          />
        </label>
        <button
          type="button"
          onClick={() => void importAndAttach()}
          disabled={busy || !file}
          className="btn btn-secondary"
        >
          Import &amp; attach
        </button>
      </div>
      {error ? <p className="text-sm text-[var(--danger)]">{error}</p> : null}
    </div>
  );
}
