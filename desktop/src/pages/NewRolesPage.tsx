import { useCallback, useEffect, useState } from "react";

import { NewRolesInbox } from "@/components/NewRolesInbox";
import type { TriageAction } from "@/components/companies/NewRolesList";
import { api, type WatchRoleFilters } from "@/lib/api";
import { useRunMonitorActions } from "@/lib/RunMonitorContext";
import type { JobListItem, WatchRoleSummary } from "@/lib/schema";

const WATCH_NEW_ONLY_FILTERS: WatchRoleFilters = { newOnly: true };

export function NewRolesPage() {
  const [roles, setRoles] = useState<WatchRoleSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [hasLoaded, setHasLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [triagingId, setTriagingId] = useState<string | null>(null);
  const { onRunSettled, reportRefreshFailed } = useRunMonitorActions();

  const load = useCallback(async (opts?: { quiet?: boolean }) => {
    if (!opts?.quiet) setLoading(true);
    setError(null);
    try {
      const result = await api.listWatchRoles(WATCH_NEW_ONLY_FILTERS);
      setRoles(result);
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
      setRoles((prev) => prev.filter((item) => item.job.id !== jobId));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to update this role");
    } finally {
      setTriagingId(null);
    }
  }

  const companyCount = new Set(roles.map(({ job }) => job.companyId)).size;

  return (
    <div className="mx-auto max-w-4xl space-y-5">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="font-display text-3xl font-semibold tracking-tight">New Roles</h1>
          <p className="mt-1 text-sm text-[var(--muted)]">
            Review openings found by your watched company boards.
          </p>
        </div>
        {hasLoaded ? (
          <p className="text-sm text-[var(--muted)]">
            {roles.length} {roles.length === 1 ? "role" : "roles"} · {companyCount} {companyCount === 1 ? "company" : "companies"}
          </p>
        ) : null}
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
        <NewRolesInbox
          // The legacy list component only reads fields present in the compact
          // projection; keep the adapter at this boundary until that shared
          // component can be migrated independently.
          roles={roles as unknown as JobListItem[]}
          onTriage={(jobId, action) => void triageRole(jobId, action)}
          isPending={(jobId) => triagingId === jobId}
        />
      ) : null}
    </div>
  );
}
