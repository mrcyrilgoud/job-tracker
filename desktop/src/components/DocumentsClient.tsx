import { useState } from "react";

import { api } from "@/lib/api";
import { fileToBase64 } from "@/lib/utils";

export function DocumentsClient({ onImported }: { onImported: () => void }) {
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
      const bytesBase64 = await fileToBase64(file);
      const result = await api.importDocument({
        originalFilename: file.name,
        mimeType: file.type || "application/octet-stream",
        bytesBase64,
      });
      setMessage(`Imported ${result.document.originalFilename}`);
      setFile(null);
      onImported();
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
          type="button"
          onClick={() => void onImport()}
          disabled={!file || busy}
          className="btn btn-primary"
        >
          Import document
        </button>
        <span className="text-xs text-[var(--faint)]">PDF, DOCX, or TXT</span>
      </div>
      {message ? <p className="text-sm text-[var(--green-ink)]">{message}</p> : null}
      {error ? <p className="text-sm text-[var(--danger)]">{error}</p> : null}
    </div>
  );
}
