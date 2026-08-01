import type { ConfirmedJobDiscovery, JobUrlPreview } from "@/lib/api";
import type { JobListItem } from "@/lib/schema";

export type JobPreviewFields = {
  title: string;
  companyName: string;
};

export type StoredPostingLink = {
  jobId: string;
  title: string;
  url: string;
  updatedAt: string;
};

function isHttpUrl(value: string) {
  try {
    const parsed = new URL(value);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

/** Unique posting URLs from tracked jobs, preserving newest-first order. */
export function storedPostingLinksFromJobs(items: JobListItem[]): StoredPostingLink[] {
  const seen = new Set<string>();
  const links: StoredPostingLink[] = [];
  for (const item of items) {
    const url = item.job.url.trim() || item.job.canonicalUrl.trim();
    if (!url || !isHttpUrl(url)) continue;
    const key = url.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    links.push({
      jobId: item.job.id,
      title: item.job.title.trim() || "Untitled role",
      url,
      updatedAt: item.job.updatedAt,
    });
  }
  return links;
}

export function defaultStoredPostingUrl(links: StoredPostingLink[]): string {
  return links[0]?.url ?? "";
}

export function formatStoredPostingOption(link: StoredPostingLink, maxUrlLength = 48): string {
  const url =
    link.url.length <= maxUrlLength
      ? link.url
      : `${link.url.slice(0, Math.max(1, maxUrlLength - 1))}…`;
  return `${link.title} — ${url}`;
}

export function applyJobUrlPreview(
  current: JobPreviewFields,
  preview: JobUrlPreview,
): JobPreviewFields {
  return {
    title: current.title.trim() ? current.title : (preview.title ?? current.title),
    companyName: current.companyName.trim()
      ? current.companyName
      : (preview.companyName ?? current.companyName),
  };
}

export function confirmJobUrlPreview(preview: JobUrlPreview): ConfirmedJobDiscovery | null {
  if (preview.board) {
    return {
      provider: preview.board.provider,
      boardSlug: preview.board.boardSlug,
    };
  }
  if (preview.careersUrl) {
    return { careersUrl: preview.careersUrl };
  }
  return null;
}

export function isConfirmedJobDiscovery(
  preview: JobUrlPreview | null,
  confirmedDiscovery: ConfirmedJobDiscovery | null,
): boolean {
  if (!preview || !confirmedDiscovery) return false;
  if (preview.board) {
    return (
      confirmedDiscovery.provider === preview.board.provider &&
      confirmedDiscovery.boardSlug === preview.board.boardSlug
    );
  }
  return preview.careersUrl === confirmedDiscovery.careersUrl;
}

export function resetJobUrlDiscovery() {
  return { preview: null, confirmedDiscovery: null } as const;
}

export function serializeConfirmedJobDiscovery(
  preview: JobUrlPreview | null,
  confirmedDiscovery: ConfirmedJobDiscovery | null,
): ConfirmedJobDiscovery | null {
  return isConfirmedJobDiscovery(preview, confirmedDiscovery) ? confirmedDiscovery : null;
}

/** Normalize Tauri invoke failures (often plain strings) into a readable message. */
export function formatInvokeError(error: unknown, fallback: string): string {
  if (typeof error === "string" && error.trim()) return error;
  if (error instanceof Error && error.message.trim()) return error.message;
  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    for (const key of ["message", "error", "msg"] as const) {
      const value = record[key];
      if (typeof value === "string" && value.trim()) return value;
    }
  }
  return fallback;
}

export function formatJobSaveError(error: unknown): string {
  const message = formatInvokeError(error, "Could not create job");
  if (/board|watch|validate|unavailable/i.test(message)) {
    return `${message} The watch was not created. Retry or set it up manually from Companies.`;
  }
  return message;
}
