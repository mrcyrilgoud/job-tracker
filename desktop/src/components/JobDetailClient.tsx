import { formatDistanceToNow } from "date-fns";
import { useCallback, useEffect, useState } from "react";

import { AlertIcon, ArrowUpRightIcon, CheckIcon } from "@/components/icons";
import { api } from "@/lib/api";
import { jobStatuses, type JobDetail, type JobStatus, type PostingState } from "@/lib/schema";
import {
  checkResultNote,
  jobStatusPresentation,
  postingStateMatters,
  postingStatePresentation,
  toneClasses,
} from "@/lib/ui";

type Initial = {
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

/** The editable subset of a job. Compared against the last persisted copy to
 *  decide whether there is anything to save. */
type Draft = {
  title: string;
  companyName: string;
  status: JobStatus;
  appliedAt: string;
  notes: string;
};

export type JobDetailUpdateMode = "save" | "check" | "attachment" | "full";

type SaveState = "saving" | "unsaved" | "just-saved" | "clean";

function toDraft(initial: Initial): Draft {
  return {
    title: initial.title,
    companyName: initial.companyName,
    status: initial.status,
    appliedAt: initial.appliedAt ? initial.appliedAt.slice(0, 10) : "",
    notes: initial.notes ?? "",
  };
}

function draftsMatch(a: Draft, b: Draft): boolean {
  return (
    a.title === b.title &&
    a.companyName === b.companyName &&
    a.status === b.status &&
    a.appliedAt === b.appliedAt &&
    a.notes === b.notes
  );
}

function saveStateCopy(state: SaveState): { label: string; className: string } {
  switch (state) {
    case "saving":
      return { label: "Saving…", className: "text-[var(--muted)]" };
    case "unsaved":
      return { label: "Unsaved changes", className: "text-[var(--amber-ink)]" };
    case "just-saved":
      return { label: "Saved", className: "text-[var(--green-ink)]" };
    case "clean":
      return { label: "All changes saved", className: "text-[var(--faint)]" };
    default: {
      const _exhaustive: never = state;
      return _exhaustive;
    }
  }
}

export function JobDetailClient({
  jobId,
  initial,
  onUpdated,
}: {
  jobId: string;
  initial: Initial;
  onUpdated: (payload: { detail?: JobDetail; mode: JobDetailUpdateMode }) => void;
}) {
  // `saved` mirrors what the database holds; `draft` is what the person sees.
  // The gap between the two is the entire save story this card tells.
  const [saved, setSaved] = useState<Draft>(() => toDraft(initial));
  const [draft, setDraft] = useState<Draft>(() => toDraft(initial));
  const [posting, setPosting] = useState({
    state: initial.postingState,
    lastCheckedAt: initial.lastCheckedAt,
    lastCheckResult: initial.lastCheckResult,
  });
  const [saving, setSaving] = useState(false);
  const [checking, setChecking] = useState(false);
  const [justSaved, setJustSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const isDirty = !draftsMatch(draft, saved);
  const postingInfo = postingStateMatters(draft.status)
    ? postingStatePresentation(posting.state)
    : null;
  const checkNote = checkResultNote(posting.lastCheckResult);

  const saveState: SaveState = saving
    ? "saving"
    : isDirty
      ? "unsaved"
      : justSaved
        ? "just-saved"
        : "clean";
  const saveCopy = saveStateCopy(saveState);

  useEffect(() => {
    if (!justSaved) return;
    const timer = setTimeout(() => setJustSaved(false), 4000);
    return () => clearTimeout(timer);
  }, [justSaved]);

  useEffect(() => {
    if (!isDirty) return;
    function warn(event: BeforeUnloadEvent) {
      event.preventDefault();
    }
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [isDirty]);

  function update<K extends keyof Draft>(key: K, value: Draft[K]) {
    setDraft((current) => ({ ...current, [key]: value }));
    setJustSaved(false);
    setError(null);
  }

  const save = useCallback(async () => {
    setSaving(true);
    setError(null);
    const attempted = draft;
    try {
      const result = await api.updateJob(jobId, {
        title: attempted.title,
        companyName: attempted.companyName,
        status: attempted.status,
        appliedAt: attempted.appliedAt ? new Date(attempted.appliedAt).toISOString() : null,
        notes: attempted.notes,
        isNewFromWatch: false,
      });
      const detail = result.detail;
      const baseline: Draft = {
        title: detail.job.title,
        companyName: detail.company.name,
        status: detail.job.status,
        appliedAt: detail.job.appliedAt ? detail.job.appliedAt.slice(0, 10) : "",
        notes: detail.job.notes ?? "",
      };
      setSaved(baseline);
      // If the user typed more while save was in flight, keep the newer draft.
      setDraft((current) => (draftsMatch(current, attempted) ? baseline : current));
      setJustSaved(true);
      onUpdated({ detail, mode: "save" });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Save failed");
    } finally {
      setSaving(false);
    }
  }, [draft, jobId, onUpdated]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "s" || !(event.metaKey || event.ctrlKey)) return;
      event.preventDefault();
      if (isDirty && !saving) void save();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [isDirty, saving, save]);

  async function checkPosting() {
    setChecking(true);
    setError(null);
    try {
      const result = await api.checkJobPosting(jobId);
      setPosting({
        state: result.postingState as PostingState,
        lastCheckedAt: result.lastCheckedAt,
        lastCheckResult: result.lastCheckResult,
      });
      onUpdated({ mode: "check" });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Check failed");
    } finally {
      setChecking(false);
    }
  }

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
      className="card overflow-hidden"
    >
      {/* Reversed on narrow screens so the status sits above a full-width title
          instead of squeezing it into a clipped column. */}
      <div className="flex flex-col-reverse gap-3 p-6 pb-5 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
        <div className="min-w-0 flex-1 space-y-1">
          <input
            value={draft.title}
            onChange={(event) => update("title", event.target.value)}
            aria-label="Job title"
            className="field-quiet w-full max-w-xl font-display text-2xl font-semibold tracking-tight"
          />
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-[var(--muted)]">
            <input
              value={draft.companyName}
              onChange={(event) => update("companyName", event.target.value)}
              aria-label="Company"
              size={Math.min(Math.max(draft.companyName.length, 6), 36)}
              className="field-quiet"
            />
            <span aria-hidden="true" className="text-[var(--faint)]">
              ·
            </span>
            <a
              href={initial.url}
              target="_blank"
              rel="noreferrer"
              className="inline-flex items-center gap-1 font-medium text-[var(--accent)] hover:underline"
            >
              Open posting
              <ArrowUpRightIcon size={13} />
            </a>
          </div>
        </div>

        <div className="flex shrink-0 flex-row items-center gap-2 sm:flex-col sm:items-end sm:gap-1.5 sm:text-right">
          {postingInfo ? (
            <span className={`pill ${toneClasses[postingInfo.tone]}`}>
              <span className="pill-dot" />
              {postingInfo.label}
            </span>
          ) : null}
          {posting.lastCheckedAt ? (
            <span className="text-xs text-[var(--faint)]">
              Checked{" "}
              {formatDistanceToNow(new Date(posting.lastCheckedAt), { addSuffix: true })}
            </span>
          ) : null}
        </div>
      </div>

      <div className="space-y-5 border-t border-[var(--border)] p-6">
        <div className="grid max-w-lg gap-4 sm:grid-cols-2">
          <div>
            <label htmlFor="job-stage" className="field-label">
              Stage
            </label>
            <select
              id="job-stage"
              value={draft.status}
              onChange={(event) => update("status", event.target.value as JobStatus)}
              className="field"
            >
              {jobStatuses.map((value) => (
                <option key={value} value={value}>
                  {jobStatusPresentation(value).label}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label htmlFor="job-applied" className="field-label">
              Applied
            </label>
            <input
              id="job-applied"
              type="date"
              value={draft.appliedAt}
              onChange={(event) => update("appliedAt", event.target.value)}
              aria-describedby={draft.appliedAt ? undefined : "job-applied-hint"}
              className="field"
            />
            {draft.appliedAt ? (
              <button
                type="button"
                onClick={() => update("appliedAt", "")}
                className="btn btn-ghost btn-sm mt-1 -ml-2.5"
              >
                Clear date
              </button>
            ) : (
              <p id="job-applied-hint" className="mt-1.5 text-xs text-[var(--faint)]">
                No date set yet
              </p>
            )}
          </div>
        </div>

        <div>
          <label htmlFor="job-notes" className="field-label">
            Notes
          </label>
          <textarea
            id="job-notes"
            value={draft.notes}
            onChange={(event) => update("notes", event.target.value)}
            rows={4}
            placeholder="Anything worth remembering about this one…"
            className="field"
          />
        </div>

        {checkNote ? <p className="text-xs text-[var(--muted)]">{checkNote}</p> : null}

        {error ? (
          <p className="flex items-center gap-2 rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
            <AlertIcon size={15} />
            <span className="flex-1">{error}</span>
            <button type="button" className="font-medium underline" onClick={() => setError(null)}>
              Dismiss
            </button>
          </p>
        ) : null}
      </div>

      <div className="card-footer">
        <p aria-live="polite" className={`save-state ${saveCopy.className}`}>
          {saveState === "saving" ? (
            <span className="spinner" />
          ) : saveState === "just-saved" ? (
            <CheckIcon size={14} />
          ) : (
            <span className="save-state-dot" />
          )}
          {saveCopy.label}
        </p>

        <div className="ml-auto flex items-center gap-2">
          {isDirty ? (
            <button
              type="button"
              onClick={() => {
                setDraft(saved);
                setError(null);
              }}
              disabled={saving}
              className="btn btn-ghost"
            >
              Discard
            </button>
          ) : null}
          <button
            type="button"
            onClick={() => void checkPosting()}
            disabled={checking}
            className="btn btn-secondary"
          >
            {checking ? <span className="spinner" /> : null}
            {checking ? "Checking…" : "Check posting"}
          </button>
          <button type="submit" disabled={!isDirty || saving} className="btn btn-primary">
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </form>
  );
}
