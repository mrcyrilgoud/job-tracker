import { formatDistanceToNow } from "date-fns";
import { useCallback, useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";

import { CompanyWatchAutomation } from "@/components/CompanyWatchAutomation";
import { api } from "@/lib/api";
import type { CompanyRow } from "@/lib/schema";
import { formatLabel } from "@/lib/utils";

export function CompanyDetailPage() {
  const { id } = useParams();
  const [row, setRow] = useState<CompanyRow | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!id) return;
    setLoading(true);
    setError(null);
    try {
      const result = await api.listCompanies();
      setRow(result.companies.find((candidate) => candidate.company.id === id) ?? null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load company");
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  if (loading) return <p className="text-sm text-[var(--muted)]">Loading company…</p>;
  if (error) return <p className="text-sm text-[var(--danger)]">{error}</p>;
  if (!row) {
    return (
      <div className="space-y-3">
        <p className="text-sm text-[var(--muted)]">Company not found.</p>
        <Link to="/companies" className="text-sm text-[var(--accent)] hover:underline">← Companies</Link>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <Link to="/companies" className="text-sm text-[var(--muted)] hover:text-[var(--accent)]">← Companies</Link>
      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">{row.company.name}</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Detect this company&apos;s job board from a posting URL, then watch it for new roles.
        </p>
      </div>

      <section className="space-y-3">
        <div>
          <h2 className="font-display text-xl font-medium">Job board automation</h2>
          <p className="mt-1 text-sm text-[var(--muted)]">
            Use a tracked posting from {row.company.name}, or paste another link, and we&apos;ll
            find the ATS board to watch.
          </p>
        </div>
        <CompanyWatchAutomation row={row} onChanged={() => void load()} />
      </section>

      {row.watches.length > 0 ? (
        <section className="space-y-3">
          <h2 className="font-display text-xl font-medium">Watching</h2>
          <div className="space-y-3">
            {row.watches.map((watch) => (
              <div key={watch.id} className="rounded-xl bg-[var(--surface-muted)] p-4">
                <p className="font-medium">
                  {formatLabel(watch.provider)} · {watch.boardSlug}
                </p>
                <p className="mt-0.5 text-sm text-[var(--muted)]">
                  {watch.consecutiveSyncFailures === 0
                    ? "Syncing automatically"
                    : "Needs attention"}
                  {watch.lastSyncedAt
                    ? ` · synced ${formatDistanceToNow(new Date(watch.lastSyncedAt), { addSuffix: true })}`
                    : " · not synced yet"}
                </p>
                {watch.lastSyncError ? (
                  <p className="mt-1 text-sm text-[var(--danger)]">{watch.lastSyncError}</p>
                ) : null}
              </div>
            ))}
          </div>
        </section>
      ) : null}

      <Link to={`/?companyId=${row.company.id}`} className="inline-block text-sm text-[var(--accent)] hover:underline">
        View tracked jobs for {row.company.name} →
      </Link>
    </div>
  );
}
