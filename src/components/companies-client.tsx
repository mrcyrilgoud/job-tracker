"use client";

import { formatDistanceToNow } from "date-fns";
import { useRouter } from "next/navigation";
import { useState } from "react";

import { watchProviders, type WatchProvider } from "@/lib/db/schema";
import { formatLabel } from "@/lib/utils";

type CompanyRow = {
  company: {
    id: string;
    name: string;
    careersUrl: string | null;
  };
  watches: Array<{
    id: string;
    provider: WatchProvider;
    boardSlug: string;
    lastSyncedAt: string | null;
    consecutiveSyncFailures: number;
    lastSyncError: string | null;
  }>;
  reviews: Array<{
    id: string;
    summary: string;
    createdAt: string;
  }>;
};

export function CompaniesClient({ initial }: { initial: CompanyRow[] }) {
  const router = useRouter();
  const [name, setName] = useState("");
  const [careersUrl, setCareersUrl] = useState("");
  const [companyId, setCompanyId] = useState(initial[0]?.company.id ?? "");
  const [provider, setProvider] = useState<WatchProvider>("greenhouse");
  const [boardSlug, setBoardSlug] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function createCompany() {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const response = await fetch("/api/companies", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name, careersUrl: careersUrl || null }),
      });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error ?? "Failed to create company");
      setName("");
      setCareersUrl("");
      setMessage(`Added ${data.company.name}`);
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create company");
    } finally {
      setBusy(false);
    }
  }

  async function createWatch() {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const response = await fetch("/api/watches", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          action: "create_watch",
          companyId,
          provider,
          boardSlug,
        }),
      });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error ?? "Failed to create watch");
      setBoardSlug("");
      setMessage("Board verified and now being watched");
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create watch");
    } finally {
      setBusy(false);
    }
  }

  async function runAction(body: Record<string, unknown>) {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const response = await fetch("/api/watches", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error ?? "Action failed");
      if (body.action === "sync_watch") {
        setMessage(
          data.ok
            ? data.created > 0
              ? `Found ${data.created} new ${data.created === 1 ? "role" : "roles"}`
              : "All caught up — no new roles"
            : `Couldn't sync: ${data.error}`,
        );
      } else {
        setMessage("Done");
      }
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Action failed");
    } finally {
      setBusy(false);
    }
  }

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
            onClick={createCompany}
            disabled={busy || !name.trim()}
            className="btn btn-primary"
          >
            Add company
          </button>
        </div>

        <div className="card space-y-3 p-6">
          <h3 className="font-display text-lg font-medium">Watch a job board</h3>
          <p className="text-sm text-[var(--muted)]">
            We&apos;ll check the board and pull in new roles for you.
          </p>
          <select
            value={companyId}
            onChange={(e) => setCompanyId(e.target.value)}
            className="field"
          >
            {initial.map((row) => (
              <option key={row.company.id} value={row.company.id}>
                {row.company.name}
              </option>
            ))}
          </select>
          <div className="grid grid-cols-2 gap-2">
            <select
              value={provider}
              onChange={(e) => setProvider(e.target.value as WatchProvider)}
              className="field"
            >
              {watchProviders.map((value) => (
                <option key={value} value={value}>
                  {formatLabel(value)}
                </option>
              ))}
            </select>
            <input
              value={boardSlug}
              onChange={(e) => setBoardSlug(e.target.value)}
              placeholder="Board name"
              className="field"
            />
          </div>
          <button
            onClick={createWatch}
            disabled={busy || !companyId || !boardSlug.trim()}
            className="btn btn-secondary"
          >
            Start watching
          </button>
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
        {initial.map((row) => (
          <section key={row.company.id} className="card p-6">
            <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
              <div>
                <h3 className="font-display text-xl font-medium">{row.company.name}</h3>
                <p className="text-sm text-[var(--faint)]">
                  {row.company.careersUrl
                    ? "Careers page connected"
                    : "No careers page set"}
                </p>
              </div>
              {row.company.careersUrl ? (
                <button
                  disabled={busy}
                  onClick={() =>
                    runAction({
                      action: "check_careers",
                      companyId: row.company.id,
                    })
                  }
                  className="btn btn-secondary"
                >
                  Check careers page
                </button>
              ) : null}
            </div>

            {row.reviews.length > 0 ? (
              <div className="mb-4 space-y-2 rounded-xl bg-[var(--amber-soft)] p-4">
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
                      onClick={() =>
                        runAction({ action: "dismiss_review", reviewId: review.id })
                      }
                      className="font-medium underline"
                    >
                      Dismiss
                    </button>
                  </div>
                ))}
              </div>
            ) : null}

            {row.watches.length === 0 ? (
              <p className="text-sm text-[var(--muted)]">
                Not watching any boards yet.
              </p>
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
                            disabled={busy}
                            onClick={() =>
                              runAction({ action: "sync_watch", watchId: watch.id })
                            }
                            className="btn btn-primary"
                          >
                            Sync now
                          </button>
                          <button
                            disabled={busy}
                            onClick={() =>
                              runAction({ action: "delete_watch", watchId: watch.id })
                            }
                            className="btn btn-secondary"
                          >
                            Remove
                          </button>
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </section>
        ))}
      </div>
    </div>
  );
}
