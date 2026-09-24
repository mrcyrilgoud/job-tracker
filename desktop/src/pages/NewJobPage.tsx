import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";

import { api } from "@/lib/api";
import {
  confirmJobUrlPreview,
  formatJobSaveError,
  isConfirmedJobDiscovery,
  serializeConfirmedJobDiscovery,
  watchOptionForJobUrlPreview,
} from "@/lib/job-url-preview";
import { jobStatuses, type JobStatus } from "@/lib/schema";
import { jobStatusPresentation } from "@/lib/ui";
import { useJobUrlPreview } from "@/lib/use-job-url-preview";

export function NewJobPage() {
  const navigate = useNavigate();
  const [url, setUrl] = useState("");
  const [title, setTitle] = useState("");
  const [companyName, setCompanyName] = useState("");
  const [status, setStatus] = useState<JobStatus>("wishlist");
  const [appliedAt, setAppliedAt] = useState("");
  const [description, setDescription] = useState("");
  const [notes, setNotes] = useState("");
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  const {
    preview,
    confirmedDiscovery,
    setConfirmedDiscovery,
    autofillError,
    autofillStatus,
    isAutofilling,
    onAutofill,
    clearDiscoveryForUrlChange,
  } = useJobUrlPreview({
    url,
    title,
    companyName,
    description,
    setTitle,
    setCompanyName,
    setDescription,
  });
  const watchEnabled = isConfirmedJobDiscovery(preview, confirmedDiscovery);
  const watchOption = watchOptionForJobUrlPreview(preview, companyName);

  async function onSubmit(event: React.FormEvent) {
    event.preventDefault();
    setIsSaving(true);
    setSubmitError(null);
    try {
      const result = await api.createJob({
        url,
        title: title || undefined,
        companyName: companyName || undefined,
        status,
        appliedAt: appliedAt ? new Date(appliedAt).toISOString() : null,
        description: description || null,
        notes: notes || null,
        confirmedDiscovery: serializeConfirmedJobDiscovery(preview, confirmedDiscovery),
      });
      navigate(`/jobs/${result.job.id}`);
    } catch (err) {
      setSubmitError(formatJobSaveError(err));
    } finally {
      setIsSaving(false);
    }
  }

  return (
    <div className="mx-auto max-w-2xl space-y-6">
      <Link
        to="/"
        className="text-sm text-[var(--muted)] transition-colors hover:text-[var(--accent)]"
      >
        ← All jobs
      </Link>

      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">Add a job</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Import a posting first, then review its details and decide whether Job Tracker should
          keep watching for related roles.
        </p>
      </div>

      <form onSubmit={(e) => void onSubmit(e)} className="card space-y-5 p-6">
        <div className="space-y-1.5 text-sm">
          <div className="flex items-center gap-2">
            <span className="flex h-5 w-5 items-center justify-center rounded-full bg-[var(--accent-soft)] text-xs font-semibold text-[var(--accent-ink)]">
              1
            </span>
            <label htmlFor="posting-url" className="block font-medium">
              Import posting
            </label>
          </div>
          <div className="flex flex-col gap-2 sm:flex-row">
            <input
              id="posting-url"
              required
              type="url"
              value={url}
              onChange={(e) => {
                setUrl(e.target.value);
                clearDiscoveryForUrlChange();
              }}
              placeholder="https://…"
              className="field"
            />
            <button
              type="button"
              disabled={isAutofilling}
              onClick={() => void onAutofill()}
              className="btn btn-secondary shrink-0"
            >
              {isAutofilling ? "Finding details…" : "Find details"}
            </button>
          </div>
          {autofillError ? (
            <p className="text-sm text-[var(--danger)]" role="alert">
              {autofillError}
            </p>
          ) : null}
          {autofillStatus ? (
            <p className="text-sm text-[var(--muted)]" aria-live="polite">
              {autofillStatus}
            </p>
          ) : null}
        </div>
        {preview && watchOption ? (
          <div className="rounded-xl border border-[var(--border)] bg-[var(--surface-muted)] p-4">
            <div className="flex items-start gap-3">
              <span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-[var(--accent-soft)] text-xs font-semibold text-[var(--accent-ink)]">
                2
              </span>
              <div className="min-w-0 flex-1 space-y-3">
                <div className="space-y-1">
                  <p className="text-sm font-medium">Review detected source</p>
                  <p className="text-sm text-[var(--muted)]">
                    {preview.board
                      ? `Job Tracker found a ${preview.board.provider} job board.`
                      : "Job Tracker found this company careers page."}
                  </p>
                </div>
                <p className="text-sm text-[var(--muted)]">
                  {preview.board
                    ? `${preview.board.provider} / ${preview.board.boardSlug}`
                    : preview.careersUrl}
                </p>
                <a
                  href={preview.board ? preview.board.boardUrl : preview.careersUrl!}
                  target="_blank"
                  rel="noreferrer"
                  className="text-sm text-[var(--accent)] hover:underline"
                >
                  {preview.board ? preview.board.boardUrl : preview.careersUrl}
                </a>
                <label className="flex cursor-pointer items-start gap-3 rounded-xl border border-[var(--border)] bg-[var(--surface)] p-3 text-sm">
                  <input
                    type="checkbox"
                    checked={watchEnabled}
                    onChange={(event) =>
                      setConfirmedDiscovery(event.target.checked ? confirmJobUrlPreview(preview) : null)
                    }
                    className="mt-0.5 h-4 w-4 accent-[var(--accent)]"
                  />
                  <span>
                    <span className="block font-medium">{watchOption.label}</span>
                    <span className="mt-0.5 block text-xs leading-relaxed text-[var(--muted)]">
                      {watchOption.description}
                    </span>
                  </span>
                </label>
              </div>
            </div>
          </div>
        ) : null}

        <div className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-1.5 text-sm">
            <label htmlFor="title" className="block font-medium">
              Title
            </label>
            <input
              id="title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              className="field"
            />
          </div>
          <div className="space-y-1.5 text-sm">
            <label htmlFor="company" className="block font-medium">
              Company
            </label>
            <input
              id="company"
              value={companyName}
              onChange={(e) => setCompanyName(e.target.value)}
              className="field"
            />
          </div>
        </div>

        <div className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-1.5 text-sm">
            <label htmlFor="status" className="block font-medium">
              Status
            </label>
            <select
              id="status"
              value={status}
              onChange={(e) => setStatus(e.target.value as JobStatus)}
              className="field"
            >
              {jobStatuses.map((value) => (
                <option key={value} value={value}>
                  {jobStatusPresentation(value).label}
                </option>
              ))}
            </select>
          </div>
          <div className="space-y-1.5 text-sm">
            <label htmlFor="applied-at" className="block font-medium">
              Applied on
            </label>
            <input
              id="applied-at"
              type="date"
              value={appliedAt}
              onChange={(e) => setAppliedAt(e.target.value)}
              className="field"
            />
          </div>
        </div>

        <div className="space-y-1.5 text-sm">
          <div className="flex items-center justify-between">
            <label htmlFor="description" className="block font-medium">
              Job Description
            </label>
            <span className="text-xs text-[var(--faint)]">Optional / Auto-detected</span>
          </div>
          <textarea
            id="description"
            rows={5}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="Role overview, responsibilities, or requirements…"
            className="field font-mono text-xs leading-relaxed"
          />
        </div>

        <div className="space-y-1.5 text-sm">
          <div className="flex items-center justify-between">
            <label htmlFor="notes" className="block font-medium">
              Notes
            </label>
            <span className="text-xs text-[var(--faint)]">Personal notes</span>
          </div>
          <textarea
            id="notes"
            rows={3}
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            placeholder="Referral info, recruiter name, compensation notes…"
            className="field"
          />
        </div>

        {submitError ? (
          <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
            {submitError}
          </p>
        ) : null}

        <button type="submit" disabled={isSaving} className="btn btn-primary">
          {isSaving ? "Saving…" : watchEnabled ? "Save job and start watch" : "Save job"}
        </button>
      </form>
    </div>
  );
}
