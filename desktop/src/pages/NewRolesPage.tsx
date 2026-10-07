import { useCallback, useEffect, useState } from "react";

import { NewRolesPanel } from "@/components/companies/NewRolesPanel";
import type { TriageAction } from "@/components/companies/NewRolesList";
import { api } from "@/lib/api";
import { useRunMonitor } from "@/lib/RunMonitorContext";
import type { JobListItem } from "@/lib/schema";

export function NewRolesPage() {
  const [roles, setRoles] = useState<JobListItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [hasLoaded, setHasLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [triagingId, setTriagingId] = useState<string | null>(null);
  const { onRunSettled, reportRefreshFailed } = useRunMonitor();

  const load = useCallback(async (opts?: { quiet?: boolean }) => {
    if (!opts?.quiet) setLoading(true);
    setError(null);
    try {
      const result = await api.listJobs({ newFromWatch: true });
      setRoles(result.jobs);
      setHasLoaded(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to load new roles";
      setError(message);
      if (opts?.quiet) reportRefreshFailed(message);
    } finally {
      if (!opts?.quiet) setLoading(false);
    }
  }, [reportRefreshFailed]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(
    () => onRunSettled(() => { void load({ quiet: true }); }),
    [load, onRunSettled],
  );

  async function triageRole(jobId: string, action: TriageAction) {
    setTriagingId(jobId);
    setError(null);
    try {
      if (action === "save") {
        await api.approveWatchJob(jobId);
      } else {
        await api.dismissWatchJob(jobId);
      }
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to update this role");
    } finally {
      setTriagingId(null);
    }
  }

  return (
    <div className="mx-auto max-w-4xl space-y-5">
      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">New Roles</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Review openings found by your watched company boards.
        </p>
      </div>

      {loading && !hasLoaded ? (
        <p className="text-sm text-[var(--muted)]">Loading new roles…</p>
      ) : null}

      {error ? (
        <div
          role="alert"
          className="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]"
        >
          <span>{error}</span>
          <button type="button" className="btn btn-secondary btn-sm" onClick={() => void load()}>
            Retry
          </button>
        </div>
      ) : null}

      {hasLoaded ? (
        <NewRolesPanel
          roles={roles}
          onTriage={(jobId, action) => void triageRole(jobId, action)}
          isPending={(jobId) => triagingId === jobId}
        />
      ) : null}
    </div>
  );
}
