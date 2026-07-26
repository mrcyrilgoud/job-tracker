import path from "node:path";

export function nowIso() {
  return new Date().toISOString();
}

export function createId() {
  return crypto.randomUUID();
}

export function normalizeCanonicalUrl(rawUrl: string): string {
  const url = new URL(rawUrl);
  url.hash = "";
  url.hostname = url.hostname.toLowerCase();

  if (
    (url.protocol === "http:" && url.port === "80") ||
    (url.protocol === "https:" && url.port === "443")
  ) {
    url.port = "";
  }

  // Drop common tracking params
  const dropParams = [
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "gh_src",
    "ref",
  ];
  for (const key of dropParams) {
    url.searchParams.delete(key);
  }

  let pathname = url.pathname;
  if (pathname.length > 1 && pathname.endsWith("/")) {
    pathname = pathname.slice(0, -1);
  }
  url.pathname = pathname;

  return url.toString();
}

export function guessTitleFromUrl(rawUrl: string): string {
  try {
    const url = new URL(rawUrl);
    const parts = url.pathname.split("/").filter(Boolean);
    const last = parts[parts.length - 1] ?? url.hostname;
    return decodeURIComponent(last)
      .replace(/[-_]+/g, " ")
      .replace(/\.[a-z0-9]+$/i, "")
      .replace(/\b\w/g, (c) => c.toUpperCase());
  } catch {
    return "Untitled role";
  }
}

export function extensionForMime(mimeType: string, originalFilename: string) {
  const fromName = path.extname(originalFilename).toLowerCase();
  if (fromName) {
    return fromName;
  }

  switch (mimeType) {
    case "application/pdf":
      return ".pdf";
    case "application/vnd.openxmlformats-officedocument.wordprocessingml.document":
      return ".docx";
    case "text/plain":
      return ".txt";
    default:
      return "";
  }
}

export function formatLabel(value: string) {
  return value
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}
