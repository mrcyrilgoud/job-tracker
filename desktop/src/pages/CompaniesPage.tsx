import { useCallback, useEffect, useState } from "react";

import { CompaniesClient } from "@/components/CompaniesClient";
import { api } from "@/lib/api";
import { useRunMonitorActions } from "@/lib/RunMonitorContext";
import type { CompanyRow, JobListItem } from "@/lib/schema";

export function CompaniesPage() {
  const [companies, setCompanies] = useState<CompanyRow[]>([]);
  const [newRoles, setNewRoles] = useState<JobListItem[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { onRunSettled, reportRefreshFailed } = useRunMonitorActions();

  const load = useCallback(async (opts?: { quiet?: boolean }) => {
    setError(null);
    try {
      // Roles a watch found but the user hasn't triaged live in `jobs` behind
      // the is_new_from_watch flag, so they need a second read alongside the
      // company rows.
      const [companyResult, roleResult] = await Promise.all([
        api.listCompanies(),
        api.listJobs({ newFromWatch: true }),
      ]);
      setCompanies(companyResult.companies);
      setNewRoles(roleResult.jobs);
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to load companies";
      setError(message);
      if (opts?.quiet) reportRefreshFailed(message);
    } finally {
      setLoaded(true);
    }
  }, [reportRefreshFailed]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(
    () => onRunSettled(() => { void load({ quiet: true }); }),
    [load, onRunSettled],
  );

  return (
    <div className="mx-auto max-w-4xl">
      {error ? (
        <p
          role="alert"
          className="mb-6 rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]"
        >
          {error}
        </p>
      ) : null}

      {/* Reloads after an action keep the current view rather than blanking it. */}
      {loaded ? (
        <CompaniesClient
          companies={companies}
          newRoles={newRoles}
          onChanged={() => void load()}
        />
      ) : (
        <p className="text-sm text-[var(--muted)]">Loading companies…</p>
      )}
    </div>
  );
}
