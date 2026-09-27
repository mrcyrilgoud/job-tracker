import { save } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";

import { FilterCriteriaEditor } from "@/components/companies/FilterCriteriaEditor";
import { api, type CsvConfig, type CsvPathStatus, type FilterCriteria } from "@/lib/api";

/** Match-all default so the editor always has a well-formed criteria to edit. */
const MATCH_ALL_CRITERIA: FilterCriteria = {
  version: 1,
  title: { include: [], exclude: [], matchMode: "word" },
  location: { country: null, include: [], exclude: [], matchMode: "word" },
  remote: "any",
};

/** Normalized status for a single saveable setting. */
type SaveStatus =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "saved" }
  | { kind: "error"; message: string };

const IDLE: SaveStatus = { kind: "idle" };

function errorMessage(err: unknown, fallback: string): string {
  return err instanceof Error ? err.message : fallback;
}

/** Shared save-state indicator so every card reports progress the same way. */
function SaveState({ status }: { status: SaveStatus }) {
  if (status.kind === "saving") {
    return (
      <span className="save-state text-[var(--muted)]">
        <span className="spinner" aria-hidden="true" />
        Saving…
      </span>
    );
  }
  if (status.kind === "saved") {
    return (
      <span className="save-state text-[var(--green-ink)]" role="status">
        <span className="save-state-dot" aria-hidden="true" />
        Saved
      </span>
    );
  }
  return null;
}

/** A group of related settings, e.g. "Search filters". */
function SettingsGroup({
  id,
  title,
  description,
  children,
}: {
  id: string;
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <section aria-labelledby={id} className="space-y-4">
      <div className="px-1">
        <h2 id={id} className="font-display text-lg font-semibold tracking-tight">
          {title}
        </h2>
        <p className="mt-0.5 text-sm text-[var(--muted)]">{description}</p>
      </div>
      {children}
    </section>
  );
}

/** A single setting card with a scoped error slot and optional footer. */
function SettingCard({
  id,
  title,
  description,
  badge,
  error,
  footer,
  children,
}: {
  id: string;
  title: string;
  description?: React.ReactNode;
  badge?: React.ReactNode;
  error?: string | null;
  footer?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <section className="card overflow-hidden" aria-labelledby={id}>
      <div className="p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h3 id={id} className="font-display text-base font-semibold">
              {title}
            </h3>
            {description ? (
              <p className="mt-1 text-sm text-[var(--muted)]">{description}</p>
            ) : null}
          </div>
          {badge ?? null}
        </div>
        {children}
        {error ? (
          <p
            role="alert"
            className="mt-4 rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]"
          >
            {error}
          </p>
        ) : null}
      </div>
      {footer ? <div className="card-footer">{footer}</div> : null}
    </section>
  );
}

/** De-emphasized, expandable helper text — keeps the primary control uncluttered. */
function Hint({ summary, children }: { summary: string; children: React.ReactNode }) {
  return (
    <details className="mt-3 text-xs text-[var(--faint)]">
      <summary className="cursor-pointer select-none font-medium text-[var(--muted)] hover:text-[var(--foreground)]">
        {summary}
      </summary>
      <p className="mt-2 leading-relaxed">{children}</p>
    </details>
  );
}

export function SettingsPage() {
  const [config, setConfig] = useState<CsvConfig | null>(null);
  const [pendingPath, setPendingPath] = useState<CsvPathStatus | null>(null);
  const [criteria, setCriteria] = useState<FilterCriteria>(MATCH_ALL_CRITERIA);

  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [csvStatus, setCsvStatus] = useState<SaveStatus>(IDLE);
  const [filterStatus, setFilterStatus] = useState<SaveStatus>(IDLE);

  const csvBusy = csvStatus.kind === "saving";

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const [csv, filterCriteria] = await Promise.all([
        api.csvConfig(),
        api.getFilterCriteria(),
      ]);
      setConfig(csv);
      setCriteria(filterCriteria);
    } catch (err) {
      setLoadError(errorMessage(err, "Failed to load settings"));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  function flashSaved(setStatus: (status: SaveStatus) => void) {
    setStatus({ kind: "saved" });
    setTimeout(() => setStatus(IDLE), 1500);
  }

  async function choosePath() {
    if (!config) return;
    setCsvStatus(IDLE);
    const selected = await save({
      defaultPath: config.path,
      filters: [{ name: "CSV files", extensions: ["csv"] }],
    });
    if (!selected) return;

    setCsvStatus({ kind: "saving" });
    try {
      const status = await api.csvPathStatus(selected);
      setPendingPath(status);
      setCsvStatus(IDLE);
    } catch (err) {
      setCsvStatus({ kind: "error", message: errorMessage(err, "Could not use that CSV location") });
    }
  }

  async function confirmPath(mode: "import" | "replace") {
    if (!pendingPath) return;
    setCsvStatus({ kind: "saving" });
    try {
      setConfig(await api.configureCsv(pendingPath.path, mode));
      setPendingPath(null);
      flashSaved(setCsvStatus);
    } catch (err) {
      setCsvStatus({ kind: "error", message: errorMessage(err, "Could not update the CSV location") });
    }
  }

  async function useDefault() {
    setCsvStatus({ kind: "saving" });
    try {
      setConfig(await api.resetCsvConfig());
      flashSaved(setCsvStatus);
    } catch (err) {
      setCsvStatus({ kind: "error", message: errorMessage(err, "Could not restore the default location") });
    }
  }

  async function saveFilter() {
    setFilterStatus({ kind: "saving" });
    try {
      await api.setFilterCriteria(criteria);
      flashSaved(setFilterStatus);
    } catch (err) {
      setFilterStatus({ kind: "error", message: errorMessage(err, "Failed to save filter") });
    }
  }

  return (
    <div className="mx-auto max-w-3xl space-y-8">
      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">Settings</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Tune what shows up in your feed and where your data lives. Everything stays local to Job Tracker.
        </p>
      </div>

      {loading ? <p className="text-sm text-[var(--muted)]">Loading settings…</p> : null}
      {loadError ? (
        <p role="alert" className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
          {loadError}
        </p>
      ) : null}

      {!loading && !loadError ? (
        <>
          <SettingsGroup
            id="group-search-filters"
            title="Search filters"
            description="Control which new roles surface from your watches."
          >
            <SettingCard
              id="filter-heading"
              title="Filter new roles"
              description="Choose which new roles surface from your watches by title, location, and remote preference. Leave everything empty to show all."
              error={filterStatus.kind === "error" ? filterStatus.message : null}
              footer={
                <>
                  <button
                    type="button"
                    className="btn-primary btn-sm"
                    disabled={filterStatus.kind === "saving"}
                    onClick={() => void saveFilter()}
                  >
                    {filterStatus.kind === "saving" ? "Saving…" : "Save"}
                  </button>
                  <SaveState status={filterStatus} />
                </>
              }
            >
              <div className="mt-4">
                <FilterCriteriaEditor value={criteria} onChange={setCriteria} />
              </div>
            </SettingCard>
          </SettingsGroup>

          {config ? (
            <SettingsGroup
              id="group-data-storage"
              title="Data & storage"
              description="Where your editable data is kept on disk."
            >
              <SettingCard
                id="csv-location-heading"
                title="Jobs CSV location"
                description="Job Tracker synchronizes this file after edits and scheduled jobs."
                badge={
                  <span className="rounded-full bg-[var(--surface-muted)] px-2.5 py-1 text-xs font-medium text-[var(--muted)]">
                    {config.isCustom ? "Custom location" : "Default location"}
                  </span>
                }
                error={csvStatus.kind === "error" ? csvStatus.message : null}
                footer={
                  <>
                    <button type="button" className="btn-primary btn-sm" disabled={csvBusy} onClick={() => void choosePath()}>
                      {csvBusy ? "Updating…" : "Choose CSV…"}
                    </button>
                    {config.isCustom ? (
                      <button type="button" className="btn-secondary btn-sm" disabled={csvBusy} onClick={() => void useDefault()}>
                        Use default location
                      </button>
                    ) : null}
                    <SaveState status={csvStatus} />
                  </>
                }
              >
                <code className="mt-4 block overflow-x-auto rounded-xl bg-[var(--surface-muted)] px-3 py-2.5 text-xs text-[var(--foreground)]">
                  {config.path}
                </code>
                <Hint summary="Using a cloud-synced folder?">
                  Cloud-synced folders are supported, but avoid editing the file at the same time on
                  multiple devices.
                </Hint>
              </SettingCard>
            </SettingsGroup>
          ) : null}
        </>
      ) : null}

      {pendingPath ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4" role="presentation">
          <section
            role="dialog"
            aria-modal="true"
            aria-labelledby="existing-csv-title"
            className="w-full max-w-lg rounded-2xl bg-[var(--surface)] p-6 shadow-[var(--shadow-md)]"
          >
            <h2 id="existing-csv-title" className="font-display text-xl font-semibold">
              {pendingPath.exists ? "CSV file already exists" : "Use this CSV location?"}
            </h2>
            <p className="mt-2 text-sm leading-relaxed text-[var(--muted)]">
              {pendingPath.exists
                ? "Import it to merge its editable fields into Job Tracker, or replace it with the current data from Job Tracker."
                : "Job Tracker will create this CSV and use it for future edits and scheduled jobs."}
            </p>
            <code className="mt-4 block overflow-x-auto rounded-xl bg-[var(--surface-muted)] px-3 py-2 text-xs">
              {pendingPath.path}
            </code>
            <div className="mt-6 flex flex-wrap justify-end gap-2">
              <button type="button" className="btn-secondary" disabled={csvBusy} onClick={() => setPendingPath(null)}>
                Cancel
              </button>
              {pendingPath.exists ? (
                <>
                  <button type="button" className="btn-secondary" disabled={csvBusy} onClick={() => void confirmPath("replace")}>
                    Replace with current data
                  </button>
                  <button type="button" className="btn-primary" disabled={csvBusy} onClick={() => void confirmPath("import")}>
                    Import and use
                  </button>
                </>
              ) : (
                <button type="button" className="btn-primary" disabled={csvBusy} onClick={() => void confirmPath("replace")}>
                  Confirm location
                </button>
              )}
            </div>
          </section>
        </div>
      ) : null}
    </div>
  );
}
