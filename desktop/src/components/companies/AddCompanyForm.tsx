import { useState } from "react";

import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/job-url-preview";

/**
 * Opened on demand from the page header. Previously this was a permanent card
 * occupying the top-left of the screen for a task most users do rarely.
 */
export function AddCompanyForm({
  onAdded,
  onCancel,
}: {
  onAdded: (companyName: string) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState("");
  const [careersUrl, setCareersUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      const result = await api.createCompany(name.trim(), careersUrl.trim() || null);
      setName("");
      setCareersUrl("");
      onAdded(result.company.name);
    } catch (err) {
      setError(formatInvokeError(err, "Could not add that company."));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      className="card space-y-3 p-5"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="block space-y-1.5 text-sm">
          <span className="font-medium">Company</span>
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="Anthropic"
            className="field"
            autoFocus
          />
        </label>
        <label className="block space-y-1.5 text-sm">
          <span className="font-medium">
            Careers page <span className="font-normal text-[var(--faint)]">optional</span>
          </span>
          <input
            value={careersUrl}
            onChange={(event) => setCareersUrl(event.target.value)}
            placeholder="https://…"
            className="field"
            type="url"
          />
        </label>
      </div>
      {error ? (
        <p className="text-sm text-[var(--danger)]" role="alert">
          {error}
        </p>
      ) : null}
      <div className="flex items-center gap-2">
        <button type="submit" className="btn btn-primary btn-sm" disabled={busy || !name.trim()}>
          {busy ? <span className="spinner" aria-hidden /> : null}
          Add company
        </button>
        <button type="button" className="btn btn-ghost btn-sm" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
