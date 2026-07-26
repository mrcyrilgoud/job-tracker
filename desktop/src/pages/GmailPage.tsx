import { useCallback, useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";

import { GmailClient } from "@/components/GmailClient";
import { api } from "@/lib/api";

export function GmailPage() {
  const [searchParams] = useSearchParams();
  const [status, setStatus] = useState<Awaited<ReturnType<typeof api.gmailStatus>> | null>(null);
  const [jobs, setJobs] = useState<Array<{ id: string; label: string }>>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [gmailResult, jobsResult] = await Promise.all([
        api.gmailStatus(),
        api.listJobs(),
      ]);
      setStatus(gmailResult);
      setJobs(
        jobsResult.jobs.map(({ job, companyName }) => ({
          id: job.id,
          label: `${job.title} · ${companyName}`,
        })),
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load Gmail status");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">Gmail</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Connect your inbox and Job Tracker will quietly match application emails to the right job.
          It only ever reads, and everything stays on your Mac.
        </p>
      </div>

      {loading ? <p className="text-sm text-[var(--muted)]">Loading Gmail…</p> : null}
      {error ? (
        <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
          {error}
        </p>
      ) : null}

      {status ? (
        <GmailClient
          connected={status.connected}
          configured={status.configured}
          redirectUri={status.redirectUri}
          pending={status.pending}
          jobs={jobs}
          initialError={searchParams.get("error") ?? undefined}
          justConnected={searchParams.get("connected") === "1"}
          onChanged={() => void load()}
        />
      ) : null}
    </div>
  );
}
