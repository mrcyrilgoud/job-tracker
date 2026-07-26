"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";

import { documentKinds, type DocumentKind } from "@/lib/db/schema";
import { formatLabel } from "@/lib/utils";

type LibraryDoc = {
  id: string;
  originalFilename: string;
};

export function AttachDocumentForm({
  jobId,
  library,
}: {
  jobId: string;
  library: LibraryDoc[];
}) {
  const router = useRouter();
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
      const response = await fetch("/api/documents/attach", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ jobId, documentId, kind }),
      });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error ?? "Attach failed");
      router.refresh();
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
      const form = new FormData();
      form.set("file", file);
      form.set("jobId", jobId);
      form.set("kind", kind);
      const response = await fetch("/api/documents", { method: "POST", body: form });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error ?? "Import failed");
      setFile(null);
      router.refresh();
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
              onClick={attachExisting}
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
          onClick={importAndAttach}
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
