import { useCallback, useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";

import { CompanyCard } from "@/components/companies/CompanyCard";
import { api } from "@/lib/api";
import type { CompanyRow, JobListItem } from "@/lib/schema";
import { useCompanyActions } from "@/lib/use-company-actions";

/**
 * One company on its own. Renders the same card as the Companies list in `solo`
 * mode rather than maintaining a second layout — the previous version drifted
 * into a read-only view with no way to sync or remove a watch from here.
 */
export function CompanyDetailPage() {
  const { id } = useParams();
  const [row, setRow] = useState<CompanyRow | null>(null);
  const [newRoles, setNewRoles] = useState<JobListItem[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!id) return;
    setError(null);
    try {
      const [companyResult, roleResult] = await Promise.all([
        api.listCompanies(),
        api.listJobs({ companyId: id, newFromWatch: true }),
      ]);
      setRow(companyResult.companies.find((candidate) => candidate.company.id === id) ?? null);
      setNewRoles(roleResult.jobs);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load company");
    } finally {
      setLoaded(true);
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  const actions = useCompanyActions(() => void load());

  // On a single-company page there is nothing competing for the space, so show
  // what the watch found rather than making the user open it.
  const { toggleRoles } = actions;
  const companyId = row?.company.id;
  const hasRoles = newRoles.length > 0;
  useEffect(() => {
    if (companyId && hasRoles) toggleRoles(companyId, true);
  }, [companyId, hasRoles, toggleRoles]);

  if (!loaded) return <p className="text-sm text-[var(--muted)]">Loading company…</p>;
  if (error) {
    return (
      <p className="text-sm text-[var(--danger)]" role="alert">
        {error}
      </p>
    );
  }
  if (!row) {
    return (
      <div className="space-y-3">
        <p className="text-sm text-[var(--muted)]">We couldn&apos;t find that company.</p>
        <Link to="/companies" className="text-sm text-[var(--accent)] hover:underline">
          ← All companies
        </Link>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <Link
        to="/companies"
        className="inline-block text-sm text-[var(--muted)] hover:text-[var(--accent)]"
      >
        ← All companies
      </Link>

      <CompanyCard
        row={row}
        roles={newRoles}
        solo
        rolesOpen={actions.openRoles.has(row.company.id)}
        onToggleRoles={() => actions.toggleRoles(row.company.id)}
        onChanged={() => void load()}
        onSync={(watchId) => {
          const watch = row.watches.find((candidate) => candidate.id === watchId);
          actions.syncWatch(watchId, row.company.id, watch?.provider ?? "");
        }}
        onRemoveWatch={actions.removeWatch}
        onCheckCareers={actions.checkCareers}
        onDismissReview={actions.dismissReview}
        onTriage={actions.triageRole}
        isPending={actions.isPending}
        feedback={actions.feedback}
      />

      <Link
        to={`/?companyId=${row.company.id}`}
        className="inline-block text-sm text-[var(--accent)] hover:underline"
      >
        See the {row.company.name} roles you already track →
      </Link>
    </div>
  );
}
