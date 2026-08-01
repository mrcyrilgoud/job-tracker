import { useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";

import { api, type ConfirmedJobDiscovery, type JobUrlPreview } from "@/lib/api";
import {
  applyJobUrlPreview,
  confirmJobUrlPreview,
  formatJobSaveError,
  isConfirmedJobDiscovery,
  resetJobUrlDiscovery,
  serializeConfirmedJobDiscovery,
} from "@/lib/job-url-preview";
import { jobStatuses, type JobStatus } from "@/lib/schema";
import { jobStatusPresentation } from "@/lib/ui";

function isValidPostingUrl(value: string) {
  try {
    const parsed = new URL(value);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

export function NewJobPage() {
  const navigate = useNavigate();
  const [url, setUrl] = useState("");
  const [title, setTitle] = useState("");
  const [companyName, setCompanyName] = useState("");
  const [status, setStatus] = useState<JobStatus>("wishlist");
  const [appliedAt, setAppliedAt] = useState("");
  const [notes, setNotes] = useState("");
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [autofillError, setAutofillError] = useState<string | null>(null);
  const [autofillStatus, setAutofillStatus] = useState<string | null>(null);
  const [isAutofilling, setIsAutofilling] = useState(false);
  const [preview, setPreview] = useState<JobUrlPreview | null>(null);
  const [confirmedDiscovery, setConfirmedDiscovery] =
    useState<ConfirmedJobDiscovery | null>(null);
  const autofillInFlight = useRef(false);

  async function onAutofill() {
    if (autofillInFlight.current) return;

    const postingUrl = url.trim();
    setAutofillError(null);
    setAutofillStatus(null);

    if (!isValidPostingUrl(postingUrl)) {
      setAutofillError("Enter a valid http or https posting link.");
      return;
    }

    autofillInFlight.current = true;
    setIsAutofilling(true);
    try {
      const preview = await api.previewJobUrl(postingUrl);
      setPreview(preview);
      setConfirmedDiscovery(resetJobUrlDiscovery().confirmedDiscovery);
      setTitle((current) =>
        applyJobUrlPreview({ title: current, companyName }, preview).title,
      );
      setCompanyName((current) =>
        applyJobUrlPreview({ title, companyName: current }, preview).companyName,
      );

      if (preview.title && preview.companyName) {
        setAutofillStatus("Found the role title and company.");
      } else if (preview.title) {
        setAutofillStatus("Found the role title. Company could not be found.");
      } else if (preview.companyName) {
        setAutofillStatus("Found the company. Role title could not be found.");
      } else {
        setAutofillStatus("No title or company was found. Enter the details manually.");
      }
    } catch {
      setAutofillError("Could not autofill details. Check the link and try again.");
    } finally {
      autofillInFlight.current = false;
      setIsAutofilling(false);
    }
  }

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
          Paste the posting link and hit Autofill — we&apos;ll fill title/company and detect the
          job board when we can. To watch a company&apos;s board ongoing, use the Companies tab.
        </p>
      </div>

      <form onSubmit={(e) => void onSubmit(e)} className="card space-y-5 p-6">
        <div className="space-y-1.5 text-sm">
          <label htmlFor="posting-url" className="block font-medium">
            Posting link
          </label>
          <div className="flex flex-col gap-2 sm:flex-row">
            <input
              id="posting-url"
              required
              type="url"
              value={url}
              onChange={(e) => {
                setUrl(e.target.value);
                setAutofillError(null);
                setAutofillStatus(null);
                setPreview(null);
                setConfirmedDiscovery(null);
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
              {isAutofilling ? "Finding details…" : "Autofill + detect board"}
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
        {preview?.board ? (
          <div className="rounded-xl border border-[var(--line)] bg-[var(--surface-muted)] p-4">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="space-y-1">
                <p className="text-sm font-medium">
                  {isConfirmedJobDiscovery(preview, confirmedDiscovery)
                    ? "Watch confirmed"
                    : "Detected job board"}
                </p>
                <p className="text-sm text-[var(--muted)]">
                  {preview.board.provider} / {preview.board.boardSlug}
                </p>
                <a
                  href={preview.board.boardUrl}
                  target="_blank"
                  rel="noreferrer"
                  className="text-sm text-[var(--accent)] hover:underline"
                >
                  {preview.board.boardUrl}
                </a>
              </div>
              {isConfirmedJobDiscovery(preview, confirmedDiscovery) ? (
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setConfirmedDiscovery(null)}
                >
                  Clear
                </button>
              ) : (
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setConfirmedDiscovery(confirmJobUrlPreview(preview))}
                >
                  Use/Watch this board
                </button>
              )}
            </div>
          </div>
        ) : preview?.careersUrl ? (
          <div className="rounded-xl border border-[var(--line)] bg-[var(--surface-muted)] p-4">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="space-y-1">
                <p className="text-sm font-medium">
                  {isConfirmedJobDiscovery(preview, confirmedDiscovery)
                    ? "Careers page confirmed"
                    : "Detected careers page"}
                </p>
                <a
                  href={preview.careersUrl}
                  target="_blank"
                  rel="noreferrer"
                  className="text-sm text-[var(--accent)] hover:underline"
                >
                  {preview.careersUrl}
                </a>
              </div>
              {isConfirmedJobDiscovery(preview, confirmedDiscovery) ? (
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setConfirmedDiscovery(null)}
                >
                  Clear
                </button>
              ) : (
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setConfirmedDiscovery(confirmJobUrlPreview(preview))}
                >
                  Use this careers page
                </button>
              )}
            </div>
          </div>
        ) : null}
        <div className="grid gap-4 md:grid-cols-2">
          <label className="block space-y-1.5 text-sm">
            <span className="font-medium">
              Role title <span className="text-[var(--faint)]">(optional)</span>
            </span>
            <input value={title} onChange={(e) => setTitle(e.target.value)} className="field" />
          </label>
          <label className="block space-y-1.5 text-sm">
            <span className="font-medium">
              Company <span className="text-[var(--faint)]">(optional)</span>
            </span>
            <input
              value={companyName}
              onChange={(e) => setCompanyName(e.target.value)}
              className="field"
            />
          </label>
        </div>
        <div className="grid gap-4 md:grid-cols-2">
          <label className="block space-y-1.5 text-sm">
            <span className="font-medium">Stage</span>
            <select
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
          </label>
          <label className="block space-y-1.5 text-sm">
            <span className="font-medium">
              Applied on <span className="text-[var(--faint)]">(optional)</span>
            </span>
            <input
              type="date"
              value={appliedAt}
              onChange={(e) => setAppliedAt(e.target.value)}
              className="field"
            />
          </label>
        </div>
        <label className="block space-y-1.5 text-sm">
          <span className="font-medium">Notes</span>
          <textarea
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            rows={4}
            placeholder="Anything worth remembering about this one…"
            className="field"
          />
        </label>
        {submitError ? (
          <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
            {submitError}
          </p>
        ) : null}
        <button type="submit" disabled={isSaving} className="btn btn-primary">
          {isSaving ? "Saving…" : "Track this job"}
        </button>
      </form>
    </div>
  );
}
