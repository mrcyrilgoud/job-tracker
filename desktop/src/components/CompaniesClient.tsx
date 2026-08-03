import { useMemo, useState } from "react";

import { AddCompanyForm } from "@/components/companies/AddCompanyForm";
import { CompanyCard } from "@/components/companies/CompanyCard";
import { NewRolesPanel } from "@/components/companies/NewRolesPanel";
import { groupRolesByCompany } from "@/lib/companies-ui";
import type { CompanyRow, JobListItem } from "@/lib/schema";
import { useCompanyActions } from "@/lib/use-company-actions";

type CompanyFilter = "all" | "roles" | "attention" | "unconnected";

const filters: Array<{ key: CompanyFilter; label: string }> = [
  { key: "all", label: "All" },
  { key: "roles", label: "New roles" },
  { key: "attention", label: "Needs attention" },
  { key: "unconnected", label: "No board yet" },
];

const SEARCHABLE_FROM = 6;

export function CompaniesClient({
  companies,
  newRoles,
  onChanged,
}: {
  companies: CompanyRow[];
  /** Roles a watch found that are not on the user's list yet. */
  newRoles: JobListItem[];
  onChanged: () => void;
}) {
  const [adding, setAdding] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<CompanyFilter>("all");
  const actions = useCompanyActions(onChanged);

  const rolesByCompany = useMemo(() => groupRolesByCompany(newRoles), [newRoles]);

  const visible = useMemo(() => {
    const query = search.trim().toLowerCase();
    return companies
      .filter((row) => (query ? row.company.name.toLowerCase().includes(query) : true))
      .filter((row) => {
        switch (filter) {
          case "roles":
            return (rolesByCompany.get(row.company.id)?.length ?? 0) > 0;
          case "attention":
            return row.watches.some((watch) => watch.consecutiveSyncFailures > 0);
          case "unconnected":
            return row.watches.length === 0;
          default:
            return true;
        }
      })
      .slice()
      .sort((a, b) => a.company.name.localeCompare(b.company.name));
  }, [companies, filter, rolesByCompany, search]);

  const watchedBoards = companies.reduce((total, row) => total + row.watches.length, 0);

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="font-display text-3xl font-semibold tracking-tight">Companies</h1>
          <p className="mt-1 text-sm text-[var(--muted)]">
            {watchedBoards === 0
              ? "Connect a company's job board and new roles will show up here on their own."
              : `Watching ${watchedBoards} ${
                  watchedBoards === 1 ? "board" : "boards"
                } across ${companies.length} ${
                  companies.length === 1 ? "company" : "companies"
                }.`}
          </p>
        </div>
        {!adding ? (
          <button type="button" className="btn btn-primary" onClick={() => setAdding(true)}>
            Add company
          </button>
        ) : null}
      </div>

      {adding ? (
        <AddCompanyForm
          onAdded={(companyName) => {
            setAdding(false);
            setNotice(`Added ${companyName}. Connect its job board to start finding roles.`);
            onChanged();
          }}
          onCancel={() => setAdding(false)}
        />
      ) : null}

      {notice ? (
        <p className="rounded-xl bg-[var(--green-soft)] px-3.5 py-2.5 text-sm text-[var(--green-ink)]">
          {notice}
        </p>
      ) : null}

      {newRoles.length > 0 ? (
        <NewRolesPanel
          roles={newRoles}
          onTriage={actions.triageRole}
          isPending={(jobId) => actions.isPending(`triage:${jobId}`)}
        />
      ) : null}

      {companies.length >= SEARCHABLE_FROM ? (
        <div className="flex flex-wrap items-center gap-2">
          <input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search companies…"
            aria-label="Search companies"
            className="field min-w-[200px] flex-1 border-transparent bg-[var(--surface-muted)]"
          />
          <div className="flex flex-wrap gap-1">
            {filters.map(({ key, label }) => (
              <button
                key={key}
                type="button"
                onClick={() => setFilter(key)}
                aria-pressed={filter === key}
                className={`rounded-full px-3 py-1.5 text-sm font-medium transition-colors ${
                  filter === key
                    ? "bg-[var(--accent-soft)] text-[var(--accent-ink)]"
                    : "text-[var(--muted)] hover:bg-[var(--surface-muted)] hover:text-[var(--foreground)]"
                }`}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
      ) : null}

      {companies.length === 0 ? (
        <div className="card flex flex-col items-center gap-3 px-6 py-16 text-center">
          <p className="font-display text-lg">No companies yet</p>
          <p className="max-w-sm text-sm text-[var(--muted)]">
            Add a company and connect its job board — we&apos;ll watch it and tell you when
            something new opens up.
          </p>
          <button type="button" className="btn btn-primary mt-1" onClick={() => setAdding(true)}>
            Add your first company
          </button>
        </div>
      ) : visible.length === 0 ? (
        <p className="text-sm text-[var(--muted)]">No companies match that.</p>
      ) : (
        <div className="space-y-4">
          {visible.map((row) => (
            <CompanyCard
              key={row.company.id}
              row={row}
              roles={rolesByCompany.get(row.company.id) ?? []}
              rolesOpen={actions.openRoles.has(row.company.id)}
              onToggleRoles={() => actions.toggleRoles(row.company.id)}
              onChanged={onChanged}
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
          ))}
        </div>
      )}
    </div>
  );
}
