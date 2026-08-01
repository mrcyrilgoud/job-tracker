import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";

import { api, type JobsRunnerProgress } from "@/lib/api";
import { isDesktopShell } from "@/lib/tauri";

export function RunJobsButton() {
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<JobsRunnerProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const busyRef = useRef(false);
  const clearProgressTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (!isDesktopShell()) {
      return;
    }

    let unlisten: (() => void) | undefined;

    void listen<JobsRunnerProgress>("jobs-runner-progress", (event) => {
      // This event is shared with the page-level posting check. Only show
      // progress here when this button started the jobs cycle.
      if (busyRef.current) {
        setProgress(event.payload);
      }
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
      if (clearProgressTimer.current !== undefined) {
        window.clearTimeout(clearProgressTimer.current);
      }
    };
  }, []);

  async function run() {
    if (clearProgressTimer.current !== undefined) {
      window.clearTimeout(clearProgressTimer.current);
    }
    busyRef.current = true;
    setBusy(true);
    setError(null);
    setProgress({ phase: "starting", message: "Starting jobs cycle…" });
    try {
      await api.runJobsCycle();
      setProgress({ phase: "done", message: "Jobs cycle complete" });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Jobs cycle failed");
      setProgress(null);
    } finally {
      busyRef.current = false;
      setBusy(false);
      clearProgressTimer.current = window.setTimeout(() => {
        setProgress(null);
      }, 3000);
    }
  }

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => void run()}
        disabled={busy}
        className="btn btn-secondary text-xs"
        title="Run posting checks, watch sync, Gmail poll, and CSV export"
      >
        {busy ? "Running…" : "Run jobs"}
      </button>
      {progress ? (
        <p className="absolute right-0 top-full z-10 mt-1 w-56 rounded-lg bg-[var(--surface)] px-2 py-1 text-xs text-[var(--muted)] shadow-[var(--shadow-md)]">
          {progress.message}
        </p>
      ) : null}
      {error ? (
        <p className="absolute right-0 top-full z-10 mt-1 w-56 rounded-lg bg-[var(--danger-soft)] px-2 py-1 text-xs text-[var(--danger)]">
          {error}
        </p>
      ) : null}
    </div>
  );
}
