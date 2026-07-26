import { formatDistanceToNow } from "date-fns";
import { useState } from "react";

import { api } from "@/lib/api";
import { jobStatuses, type JobStatus, type PostingState } from "@/lib/schema";
import { jobStatusPresentation, postingStatePresentation, toneClasses } from "@/lib/ui";

export function JobDetailClient({
  jobId,
  initial,
  onUpdated,
}: {
  jobId: string;
  initial: {
    title: string;
    companyName: string;
    status: JobStatus;
    appliedAt: string | null;
    notes: string | null;
    postingState: PostingState;
    lastCheckedAt: string | null;
    lastCheckResult: string | null;
    url: string;
    isNewFromWatch: boolean;
  };
  onUpdated: () => void;
}) {
  const [status, setStatus] = useState(initial.status);
  const [title, setTitle] = useState(initial.title);
  const [companyName, setCompanyName] = useState(initial.companyName);
  const [appliedAt, setAppliedAt] = useState(
    initial.appliedAt ? initial.appliedAt.slice(0, 10) : "",
  );
  const [notes, setNotes] = useState(initial.notes ?? "");
  const [postingState, setPostingState] = useState(initial.postingState);
  const [lastCheckedAt, setLastCheckedAt] = useState(initial.lastCheckedAt);
  const [lastCheckResult, setLastCheckResult] = useState(initial.lastCheckResult);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const postingInfo = postingStatePresentation(postingState);

  async function save() {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await api.updateJob(jobId, {
        title,
        companyName,
        status,
        appliedAt: appliedAt ? new Date(appliedAt).toISOString() : null,
        notes,
        isNewFromWatch: false,
      });
      setMessage("Saved");
      onUpdated();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Save failed");
    } finally {
      setBusy(false);
    }
  }

  async function checkPosting() {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const result = await api.checkJobPosting(jobId);
      const info = postingStatePresentation(result.postingState as PostingState);
      setPostingState(result.postingState as PostingState);
      setLastCheckedAt(result.lastCheckedAt);
      setLastCheckResult(result.lastCheckResult);
      setMessage(`This posting looks ${info.label.toLowerCase()}.`);
      onUpdated();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Check failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="card space-y-5 p-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 flex-1 space-y-2">
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            className="w-full max-w-xl border-0 bg-transparent font-display text-2xl font-semibold tracking-tight outline-none"
          />
          <div className="flex flex-wrap items-center gap-2 text-sm text-[var(--muted)]">
            <input
              value={companyName}
              onChange={(e) => setCompanyName(e.target.value)}
              className="field w-auto max-w-[220px] px-3 py-1.5"
            />
            <a
              href={initial.url}
              target="_blank"
              rel="noreferrer"
              className="font-medium text-[var(--accent)] hover:underline"
            >
              Open posting ↗
            </a>
          </div>
        </div>
        <span className={`pill ${toneClasses[postingInfo.tone]}`}>
          <span className="pill-dot" />
          {postingInfo.label}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-3 rounded-xl bg-[var(--surface-muted)] p-4">
        <label className="text-sm text-[var(--muted)]">
          <span className="mr-2 font-medium text-[var(--foreground)]">Stage</span>
          <select
            value={status}
            onChange={(e) => setStatus(e.target.value as JobStatus)}
            className="field w-auto"
          >
            {jobStatuses.map((value) => (
              <option key={value} value={value}>
                {jobStatusPresentation(value).label}
              </option>
            ))}
          </select>
        </label>
        <label className="text-sm text-[var(--muted)]">
          <span className="mr-2 font-medium text-[var(--foreground)]">Applied</span>
          <input
            type="date"
            value={appliedAt}
            onChange={(e) => setAppliedAt(e.target.value)}
            className="field w-auto"
          />
        </label>
        <div className="ml-auto flex gap-2">
          <button type="button" onClick={() => void checkPosting()} disabled={busy} className="btn btn-secondary">
            Check posting
          </button>
          <button type="button" onClick={() => void save()} disabled={busy} className="btn btn-primary">
            Save
          </button>
        </div>
      </div>

      <label className="block space-y-1.5">
        <span className="text-sm font-medium">Notes</span>
        <textarea
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          rows={4}
          placeholder="Anything worth remembering about this one…"
          className="field"
        />
      </label>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-[var(--faint)]">
        {lastCheckedAt ? (
          <span>
            Last checked {formatDistanceToNow(new Date(lastCheckedAt), { addSuffix: true })}
          </span>
        ) : (
          <span>Not checked yet</span>
        )}
        {lastCheckResult ? <span>· {lastCheckResult}</span> : null}
      </div>

      {message ? (
        <p className="rounded-xl bg-[var(--green-soft)] px-3.5 py-2.5 text-sm text-[var(--green-ink)]">
          {message}
        </p>
      ) : null}
      {error ? (
        <p className="flex items-center justify-between gap-2 rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
          {error}
          <button type="button" className="font-medium underline" onClick={() => setError(null)}>
            Dismiss
          </button>
        </p>
      ) : null}
    </div>
  );
}
