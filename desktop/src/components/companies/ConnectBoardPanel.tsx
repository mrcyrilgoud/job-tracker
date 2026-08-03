import { useEffect, useRef, useState } from "react";

import { api, type JobUrlPreview } from "@/lib/api";
import { providerLabel } from "@/lib/companies-ui";
import {
  defaultStoredPostingUrl,
  formatInvokeError,
  formatStoredPostingOption,
  storedPostingLinksFromJobs,
  type StoredPostingLink,
} from "@/lib/job-url-preview";
import { watchProviders, type CompanyRow, type WatchProvider } from "@/lib/schema";

function isValidPostingUrl(value: string) {
  try {
    const parsed = new URL(value);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

/**
 * Connects one company to an ATS board. Previously mounted in three places at
 * once (a global picker plus two per-company variants), which put up to six
 * copies of this form on one screen; it is now only ever opened on demand for a
 * single company, so it needs no intro variants and no company selector.
 */
export function ConnectBoardPanel({
  row,
  onChanged,
  onDone,
}: {
  row: CompanyRow;
  onChanged: () => void;
  /** Lets the parent close the panel once a board is connected. */
  onDone?: () => void;
}) {
  const [url, setUrl] = useState("");
  const [storedLinks, setStoredLinks] = useState<StoredPostingLink[]>([]);
  const [linksLoading, setLinksLoading] = useState(false);
  const [preview, setPreview] = useState<JobUrlPreview | null>(null);
  const [provider, setProvider] = useState<WatchProvider>("greenhouse");
  const [boardSlug, setBoardSlug] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const previewRequestId = useRef(0);
  const companyId = row.company.id;

  useEffect(() => {
    let cancelled = false;
    setPreview(null);
    setError(null);

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
    const postingUrl = url.trim();
    const requestId = ++previewRequestId.current;
    setError(null);
    if (!isValidPostingUrl(postingUrl)) {
      setError("Enter a link starting with http or https.");
      return;
    }
    setBusy(true);
    try {
      const result = await api.previewJobUrl(postingUrl);
      if (requestId !== previewRequestId.current || url.trim() !== postingUrl) return;
      setPreview(result);
      if (!result.board && !result.careersUrl) {
        setError("That link isn't a job board we recognize. Try a posting URL from this company.");
      }
    } catch (err) {
      if (requestId !== previewRequestId.current || url.trim() !== postingUrl) return;
      setError(formatInvokeError(err, "Could not read that link."));
    } finally {
      if (requestId === previewRequestId.current) setBusy(false);
    }
  }

  async function startWatching() {
    if (!preview) return;
    setBusy(true);
    setError(null);
    try {
      if (preview.board) {
        await api.createWatch(companyId, preview.board.provider, preview.board.boardSlug);
      } else if (preview.careersUrl) {
        await api.createCompany(row.company.name, preview.careersUrl);
      }
      setPreview(null);
      setUrl(defaultStoredPostingUrl(storedLinks));
      onChanged();
      onDone?.();
    } catch (err) {
      setError(formatInvokeError(err, "Could not start watching that board."));
    } finally {
      setBusy(false);
    }
  }

  async function createManualWatch() {
    setBusy(true);
    setError(null);
    try {
      await api.createWatch(companyId, provider, boardSlug.trim());
      setBoardSlug("");
      onChanged();
      onDone?.();
    } catch (err) {
      setError(formatInvokeError(err, "Could not verify that board."));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="rounded-xl bg-[var(--surface-muted)] p-4">
      {storedLinks.length > 0 ? (
        <label className="block space-y-1.5 text-sm">
          <span className="font-medium">Use a posting you already track</span>
          <select
            value={storedLinks.some((link) => link.url === url) ? url : ""}
            onChange={(event) => {
              setUrl(event.target.value);
              previewRequestId.current += 1;
              setPreview(null);
              setError(null);
            }}
            className="field"
            aria-label="Tracked job posting for this company"
            disabled={linksLoading || busy}
          >
            <option value="">Paste a different link below…</option>
            {storedLinks.map((link) => (
              <option key={link.jobId} value={link.url}>
                {formatStoredPostingOption(link)}
              </option>
            ))}
          </select>
        </label>
      ) : null}

      <div className={`flex flex-col gap-2 sm:flex-row ${storedLinks.length > 0 ? "mt-3" : ""}`}>
        <input
          value={url}
          onChange={(event) => {
            setUrl(event.target.value);
            previewRequestId.current += 1;
            setPreview(null);
            setError(null);
          }}
          placeholder="Paste any job posting link from this company…"
          aria-label="Job posting URL to detect board"
          className="field"
          type="url"
        />
        <button
          type="button"
          className="btn btn-secondary shrink-0"
          disabled={busy || linksLoading}
          onClick={() => void detectBoard()}
        >
          {busy && !preview ? <span className="spinner" aria-hidden /> : null}
          {busy && !preview ? "Looking…" : "Find board"}
        </button>
      </div>

      {preview?.board ? (
        <div className="mt-3 flex flex-wrap items-center justify-between gap-3 rounded-xl bg-[var(--surface)] p-3.5 shadow-[var(--shadow-sm)]">
          <div>
            <p className="text-sm font-medium">
              Found this company on {providerLabel(preview.board.provider)}
            </p>
            <p className="text-xs text-[var(--muted)]">Board: {preview.board.boardSlug}</p>
          </div>
          <button
            type="button"
            className="btn btn-primary btn-sm"
            disabled={busy}
            onClick={() => void startWatching()}
          >
            {busy ? <span className="spinner" aria-hidden /> : null}
            Start watching
          </button>
        </div>
      ) : preview?.careersUrl ? (
        <div className="mt-3 flex flex-wrap items-center justify-between gap-3 rounded-xl bg-[var(--surface)] p-3.5 shadow-[var(--shadow-sm)]">
          <div className="min-w-0">
            <p className="text-sm font-medium">No job board, but there is a careers page</p>
            <a
              href={preview.careersUrl}
              target="_blank"
              rel="noreferrer"
              className="block truncate text-xs text-[var(--accent)] hover:underline"
            >
              {preview.careersUrl}
            </a>
          </div>
          <button
            type="button"
            className="btn btn-primary btn-sm"
            disabled={busy}
            onClick={() => void startWatching()}
          >
            Watch this page
          </button>
        </div>
      ) : null}

      {error ? (
        <p className="mt-3 text-sm text-[var(--danger)]" role="alert">
          {error}
        </p>
      ) : null}

      <details className="mt-3">
        <summary className="cursor-pointer text-xs text-[var(--muted)] hover:text-[var(--foreground)]">
          I know the board name
        </summary>
        <div className="mt-2 grid grid-cols-2 gap-2">
          <select
            value={provider}
            onChange={(event) => setProvider(event.target.value as WatchProvider)}
            className="field"
            aria-label="Applicant tracking system"
          >
            {watchProviders.map((value) => (
              <option key={value} value={value}>
                {providerLabel(value)}
              </option>
            ))}
          </select>
          <input
            value={boardSlug}
            onChange={(event) => setBoardSlug(event.target.value)}
            placeholder="Board name"
            aria-label="Board name"
            className="field"
          />
        </div>
        <button
          type="button"
          className="btn btn-secondary btn-sm mt-2"
          disabled={busy || !boardSlug.trim()}
          onClick={() => void createManualWatch()}
        >
          Start watching
        </button>
      </details>
    </div>
  );
}
