import { useCallback, useEffect, useState } from "react";

import { CompaniesClient } from "@/components/CompaniesClient";
import { api } from "@/lib/api";
import type { CompanyRow } from "@/lib/schema";

export function CompaniesPage() {
  const [companies, setCompanies] = useState<CompanyRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.listCompanies();
      setCompanies(result.companies);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load companies");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">Companies</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Follow a company&apos;s job board and we&apos;ll surface new roles automatically. Careers
          pages only ping you when something actually changes.
        </p>
      </div>

      {loading ? <p className="text-sm text-[var(--muted)]">Loading companies…</p> : null}
      {error ? (
        <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
          {error}
        </p>
      ) : null}

      {!loading ? <CompaniesClient initial={companies} onChanged={() => void load()} /> : null}
    </div>
  );
}
