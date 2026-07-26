"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";

import { jobStatuses, type JobStatus } from "@/lib/db/schema";
import { jobStatusPresentation } from "@/lib/ui";

export default function NewJobPage() {
  const router = useRouter();
  const [url, setUrl] = useState("");
  const [title, setTitle] = useState("");
  const [companyName, setCompanyName] = useState("");
  const [status, setStatus] = useState<JobStatus>("wishlist");
  const [appliedAt, setAppliedAt] = useState("");
  const [notes, setNotes] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function onSubmit(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const response = await fetch("/api/jobs", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          url,
          title: title || undefined,
          companyName: companyName || undefined,
          status,
          appliedAt: appliedAt ? new Date(appliedAt).toISOString() : null,
          notes: notes || null,
        }),
      });
      const data = await response.json();
      if (!response.ok) {
        throw new Error(data.error ?? "Could not create job");
      }
      router.push(`/jobs/${data.job.id}`);
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not create job");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mx-auto max-w-2xl space-y-6">
      <Link
        href="/"
        className="text-sm text-[var(--muted)] transition-colors hover:text-[var(--accent)]"
      >
        ← All jobs
      </Link>

      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">Add a job</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Paste the posting link and we&apos;ll fill in what we can. You can fix the title
          and company afterward.
        </p>
      </div>

      <form onSubmit={onSubmit} className="card space-y-5 p-6">
        <label className="block space-y-1.5 text-sm">
          <span className="font-medium">Posting link</span>
          <input
            required
            type="url"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="https://…"
            className="field"
          />
        </label>
        <div className="grid gap-4 md:grid-cols-2">
          <label className="block space-y-1.5 text-sm">
            <span className="font-medium">
              Role title <span className="text-[var(--faint)]">(optional)</span>
            </span>
            <input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              className="field"
            />
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
        {error ? (
          <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
            {error}
          </p>
        ) : null}
        <button disabled={busy} className="btn btn-primary">
          {busy ? "Saving…" : "Track this job"}
        </button>
      </form>
    </div>
  );
}
