import dns from "node:dns/promises";
import net from "node:net";
import { isIP } from "node:net";

const MAX_BYTES = 1_500_000;
const TIMEOUT_MS = 10_000;
const MAX_REDIRECTS = 5;

export type SafeFetchResult = {
  ok: boolean;
  status: number;
  finalUrl: string;
  bodyText: string;
  error?: string;
};

function isPrivateIp(ip: string): boolean {
  if (net.isIPv4(ip)) {
    const parts = ip.split(".").map(Number);
    const [a, b] = parts;
    if (a === 10) return true;
    if (a === 127) return true;
    if (a === 0) return true;
    if (a === 169 && b === 254) return true;
    if (a === 172 && b >= 16 && b <= 31) return true;
    if (a === 192 && b === 168) return true;
    return false;
  }

  if (net.isIPv6(ip)) {
    const normalized = ip.toLowerCase();
    if (normalized === "::1") return true;
    if (normalized.startsWith("fc") || normalized.startsWith("fd")) return true;
    if (normalized.startsWith("fe80")) return true;
    return false;
  }

  return true;
}

async function assertPublicHostname(hostname: string) {
  if (hostname === "localhost" || hostname.endsWith(".local")) {
    throw new Error("Private or local hostnames are not allowed");
  }

  if (isIP(hostname)) {
    if (isPrivateIp(hostname)) {
      throw new Error("Private IP addresses are not allowed");
    }
    return;
  }

  const records = await dns.lookup(hostname, { all: true });
  if (records.length === 0) {
    throw new Error("Hostname could not be resolved");
  }

  for (const record of records) {
    if (isPrivateIp(record.address)) {
      throw new Error("Hostname resolves to a private address");
    }
  }
}

export async function safeFetch(
  rawUrl: string,
  options: { method?: "GET" | "HEAD"; accept?: string } = {},
): Promise<SafeFetchResult> {
  let current = rawUrl;

  try {
    for (let redirectCount = 0; redirectCount <= MAX_REDIRECTS; redirectCount++) {
      const url = new URL(current);
      if (url.protocol !== "http:" && url.protocol !== "https:") {
        return {
          ok: false,
          status: 0,
          finalUrl: current,
          bodyText: "",
          error: "Only HTTP and HTTPS URLs are allowed",
        };
      }

      await assertPublicHostname(url.hostname);

      const controller = new AbortController();
      const timeout = setTimeout(() => controller.abort(), TIMEOUT_MS);

      try {
        const response = await fetch(url.toString(), {
          method: options.method ?? "GET",
          redirect: "manual",
          signal: controller.signal,
          headers: {
            "User-Agent": "JobTrackerLocal/1.0",
            Accept: options.accept ?? "text/html,application/json,*/*",
          },
        });

        if ([301, 302, 303, 307, 308].includes(response.status)) {
          const location = response.headers.get("location");
          if (!location) {
            return {
              ok: false,
              status: response.status,
              finalUrl: current,
              bodyText: "",
              error: "Redirect missing Location header",
            };
          }
          current = new URL(location, current).toString();
          continue;
        }

        const contentLength = Number(response.headers.get("content-length") ?? "0");
        if (contentLength > MAX_BYTES) {
          return {
            ok: false,
            status: response.status,
            finalUrl: current,
            bodyText: "",
            error: "Response exceeds size limit",
          };
        }

        const buffer = Buffer.from(await response.arrayBuffer());
        if (buffer.byteLength > MAX_BYTES) {
          return {
            ok: false,
            status: response.status,
            finalUrl: current,
            bodyText: "",
            error: "Response exceeds size limit",
          };
        }

        return {
          ok: response.ok,
          status: response.status,
          finalUrl: current,
          bodyText: buffer.toString("utf8"),
        };
      } finally {
        clearTimeout(timeout);
      }
    }

    return {
      ok: false,
      status: 0,
      finalUrl: current,
      bodyText: "",
      error: "Too many redirects",
    };
  } catch (error) {
    const message =
      error instanceof Error
        ? error.name === "AbortError"
          ? "Request timed out"
          : error.message
        : "Request failed";

    return {
      ok: false,
      status: 0,
      finalUrl: current,
      bodyText: "",
      error: message,
    };
  }
}

export function extractHtmlTitle(html: string): string | null {
  const match = html.match(/<title[^>]*>([^<]*)<\/title>/i);
  if (!match?.[1]) {
    return null;
  }
  return match[1].replace(/\s+/g, " ").trim() || null;
}

export function looksLikeClosedPosting(html: string, status: number): boolean {
  if (status === 404 || status === 410) {
    return true;
  }

  const lower = html.toLowerCase();
  const closedSignals = [
    "no longer accepting applications",
    "job is closed",
    "this job has expired",
    "position has been filled",
    "this posting is no longer available",
    "sorry, this job is no longer available",
  ];

  return closedSignals.some((signal) => lower.includes(signal));
}
