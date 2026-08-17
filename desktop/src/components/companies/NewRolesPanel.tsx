import { useMemo, useState } from "react";

import { NewRolesList, type TriageAction } from "@/components/companies/NewRolesList";
import { matchesRoleSearch } from "@/lib/companies-ui";
import type { JobListItem } from "@/lib/schema";

const COLLAPSED_COUNT = 6;

/**
 * The payoff of watching a board, and the anchor of the Companies screen: every
 * role the watches turned up that is not on the board yet, across all companies.
 * Previously these were reachable only from the Jobs tab, capped at five.
 */
export function NewRolesPanel({
  roles,
  onTriage,
  isPending,
}: {
  roles: JobListItem[];
  onTriage: (jobId: string, action: TriageAction) => void;
  isPending: (jobId: string) => boolean;
}) {
  const [search, setSearch] = useState("");
  const [expanded, setExpanded] = useState(false);
  const [open, setOpen] = useState(true);

  const matching = useMemo(
    () => roles.filter((role) => matchesRoleSearch(role, search)),
    [roles, search],
  );
  const visible = expanded ? matching : matching.slice(0, COLLAPSED_COUNT);
  const hiddenCount = matching.length - visible.length;

  return (
    <section className="card p-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <p className="font-display text-3xl font-medium leading-none text-[var(--accent)]">
            {roles.length}
          </p>
          <h2 className="mt-1.5 font-display text-lg font-medium">
            new {roles.length === 1 ? "role" : "roles"} from your watches
          </h2>
          <p className="mt-0.5 text-sm text-[var(--muted)]">
            {roles.length > 0
              ? "Not on your list yet. Save the ones worth tracking."
              : "Your watches are up to date. No new matches found."}
          </p>
        </div>
        {roles.length > 0 ? (
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={() => setOpen((value) => !value)}
            aria-expanded={open}
          >
            {open ? "Hide" : "Show"}
          </button>
        ) : null}
      </div>

      {open && roles.length > 0 ? (
        <>
          {roles.length > COLLAPSED_COUNT ? (
            <input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search these roles…"
              aria-label="Search new roles"
              className="field mt-4 border-transparent bg-[var(--surface-muted)]"
            />
          ) : null}

          <div className="mt-4">
            {matching.length === 0 ? (
              <p className="text-sm text-[var(--muted)]">
                No new roles match “{search.trim()}”.
              </p>
            ) : (
              <NewRolesList
                roles={visible}
                showCompany
                onTriage={onTriage}
                isPending={isPending}
              />
            )}
          </div>

          {hiddenCount > 0 ? (
            <button
              type="button"
              className="btn btn-ghost btn-sm mt-2 px-0"
              onClick={() => setExpanded(true)}
            >
              Show {hiddenCount} more
            </button>
          ) : null}
        </>
      ) : null}
    </section>
  );
}
