import { save } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";

import { api, type CsvConfig, type CsvPathStatus } from "@/lib/api";

export function SettingsPage() {
  const [config, setConfig] = useState<CsvConfig | null>(null);
  const [pendingPath, setPendingPath] = useState<CsvPathStatus | null>(null);
  const [roleKeywords, setRoleKeywords] = useState("");
  const [locationCountry, setLocationCountry] = useState("");
  const [locationCities, setLocationCities] = useState("");
  const [locationSaved, setLocationSaved] = useState(false);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [csv, keywords, locationSettings] = await Promise.all([
        api.csvConfig(),
        api.getWatchRoleKeywords(),
        api.getLocationSettings(),
      ]);
      setConfig(csv);
      setRoleKeywords(keywords);
      setLocationCountry(locationSettings.country);
      setLocationCities(locationSettings.cities);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load CSV settings");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function choosePath() {
    if (!config) return;
    setError(null);
    const selected = await save({
      defaultPath: config.path,
      filters: [{ name: "CSV files", extensions: ["csv"] }],
    });
    if (!selected) return;

    setBusy(true);
    try {
      const status = await api.csvPathStatus(selected);
      setPendingPath(status);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not use that CSV location");
    } finally {
      setBusy(false);
    }
  }

  async function confirmPath(mode: "import" | "replace") {
    if (!pendingPath) return;
    setBusy(true);
    setError(null);
    try {
      setConfig(await api.configureCsv(pendingPath.path, mode));
      setPendingPath(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not update the CSV location");
    } finally {
      setBusy(false);
    }
  }

  async function useDefault() {
    setBusy(true);
    setError(null);
    try {
      setConfig(await api.resetCsvConfig());
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not restore the default location");
    } finally {
      setBusy(false);
    }
  }

  async function handleKeywordsSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.setWatchRoleKeywords(roleKeywords);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save keywords");
    } finally {
      setBusy(false);
    }
  }

  async function handleLocationSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.setLocationSettings({ country: locationCountry, cities: locationCities });
      setLocationSaved(true);
      setTimeout(() => setLocationSaved(false), 1000);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save location settings");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">Settings</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Choose where your editable jobs CSV is stored. Your database stays local to Job Tracker.
        </p>
      </div>

      {loading ? <p className="text-sm text-[var(--muted)]">Loading settings…</p> : null}
      {error ? (
        <p role="alert" className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
          {error}
        </p>
      ) : null}

      {config ? (
        <section className="card p-5" aria-labelledby="csv-location-heading">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h2 id="csv-location-heading" className="font-display text-lg font-semibold">
                Jobs CSV location
              </h2>
              <p className="mt-1 text-sm text-[var(--muted)]">
                Job Tracker synchronizes this file after edits and scheduled jobs.
              </p>
            </div>
            <span className="rounded-full bg-[var(--surface-muted)] px-2.5 py-1 text-xs font-medium text-[var(--muted)]">
              {config.isCustom ? "Custom location" : "Default location"}
            </span>
          </div>

          <code className="mt-5 block overflow-x-auto rounded-xl bg-[var(--surface-muted)] px-3 py-2.5 text-xs text-[var(--foreground)]">
            {config.path}
          </code>

          <div className="mt-5 flex flex-wrap gap-2">
            <button type="button" className="btn-primary" disabled={busy} onClick={() => void choosePath()}>
              {busy ? "Updating…" : "Choose CSV…"}
            </button>
            {config.isCustom ? (
              <button type="button" className="btn-secondary" disabled={busy} onClick={() => void useDefault()}>
                Use default location
              </button>
            ) : null}
          </div>
          <p className="mt-4 text-xs leading-relaxed text-[var(--faint)]">
            Cloud-synced folders are supported, but avoid editing the file at the same time on multiple devices.
          </p>
        </section>
      ) : null}

      <section className="card p-5" aria-labelledby="keywords-heading">
        <h2 id="keywords-heading" className="font-display text-lg font-semibold">
          Role Keywords
        </h2>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Only show new roles from watches that match these comma-separated keywords (e.g. "Software Engineer, Frontend"). Leaving this empty will show all new roles.
        </p>
        <form className="mt-5 flex gap-2" onSubmit={handleKeywordsSubmit}>
          <input
            value={roleKeywords}
            onChange={(e) => setRoleKeywords(e.target.value)}
            placeholder="e.g. Software Engineer, Frontend"
            className="field flex-1 border-transparent bg-[var(--surface-muted)]"
          />
          <button type="submit" className="btn-primary" disabled={busy}>
            {busy ? "Saving…" : "Save"}
          </button>
        </form>
      </section>

      <section className="card p-5" aria-labelledby="location-heading">
        <h2 id="location-heading" className="font-display text-lg font-semibold">
          Location Preferences
        </h2>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Set global location filters. Jobs matching your preferred country or cities will be included. 
          <br/><em>Note: The cities field is smart. If you type a major hub like "San Jose" or "Bay Area", nearby cities like San Francisco and Oakland will automatically be matched!</em>
        </p>
        <form className="mt-5 flex flex-col gap-4" onSubmit={handleLocationSubmit}>
          <div className="flex flex-col gap-1.5">
            <label htmlFor="country" className="text-sm font-medium text-[var(--foreground)]">Country</label>
            <select
              id="country"
              value={locationCountry}
              onChange={(e) => setLocationCountry(e.target.value)}
              className="field border-transparent bg-[var(--surface-muted)]"
            >
              <option value="">Any Country</option>
              <option value="United States">United States</option>
              <option value="Canada">Canada</option>
              <option value="United Kingdom">United Kingdom</option>
              <option value="Australia">Australia</option>
              <option value="Germany">Germany</option>
              <option value="France">France</option>
              <option value="Spain">Spain</option>
              <option value="India">India</option>
            </select>
          </div>
          <div className="flex flex-col gap-1.5">
            <label htmlFor="cities" className="text-sm font-medium text-[var(--foreground)]">Cities / Regions</label>
            <input
              id="cities"
              value={locationCities}
              onChange={(e) => setLocationCities(e.target.value)}
              placeholder="e.g. San Jose, Austin, Seattle"
              className="field border-transparent bg-[var(--surface-muted)]"
            />
          </div>
          <div>
            <button type="submit" className="btn-primary" disabled={busy}>
              {busy ? "Saving…" : locationSaved ? "Saved!" : "Save Locations"}
            </button>
          </div>
        </form>
      </section>

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
              <button type="button" className="btn-secondary" disabled={busy} onClick={() => setPendingPath(null)}>
                Cancel
              </button>
              {pendingPath.exists ? (
                <>
                  <button type="button" className="btn-secondary" disabled={busy} onClick={() => void confirmPath("replace")}>
                    Replace with current data
                  </button>
                  <button type="button" className="btn-primary" disabled={busy} onClick={() => void confirmPath("import")}>
                    Import and use
                  </button>
                </>
              ) : (
                <button type="button" className="btn-primary" disabled={busy} onClick={() => void confirmPath("replace")}>
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
