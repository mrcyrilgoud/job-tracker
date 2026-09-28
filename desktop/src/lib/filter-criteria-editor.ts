import type { FilterCriteria } from "@/lib/schema";

/**
 * Pure logic backing the {@link FilterCriteriaEditor} component and the
 * per-watch filter controls in `WatchRow`. Keeping these helpers free of React
 * lets us unit-test the real code paths the components use (chip editing, the
 * match-all seed, the "using global" vs override derivation, reset-to-global,
 * and preview outcome mapping) following the existing `lib/*.test.ts`
 * pure-function convention.
 */

/**
 * Commit a single raw chip draft onto an existing token list. The draft is
 * trimmed; a whitespace-only or empty draft leaves the list unchanged (mirrors
 * backend Req 17.3 drop-empty behavior). Returns a new array; never mutates the
 * input.
 */
export function addChip(tokens: string[], raw: string): string[] {
  const token = raw.trim();
  if (token.length === 0) return tokens;
  return [...tokens, token];
}

/**
 * Remove the chip at `index`, returning a new array. An out-of-range index
 * leaves the list unchanged (other than producing a fresh copy).
 */
export function removeChipAt(tokens: string[], index: number): string[] {
  return tokens.filter((_, i) => i !== index);
}

/**
 * Split a raw value that may contain comma delimiters, committing every part
 * before the final comma onto `tokens` and returning both the updated tokens
 * and the remaining draft (the text after the last comma). Each committed part
 * is trimmed and empty parts are dropped (Req 17.3). Mirrors the ChipInput
 * comma-delimiter handling.
 */
export function commitCommaSeparated(
  tokens: string[],
  rawWithCommas: string,
): { tokens: string[]; draft: string } {
  if (!rawWithCommas.includes(",")) {
    return { tokens, draft: rawWithCommas };
  }
  const parts = rawWithCommas.split(",");
  const last = parts.pop() ?? "";
  let next = tokens;
  for (const part of parts) {
    next = addChip(next, part);
  }
  return { tokens: next, draft: last };
}

/**
 * The match-all default used to seed editors when no override exists yet: an
 * empty include/exclude on both dimensions, no country, remote "any", and word
 * matching. Returns a fresh object each call so callers can edit it freely.
 */
export function matchAllCriteria(): FilterCriteria {
  return {
    version: 1,
    title: { include: [], exclude: [], matchMode: "word" },
    location: { country: null, include: [], exclude: [], matchMode: "word" },
    remote: "any",
  };
}

/**
 * True when a per-watch override exists (a non-null criteria), i.e. the board
 * carries its own "Custom filter" rather than inheriting the global filter
 * ("Using global filter"). Drives the WatchRow label (Req 13.2).
 */
export function isOverride(
  watchFilterCriteria: FilterCriteria | null | undefined,
): boolean {
  return watchFilterCriteria != null;
}

/**
 * The payload sent to `setWatchFilterCriteria` to reset a board back to the
 * global filter: `null` clears the override (Req 13.4).
 */
export const resetOverridePayload: null = null;

/**
 * Map a preview outcome to the label the editor renders in its live preview:
 * an included role is "Shown", anything else (excluded or missing outcome) is
 * "Hidden" (Req 13.5).
 */
export function previewLabel(
  outcome: { included: boolean } | null | undefined,
): "Shown" | "Hidden" {
  return outcome?.included ? "Shown" : "Hidden";
}
