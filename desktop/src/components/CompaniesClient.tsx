import { formatDistanceToNow } from "date-fns";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

import { CompanyWatchAutomation } from "@/components/CompanyWatchAutomation";
import { api } from "@/lib/api";
import type { CompanyRow } from "@/lib/schema";
import { formatLabel } from "@/lib/utils";

export function CompaniesClient({
  initial,
  onChanged,
}: {
  initial: CompanyRow[];
  onChanged: () => void;
}) {
  const [name, setName] = useState("");
  const [careersUrl, setCareersUrl] = useState("");
  const [companyId, setCompanyId] = useState(initial[0]?.company.id ?? "");
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!initial.some((row) => row.company.id === companyId)) {
      setCompanyId(initial[0]?.company.id ?? "");
    }
  }, [companyId, initial]);

  async function createCompany() {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const result = await api.createCompany(name, careersUrl || null);
      setName("");
      setCareersUrl("");
      setCompanyId(result.company.id);
      setMessage(`Added ${result.company.name}`);
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create company");
    } finally {
      setBusy(false);
    }
  }

  async function syncWatch(watchId: string) {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const data = await api.syncWatch(watchId);
      setMessage(
        data.ok
          ? data.created > 0
            ? `Found ${data.created} new ${data.created === 1 ? "role" : "roles"}`
            : "All caught up — no new roles"
          : `Couldn't sync: ${data.error}`,
      );
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Action failed");
    } finally {
      setBusy(false);
    }
  }

  async function checkCareers(companyIdToCheck: string) {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await api.checkCareers(companyIdToCheck);
      setMessage("Done");
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Action failed");
    } finally {
      setBusy(false);
    }
  }

  async function dismissReview(reviewId: string) {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await api.dismissReview(reviewId);
      setMessage("Done");
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Action failed");
    } finally {
      setBusy(false);
    }
  }

  async function removeWatch(watchId: string) {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await api.deleteWatch(watchId);
      setMessage("Done");
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Action failed");
    } finally {
      setBusy(false);
    }
  }

  const selectedRow = initial.find((row) => row.company.id === companyId) ?? null;

  return (
    <div className="space-y-6">
      <div className="grid gap-4 lg:grid-cols-2">
        <div className="card space-y-3 p-6">
          <h3 className="font-display text-lg font-medium">Add a company</h3>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Company name"
            className="field"
          />
          <input
            value={careersUrl}
            onChange={(e) => setCareersUrl(e.target.value)}
            placeholder="Careers page link (optional)"
            className="field"
          />
          <button
            type="button"
            onClick={() => void createCompany()}
            disabled={busy || !name.trim()}
            className="btn btn-primary"
          >
            Add company
          </button>
        </div>

        <div className="card space-y-3 p-6">
          <h3 className="font-display text-lg font-medium">Detect job board</h3>
          <p className="text-sm text-[var(--muted)]">
            Choose a company. We&apos;ll offer tracked posting links when available, or paste any
            posting URL to detect the ATS board to watch.
          </p>
          <label className="block space-y-1.5 text-sm">
            <span className="font-medium">Company</span>
            <select
              value={companyId}
              onChange={(e) => setCompanyId(e.target.value)}
              className="field"
              disabled={initial.length === 0}
            >
              {initial.length === 0 ? (
                <option value="">Add a company first</option>
              ) : (
                initial.map((row) => (
                  <option key={row.company.id} value={row.company.id}>
                    {row.company.name}
                  </option>
                ))
              )}
            </select>
          </label>
          {selectedRow ? (
            <CompanyWatchAutomation row={selectedRow} onChanged={onChanged} />
          ) : (
            <p className="rounded-xl bg-[var(--surface-muted)] px-3.5 py-3 text-sm text-[var(--muted)]">
              Add a company on the left, then paste a job posting URL here to detect its board.
            </p>
          )}
        </div>
      </div>

      {message ? (
        <p className="rounded-xl bg-[var(--green-soft)] px-3.5 py-2.5 text-sm text-[var(--green-ink)]">
          {message}
        </p>
      ) : null}
      {error ? (
        <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
          {error}
        </p>
      ) : null}

      <div className="space-y-4">
        {initial.length === 0 ? (
          <p className="text-sm text-[var(--muted)]">
            No companies yet. Add one above to start detecting job boards.
          </p>
        ) : null}
        {initial.map((row) => (
          <section key={row.company.id} className="card space-y-4 p-6">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div>
                <Link
                  to={`/companies/${row.company.id}`}
                  className="font-display text-xl font-medium text-[var(--accent)] underline-offset-4 hover:underline"
                >
                  {row.company.name}
                </Link>
                <p className="mt-0.5 text-sm text-[var(--muted)]">
                  <Link
                    to={`/companies/${row.company.id}`}
                    className="text-[var(--accent)] hover:underline"
                  >
                    Open company · manage job board →
                  </Link>
                  <span className="text-[var(--faint)]">
                    {" "}
                    ·{" "}
                    {row.company.careersUrl
                      ? "Careers page connected"
                      : "No careers page set"}
                  </span>
                </p>
              </div>
              {row.company.careersUrl ? (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void checkCareers(row.company.id)}
                  className="btn btn-secondary"
                >
                  Check careers page
                </button>
              ) : null}
            </div>

            {row.reviews.length > 0 ? (
              <div className="space-y-2 rounded-xl bg-[var(--amber-soft)] p-4">
                <p className="text-sm font-medium text-[var(--amber-ink)]">
                  This careers page changed
                </p>
                {row.reviews.map((review) => (
                  <div
                    key={review.id}
                    className="flex flex-wrap items-center justify-between gap-2 text-sm text-[var(--amber-ink)]"
                  >
                    <span>{review.summary}</span>
                    <button
                      type="button"
                      onClick={() => void dismissReview(review.id)}
                      className="font-medium underline"
                    >
                      Dismiss
                    </button>
                  </div>
                ))}
              </div>
            ) : null}

            {row.watches.length === 0 ? (
              <CompanyWatchAutomation row={row} onChanged={onChanged} compact />
            ) : (
              <div className="space-y-3">
                {row.watches.map((watch) => {
                  const healthy = watch.consecutiveSyncFailures === 0;
                  return (
                    <div
                      key={watch.id}
                      className="rounded-xl bg-[var(--surface-muted)] p-4"
                    >
                      <div className="flex flex-wrap items-start justify-between gap-2">
                        <div>
                          <p className="flex items-center gap-2 font-medium">
                            {formatLabel(watch.provider)}
                            <span className="text-[var(--faint)]">·</span>
                            <span className="font-normal text-[var(--muted)]">
                              {watch.boardSlug}
                            </span>
                          </p>
                          <p className="mt-0.5 flex items-center gap-1.5 text-sm text-[var(--muted)]">
                            <span
                              className={`inline-block h-2 w-2 rounded-full ${
                                healthy
                                  ? "bg-[var(--green-ink)]"
                                  : "bg-[var(--danger)]"
                              }`}
                            />
                            {healthy ? "Syncing automatically" : "Needs attention"}
                            <span className="text-[var(--faint)]">
                              ·{" "}
                              {watch.lastSyncedAt
                                ? `synced ${formatDistanceToNow(
                                    new Date(watch.lastSyncedAt),
                                    { addSuffix: true },
                                  )}`
                                : "not synced yet"}
                            </span>
                          </p>
                          {watch.lastSyncError ? (
                            <p className="mt-1 text-sm text-[var(--danger)]">
                              {watch.lastSyncError}
                            </p>
                          ) : null}
                        </div>
                        <div className="flex gap-2">
                          <button
                            type="button"
                            disabled={busy}
                            onClick={() => void syncWatch(watch.id)}
                            className="btn btn-primary"
                          >
                            Sync now
                          </button>
                          <button
                            type="button"
                            disabled={busy}
                            onClick={() => void removeWatch(watch.id)}
                            className="btn btn-secondary"
                          >
                            Remove
                          </button>
                        </div>
                      </div>
                    </div>
                  );
                })}
                <details className="rounded-xl border border-[var(--border)] bg-[var(--surface)] p-3">
                  <summary className="cursor-pointer text-sm font-medium text-[var(--accent)]">
                    Detect another job board
                  </summary>
                  <div className="mt-3">
                    <CompanyWatchAutomation row={row} onChanged={onChanged} compact />
                  </div>
                </details>
              </div>
            )}
          </section>
        ))}
      </div>
    </div>
  );
}
