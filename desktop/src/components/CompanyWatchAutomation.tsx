import { useEffect, useRef, useState } from "react";

import { api, type JobUrlPreview } from "@/lib/api";
import {
  confirmJobUrlPreview,
  defaultStoredPostingUrl,
  formatInvokeError,
  formatStoredPostingOption,
  isConfirmedJobDiscovery,
  resetJobUrlDiscovery,
  storedPostingLinksFromJobs,
  type StoredPostingLink,
} from "@/lib/job-url-preview";
import { watchProviders, type CompanyRow, type WatchProvider } from "@/lib/schema";
import { formatLabel } from "@/lib/utils";

function isValidPostingUrl(value: string) {
  try {
    const parsed = new URL(value);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

export function CompanyWatchAutomation({
  row,
  onChanged,
  compact = false,
}: {
  row: CompanyRow;
  onChanged: () => void;
  /** When true, use a shorter intro for embedding inside company cards. */
  compact?: boolean;
}) {
  const [url, setUrl] = useState("");
  const [storedLinks, setStoredLinks] = useState<StoredPostingLink[]>([]);
  const [linksLoading, setLinksLoading] = useState(false);
  const [preview, setPreview] = useState<JobUrlPreview | null>(null);
  const [confirmed, setConfirmed] = useState<ReturnType<typeof confirmJobUrlPreview>>(null);
  const [provider, setProvider] = useState<WatchProvider>("greenhouse");
  const [boardSlug, setBoardSlug] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const previewInFlight = useRef(false);
  const companyId = row.company.id;

  function clearDiscovery() {
    const reset = resetJobUrlDiscovery();
    setPreview(reset.preview);
    setConfirmed(reset.confirmedDiscovery);
  }

  useEffect(() => {
    let cancelled = false;
    const reset = resetJobUrlDiscovery();
    setPreview(reset.preview);
    setConfirmed(reset.confirmedDiscovery);
    setError(null);
    setMessage(null);

    async function loadStoredLinks() {
      setLinksLoading(true);
      try {
        const result = await api.listJobs({ companyId });
        if (cancelled) return;
        const links = storedPostingLinksFromJobs(result.jobs);
        setStoredLinks(links);
        setUrl(defaultStoredPostingUrl(links));
      } catch {
        if (cancelled) return;
        setStoredLinks([]);
        setUrl("");
      } finally {
        if (!cancelled) setLinksLoading(false);
      }
    }

    void loadStoredLinks();
    return () => {
      cancelled = true;
    };
  }, [companyId]);

  async function detectBoard() {
    if (previewInFlight.current) return;
    const postingUrl = url.trim();
    setError(null);
    setMessage(null);
    if (!isValidPostingUrl(postingUrl)) {
      setError("Enter a valid http or https posting link.");
      return;
    }
    previewInFlight.current = true;
    setBusy(true);
    try {
      const result = await api.previewJobUrl(postingUrl);
      setPreview(result);
      setConfirmed(null);
      if (!result.board && !result.careersUrl) {
        setError("No supported job board or careers page was found in that link.");
      }
    } catch (err) {
      setError(formatInvokeError(err, "Could not inspect that posting link."));
    } finally {
      previewInFlight.current = false;
      setBusy(false);
    }
  }

  async function confirmDiscovery() {
    if (!preview || !confirmed) return;
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      if (confirmed.provider && confirmed.boardSlug) {
        await api.createWatch(row.company.id, confirmed.provider, confirmed.boardSlug);
        setMessage("Board confirmed and now being watched");
      } else if (confirmed.careersUrl) {
        await api.createCompany(row.company.name, confirmed.careersUrl);
        setMessage("Careers page confirmed");
      }
      setUrl(defaultStoredPostingUrl(storedLinks));
      clearDiscovery();
      onChanged();
    } catch (err) {
      setError(formatInvokeError(err, "Could not save this discovery."));
    } finally {
      setBusy(false);
    }
  }

  async function createManualWatch() {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await api.createWatch(row.company.id, provider, boardSlug);
      setBoardSlug("");
      setMessage("Board verified and now being watched");
      onChanged();
    } catch (err) {
      setError(formatInvokeError(err, "Failed to create watch"));
    } finally {
      setBusy(false);
    }
  }

  const isConfirmed = isConfirmedJobDiscovery(preview, confirmed);
  const selectedStoredUrl = storedLinks.some((link) => link.url === url) ? url : "";
  const intro =
    storedLinks.length > 0
      ? compact
        ? "Pick a tracked posting from this company, or paste a different URL to detect its ATS board."
        : "Use a tracked posting from this company, or paste a different URL to detect the ATS board (Greenhouse, Lever, Ashby, etc.)."
      : compact
        ? "Paste any job posting URL from this company to detect and watch its ATS board."
        : "Paste a job posting URL to detect the ATS board (Greenhouse, Lever, Ashby, etc.), then start watching for new roles.";

  return (
    <div className="rounded-xl border border-[var(--line)] bg-[var(--surface-muted)] p-4">
      <p className="font-medium">
        {compact ? "Detect job board" : "Job board automation"}
      </p>
      <p className="mt-1 text-sm text-[var(--muted)]">{intro}</p>

      {storedLinks.length > 0 ? (
        <label className="mt-3 block space-y-1.5 text-sm">
          <span className="font-medium">Tracked posting</span>
          <select
            value={selectedStoredUrl}
            onChange={(event) => {
              const next = event.target.value;
              if (!next) {
                setUrl("");
              } else {
                setUrl(next);
              }
              setError(null);
              setMessage(null);
              clearDiscovery();
            }}
            className="field"
            aria-label="Tracked job posting for this company"
            disabled={linksLoading || busy}
          >
            <option value="">Paste a different URL below…</option>
            {storedLinks.map((link) => (
              <option key={link.jobId} value={link.url}>
                {formatStoredPostingOption(link)}
              </option>
            ))}
          </select>
        </label>
      ) : linksLoading ? (
        <p className="mt-3 text-sm text-[var(--muted)]">Loading tracked postings…</p>
      ) : null}

      <div className="mt-3 flex flex-col gap-2 sm:flex-row">
        <input
          value={url}
          onChange={(event) => {
            setUrl(event.target.value);
            setError(null);
            setMessage(null);
            clearDiscovery();
          }}
          placeholder={
            storedLinks.length > 0
              ? "Or paste a different job posting URL…"
              : "Paste job posting URL…"
          }
          aria-label="Job posting URL to detect board"
          className="field"
          type="url"
        />
        <button
          type="button"
          className="btn btn-primary shrink-0"
          disabled={busy || linksLoading}
          onClick={() => void detectBoard()}
        >
          {busy && !preview ? "Detecting…" : "Detect job board"}
        </button>
      </div>

      {preview?.board ? (
        <div className="mt-3 rounded-xl border border-[var(--line)] bg-[var(--surface)] p-3">
          <p className="text-sm font-medium">{isConfirmed ? "Board confirmed" : "Detected job board"}</p>
          <p className="text-sm text-[var(--muted)]">
            {preview.board.provider} / {preview.board.boardSlug}
          </p>
          {!isConfirmed ? (
            <button
              type="button"
              className="btn btn-secondary mt-2"
              disabled={busy}
              onClick={() => setConfirmed(confirmJobUrlPreview(preview))}
            >
              Confirm this board
            </button>
          ) : (
            <button type="button" className="btn btn-primary mt-2" disabled={busy} onClick={() => void confirmDiscovery()}>
              Start watching
            </button>
          )}
        </div>
      ) : preview?.careersUrl ? (
        <div className="mt-3 rounded-xl border border-[var(--line)] bg-[var(--surface)] p-3">
          <p className="text-sm font-medium">{isConfirmed ? "Careers page confirmed" : "Detected careers page"}</p>
          <a href={preview.careersUrl} target="_blank" rel="noreferrer" className="text-sm text-[var(--accent)] hover:underline">
            {preview.careersUrl}
          </a>
          {!isConfirmed ? (
            <button type="button" className="btn btn-secondary mt-2" disabled={busy} onClick={() => setConfirmed(confirmJobUrlPreview(preview))}>
              Confirm this careers page
            </button>
          ) : (
            <button type="button" className="btn btn-primary mt-2" disabled={busy} onClick={() => void confirmDiscovery()}>
              Save careers page
            </button>
          )}
        </div>
      ) : null}

      <details className="mt-3">
        <summary className="cursor-pointer text-sm text-[var(--muted)]">Enter board manually</summary>
        <div className="mt-2 grid grid-cols-2 gap-2">
          <select value={provider} onChange={(event) => setProvider(event.target.value as WatchProvider)} className="field">
            {watchProviders.map((value) => (
              <option key={value} value={value}>{formatLabel(value)}</option>
            ))}
          </select>
          <input value={boardSlug} onChange={(event) => setBoardSlug(event.target.value)} placeholder="Board name" className="field" />
        </div>
        <button type="button" className="btn btn-secondary mt-2" disabled={busy || !boardSlug.trim()} onClick={() => void createManualWatch()}>
          Verify and start watching
        </button>
      </details>

      {message ? <p className="mt-3 text-sm text-[var(--green-ink)]">{message}</p> : null}
      {error ? <p className="mt-3 text-sm text-[var(--danger)]" role="alert">{error}</p> : null}
    </div>
  );
}
