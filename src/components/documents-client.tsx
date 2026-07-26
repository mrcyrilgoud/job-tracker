"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";

export function DocumentsClient() {
  const router = useRouter();
  const [file, setFile] = useState<File | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function onImport() {
    if (!file) return;
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const form = new FormData();
      form.set("file", file);
      const response = await fetch("/api/documents", { method: "POST", body: form });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error ?? "Import failed");
      setMessage(`Imported ${data.document.originalFilename}`);
      setFile(null);
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Import failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="card space-y-3 p-5">
      <div className="flex flex-wrap items-center gap-3">
        <label className="btn btn-secondary cursor-pointer">
          {file ? file.name : "Choose a file"}
          <input
            type="file"
            accept=".pdf,.docx,.txt,application/pdf,text/plain,application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            onChange={(e) => setFile(e.target.files?.[0] ?? null)}
            className="hidden"
          />
        </label>
        <button
          onClick={onImport}
          disabled={!file || busy}
          className="btn btn-primary"
        >
          Import document
        </button>
        <span className="text-xs text-[var(--faint)]">PDF, DOCX, or TXT</span>
      </div>
      {message ? (
        <p className="text-sm text-[var(--green-ink)]">{message}</p>
      ) : null}
      {error ? <p className="text-sm text-[var(--danger)]">{error}</p> : null}
    </div>
  );
}
