import { useCallback, useEffect, useState } from "react";

import { api, type JobsCsvPathInfo, type SetJobsCsvPathResult } from "@/lib/api";

export function JobsCsvPathSettings() {
  const [info, setInfo] = useState<JobsCsvPathInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const next = await api.getJobsCsvPath();
      setInfo(next);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load CSV path");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  async function choosePath() {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const picked = await api.pickJobsCsvPath();
      if (!picked) {
        return;
      }

      const relocate = window.confirm(
        `Export your current jobs to this path?\n\n${picked}\n\nOK = export from DB (safe relocate)\nCancel = link an existing file instead`,
      );

      let result: SetJobsCsvPathResult;
      if (relocate) {
        result = await api.setJobsCsvPath({ path: picked, mode: "relocate_export" });
      } else {
        result = await linkExisting(picked);
      }

      if (result.envOverride) {
        setMessage(
          `Saved setting, but JOB_TRACKER_JOBS_CSV is set and currently overrides it → ${result.path}`,
        );
      } else {
        setMessage(`Jobs CSV path updated (${result.action})`);
      }
      await load();
    } catch (err) {
      const text = err instanceof Error ? err.message : "Failed to set CSV path";
      if (text === "Link cancelled") {
        return;
      }
      setError(text);
    } finally {
      setBusy(false);
    }
  }

  async function linkExisting(path: string): Promise<SetJobsCsvPathResult> {
    try {
      const preview = await api.setJobsCsvPath({
        path,
        mode: "link_with_sidecar",
        dryRun: true,
      });
      const summary = preview.import?.summary;
      const detail = summary
        ? `Dry-run: ${summary.created} created, ${summary.updated} updated, ${summary.conflicts} conflicts, ${summary.unchanged} unchanged.`
        : "Dry-run completed.";
      const ok = window.confirm(
        `${detail}\n\nMerge this CSV into your database?\n\n${path}`,
      );
      if (!ok) {
        throw new Error("Link cancelled");
      }
      return api.setJobsCsvPath({
        path,
        mode: "link_with_sidecar",
        confirm: true,
      });
    } catch (err) {
      const text = err instanceof Error ? err.message : String(err);
      if (text.includes("No sync sidecar") || text.includes("link_without_sidecar")) {
        return linkWithoutSidecar(path);
      }
      throw err;
    }
  }

  async function linkWithoutSidecar(path: string): Promise<SetJobsCsvPathResult> {
    const exportOverwrite = window.confirm(
      `No sync sidecar found beside this CSV.\n\nOK = replace the file with an export from your database (safe)\nCancel = see destructive import options`,
    );
    if (exportOverwrite) {
      return api.setJobsCsvPath({
        path,
        mode: "link_without_sidecar",
        withoutSidecarAction: "export_overwrite",
      });
    }

    const destructive = window.confirm(
      `Import this CSV with overwrite_editable?\n\nThis replaces editable job fields in your database from the CSV. There is no sync baseline — do this only if you trust the file.\n\n${path}`,
    );
    if (!destructive) {
      throw new Error("Link cancelled");
    }
    return api.setJobsCsvPath({
      path,
      mode: "link_without_sidecar",
      withoutSidecarAction: "overwrite_editable",
      confirm: true,
    });
  }

  async function resetPath() {
    if (
      !window.confirm(
        "Reset jobs.csv to the default location under your data directory?\n\nCurrent jobs will be exported there first. A previous custom file is left in place.",
      )
    ) {
      return;
    }
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const result = await api.resetJobsCsvPath();
      if (result.envOverride) {
        setMessage(
          `Reset saved, but JOB_TRACKER_JOBS_CSV still overrides → ${result.path}`,
        );
      } else {
        setMessage("Jobs CSV path reset to default");
      }
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to reset CSV path");
    } finally {
      setBusy(false);
    }
  }

  async function reveal() {
    setError(null);
    try {
      await api.revealJobsCsvPath();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to reveal in Finder");
    }
  }

  if (!info) {
    return null;
  }

  return (
    <section className="rounded-2xl border border-[var(--border)] bg-[var(--surface)] px-4 py-3 shadow-[var(--shadow-sm)]">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1">
          <h2 className="text-sm font-semibold text-[var(--muted)]">Jobs CSV</h2>
          <p className="truncate font-mono text-xs text-[var(--foreground)]" title={info.path}>
            {info.path}
          </p>
          <p className="text-xs text-[var(--faint)]">
            {info.isDefault
              ? "Default location beside your local database"
              : "Custom path · database stays in Application Support / data"}
            {info.envOverride
              ? " · JOB_TRACKER_JOBS_CSV overrides the saved setting"
              : null}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void choosePath()}>
            Choose…
          </button>
          <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void reveal()}>
            Reveal in Finder
          </button>
          {!info.isDefault || info.envOverride ? (
            <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void resetPath()}>
              Reset
            </button>
          ) : null}
        </div>
      </div>
      {info.envOverride ? (
        <p className="mt-2 text-xs text-[var(--amber-ink)]">
          Environment override is active. Packaged app + LaunchAgent should rely on the saved
          setting, not env.
        </p>
      ) : null}
      {message ? <p className="mt-2 text-xs text-[var(--green-ink)]">{message}</p> : null}
      {error ? <p className="mt-2 text-xs text-[var(--danger)]">{error}</p> : null}
    </section>
  );
}
