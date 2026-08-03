import type { CompanyWatch, JobListItem, WatchProvider } from "@/lib/schema";
import type { Presentation } from "@/lib/ui";

export function providerLabel(provider: WatchProvider | string): string {
  switch (provider) {
    case "greenhouse":
      return "Greenhouse";
    case "lever":
      return "Lever";
    case "ashby":
      return "Ashby";
    default:
      return provider.charAt(0).toUpperCase() + provider.slice(1);
  }
}

/**
 * A watch fails transiently all the time — a timeout on one cycle means nothing.
 * Only sustained failure is worth alarming the user about, so the first couple
 * of misses read as "retrying" rather than "broken".
 */
const ATTENTION_THRESHOLD = 3;

export function watchPresentation(watch: CompanyWatch): Presentation {
  if (watch.consecutiveSyncFailures >= ATTENTION_THRESHOLD) {
    return { label: "Needs attention", tone: "danger" };
  }
  if (watch.consecutiveSyncFailures > 0) {
    return { label: "Retrying", tone: "amber" };
  }
  if (!watch.lastSyncedAt) {
    return { label: "Not checked yet", tone: "stone" };
  }
  return { label: "Watching", tone: "green" };
}

/**
 * Sync errors arrive as raw transport strings, e.g.
 * "error sending request for url (https://api.lever.co/...): operation timed out".
 * Return a sentence a person can act on. The raw text is still shown, but tucked
 * behind a "Technical details" disclosure rather than shouted in the card.
 */
export function syncErrorNote(error: string, provider: WatchProvider | string): string {
  const name = providerLabel(provider);
  const text = error.toLowerCase();

  // Order matters: transport errors embed their cause, so the specific cause
  // has to be matched before the generic "error sending request" wrapper.
  if (/timed out|timeout/.test(text)) {
    return `${name} didn't respond in time. We'll try again on the next sync.`;
  }
  if (/\b404\b|not found/.test(text)) {
    return `${name} no longer has a board at this address. Check the board name, or remove this watch.`;
  }
  if (/\b401\b|\b403\b|unauthorized|forbidden/.test(text)) {
    return `${name} refused the request. This board may have been made private.`;
  }
  if (/\b429\b|rate limit/.test(text)) {
    return `${name} asked us to slow down. We'll try again later.`;
  }
  if (/\b5\d\d\b/.test(text)) {
    return `${name} is having trouble on their end. We'll try again later.`;
  }
  if (/dns|resolve|connect|network|error sending request/.test(text)) {
    return `Couldn't reach ${name}. Check your connection and try again.`;
  }
  return `${name} didn't respond the way we expected.`;
}

/** "3 new roles" / "1 new role" — used on the count chip and the roll-up header. */
export function roleCountLabel(count: number): string {
  return `${count} new ${count === 1 ? "role" : "roles"}`;
}

/**
 * Roles a watch found that are not on the board yet, keyed by company. The
 * backend already scopes these (`is_new_from_watch = 1`); this only groups them
 * so a company card can show its own count without a second round trip.
 */
export function groupRolesByCompany(roles: JobListItem[]): Map<string, JobListItem[]> {
  const grouped = new Map<string, JobListItem[]>();
  for (const role of roles) {
    const existing = grouped.get(role.job.companyId);
    if (existing) {
      existing.push(role);
    } else {
      grouped.set(role.job.companyId, [role]);
    }
  }
  return grouped;
}

/** Case-insensitive match across the parts of a role a person would search by. */
export function matchesRoleSearch(role: JobListItem, query: string): boolean {
  const trimmed = query.trim().toLowerCase();
  if (!trimmed) return true;
  return [role.job.title, role.job.location ?? "", role.companyName].some((field) =>
    field.toLowerCase().includes(trimmed),
  );
}
