import { useMemo, useState } from "react";
import { Link } from "react-router-dom";

import { ConnectBoardPanel } from "@/components/companies/ConnectBoardPanel";
import { NewRolesList, type TriageAction } from "@/components/companies/NewRolesList";
import { WatchRow } from "@/components/companies/WatchRow";
import { matchesRoleSearch, roleCountLabel } from "@/lib/companies-ui";
import type { CompanyRow, JobListItem } from "@/lib/schema";
import type { Feedback } from "@/lib/use-pending-actions";

const SEARCHABLE_FROM = 8;

/**
 * One company: what it is, what we are watching for it, and what that watching
 * turned up. The card is the whole surface — the detail route renders the same
 * component in `solo` mode rather than maintaining a second, divergent layout.
 */
export function CompanyCard({
  row,
  roles,
  solo = false,
  rolesOpen,
  onToggleRoles,
  onChanged,
  onSync,
  onRemoveWatch,
  onCheckCareers,
  onDismissReview,
  onTriage,
  isPending,
  feedback,
}: {
  row: CompanyRow;
  roles: JobListItem[];
  /** Rendered as the whole page for one company rather than one card in a list. */
  solo?: boolean;
  rolesOpen: boolean;
  onToggleRoles: () => void;
  onChanged: () => void;
  onSync: (watchId: string) => void;
  onRemoveWatch: (watchId: string) => void;
  onCheckCareers: (companyId: string) => void;
  onDismissReview: (reviewId: string) => void;
  onTriage: (jobId: string, action: TriageAction) => void;
  isPending: (key: string) => boolean;
  feedback: Readonly<Record<string, Feedback>>;
}) {
  const [connecting, setConnecting] = useState(false);
  const [search, setSearch] = useState("");

  const company = row.company;
  const matching = useMemo(
    () => roles.filter((role) => matchesRoleSearch(role, search)),
    [roles, search],
  );
  const careersFeedback = feedback[`careers:${company.id}`];

  const heading = solo ? (
    <h1 className="font-display text-3xl font-semibold tracking-tight">{company.name}</h1>
  ) : (
    <h2 className="font-display text-xl font-medium">
      <Link
        to={`/companies/${company.id}`}
        className="text-[var(--foreground)] underline-offset-4 hover:text-[var(--accent)] hover:underline"
      >
        {company.name}
      </Link>
    </h2>
  );

  return (
    <section className={solo ? "space-y-4" : "card space-y-4 p-6"}>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          {heading}
          {company.careersUrl ? (
            <a
              href={company.careersUrl}
              target="_blank"
              rel="noreferrer"
              className="mt-0.5 inline-block text-xs text-[var(--muted)] hover:text-[var(--accent)] hover:underline"
            >
              Careers page
            </a>
          ) : null}
        </div>

        {roles.length > 0 ? (
          <button
            type="button"
            onClick={onToggleRoles}
            aria-expanded={rolesOpen}
            className="pill bg-[var(--accent-soft)] text-[var(--accent-ink)] transition-colors hover:bg-[var(--accent)] hover:text-white"
          >
            {roleCountLabel(roles.length)}
            <span aria-hidden>{rolesOpen ? "▴" : "▾"}</span>
          </button>
        ) : null}
      </div>

      {row.reviews.length > 0 ? (
        <div className="space-y-2 rounded-xl bg-[var(--amber-soft)] p-4">
          {row.reviews.map((review) => (
            <div
              key={review.id}
              className="flex flex-wrap items-center justify-between gap-2 text-sm text-[var(--amber-ink)]"
            >
              <span>{company.name}&apos;s careers page changed since we last looked.</span>
              <button
                type="button"
                className="font-medium underline underline-offset-2"
                disabled={isPending(`review:${review.id}`)}
                onClick={() => onDismissReview(review.id)}
              >
                Dismiss
              </button>
            </div>
          ))}
        </div>
      ) : null}

      {row.watches.length > 0 ? (
        <div className="space-y-2">
          {row.watches.map((watch) => (
            <WatchRow
              key={watch.id}
              watch={watch}
              syncing={isPending(`sync:${watch.id}`)}
              removing={isPending(`remove:${watch.id}`)}
              feedback={feedback[`sync:${watch.id}`] ?? feedback[`remove:${watch.id}`]}
              onSync={() => onSync(watch.id)}
              onRemove={() => onRemoveWatch(watch.id)}
            />
          ))}
        </div>
      ) : !connecting ? (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-[var(--surface-muted)] px-4 py-3">
          <p className="text-sm text-[var(--muted)]">
            No job board connected yet, so new roles won&apos;t appear on their own.
          </p>
          <button
            type="button"
            className="btn btn-primary btn-sm"
            onClick={() => setConnecting(true)}
          >
            Connect job board
          </button>
        </div>
      ) : null}

      {connecting ? (
        <div className="space-y-2">
          <ConnectBoardPanel
            row={row}
            onChanged={onChanged}
            onDone={() => setConnecting(false)}
          />
          <button
            type="button"
            className="btn btn-ghost btn-sm px-0"
            onClick={() => setConnecting(false)}
          >
            Cancel
          </button>
        </div>
      ) : null}

      {rolesOpen && roles.length > 0 ? (
        <div className="rounded-xl bg-[var(--surface-muted)] p-4">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-sm font-medium">
              {roleCountLabel(roles.length)} we found that you aren&apos;t tracking
            </p>
            <Link
              to={`/?companyId=${company.id}`}
              className="text-xs text-[var(--accent)] hover:underline"
            >
              See what you already track →
            </Link>
          </div>
          {roles.length >= SEARCHABLE_FROM ? (
            <input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder={`Search ${company.name} roles…`}
              aria-label={`Search new roles at ${company.name}`}
              className="field mt-3 border-transparent bg-[var(--surface)]"
            />
          ) : null}
          <div className="mt-3">
            {matching.length === 0 ? (
              <p className="text-sm text-[var(--muted)]">
                No new roles match “{search.trim()}”.
              </p>
            ) : (
              <NewRolesList
                roles={matching}
                showCompany={false}
                onTriage={onTriage}
                isPending={(jobId) => isPending(`triage:${jobId}`)}
              />
            )}
          </div>
        </div>
      ) : null}

      <div className="flex flex-wrap items-center gap-2">
        {row.watches.length > 0 && !connecting ? (
          <button
            type="button"
            className="btn btn-ghost btn-sm px-0"
            onClick={() => setConnecting(true)}
          >
            Connect another board
          </button>
        ) : null}
        {company.careersUrl ? (
          <button
            type="button"
            className="btn btn-ghost btn-sm px-0"
            disabled={isPending(`careers:${company.id}`)}
            onClick={() => onCheckCareers(company.id)}
          >
            {isPending(`careers:${company.id}`) ? <span className="spinner" aria-hidden /> : null}
            Check careers page
          </button>
        ) : null}
      </div>

      {careersFeedback ? (
        <p
          className={`text-sm ${
            careersFeedback.tone === "positive"
              ? "text-[var(--green-ink)]"
              : "text-[var(--danger)]"
          }`}
          role={careersFeedback.tone === "negative" ? "alert" : undefined}
        >
          {careersFeedback.text}
        </p>
      ) : null}
    </section>
  );
}
