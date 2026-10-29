/**
 * Shared fast-check arbitraries for companies-tab-sorting property tests
 * (Properties 6–15). Test-only: imported by `*.test.ts` files, never by app code.
 *
 * Pools deliberately collide: a small name pool with case/accent variants, counts
 * 0..3, near-equal ease averages, the same instant written several ways, and
 * invalid timestamp strings. Every timestamp's expected instant is written by hand
 * (as bigint nanoseconds since the epoch) so reference models never call the parser.
 */
import fc from "fast-check";
import type { CompanyRow } from "../schema";

const MS = 1_000_000n;

/** 2026-10-01T00:00:00Z */
const OCT1 = 1790812800000n * MS;
/** 2025-03-04T23:30:00Z */
const MAR4_2330 = 1741131000000n * MS;
/** 2025-03-04T00:00:00Z */
const MAR4 = 1741046400000n * MS;

/**
 * Timestamp strings with their expected instant in ns since the epoch (null = not a
 * Valid_Timestamp). Several strings denote the same instant.
 */
export const TIMESTAMP_POOL: ReadonlyArray<readonly [string, bigint | null]> = [
  ["2026-10-01", OCT1],
  ["2026-10-01T00:00:00Z", OCT1],
  ["2026-10-01T09:00:00+09:00", OCT1],
  ["2026-09-30T17:00:00-07:00", OCT1],
  ["2026-10-01 00:00", OCT1],
  ["2026-10-01T00:00:00.000000001Z", OCT1 + 1n],
  ["2026-10-01T00:00:01Z", OCT1 + 1000n * MS],
  ["2026-10-01T23:59:59.999Z", 1790899199999n * MS],
  ["2025-03-04", MAR4],
  ["2025-03-04T23:30:00Z", MAR4_2330],
  ["2025-03-05T08:30:00+09:00", MAR4_2330],
  ["2025-03-04 23:30", MAR4_2330],
  ["pending", null],
  ["2026-02-30", null],
  [" 2026-10-01", null],
  ["March 4", null],
  ["2026-10-01T24:00", null],
  ["", null],
];

const INSTANT_BY_STRING: ReadonlyMap<string, bigint | null> = new Map(TIMESTAMP_POOL);

/** Hand-written expected instant (ns) for a pooled timestamp; null for null/invalid/unknown. */
export function expectedInstantNs(value: unknown): bigint | null {
  if (typeof value !== "string") return null;
  return INSTANT_BY_STRING.get(value) ?? null;
}

/** Company `createdAt` values with expected ms (invalid → -Infinity, sorting as oldest). */
export const CREATED_AT_POOL: ReadonlyArray<readonly [string, number]> = [
  ["2025-01-01T00:00:00.000Z", 1735689600000],
  ["2025-06-15T12:00:00.000Z", 1749988800000],
  ["2026-01-01T00:00:00.000Z", 1767225600000],
  ["not-a-date", -Infinity],
];

const CREATED_MS_BY_STRING: ReadonlyMap<string, number> = new Map(CREATED_AT_POOL);

export function expectedCreatedAtMs(value: string): number {
  return CREATED_MS_BY_STRING.get(value) ?? -Infinity;
}

/** Small name pool with case/accent variants and duplicates so Name_Order ties occur. */
export const NAME_POOL = ["Abby Care", "abby care", "AKASA", "akasa", "Ákasa", "Beta", "beta", "Éclair", "eclair", "Zeta"];

/** Well-formed ease averages, including near-equal values that round to the same or adjacent tenths. */
export const EASE_POOL: ReadonlyArray<number | null> = [null, 1, 3, 4.25, 4.6, 14 / 3, 4.67, 4.666, 5];

/** Malformed count values (Req 3.10): each sanitizes to 0. `undefined` means the field is missing. */
export const MALFORMED_COUNT_POOL: ReadonlyArray<unknown> = [undefined, null, -1, 1.5, Number.NaN, Infinity, -Infinity, "2"];

/** Malformed ease averages (Req 5.5): each sanitizes to null. */
export const MALFORMED_EASE_POOL: ReadonlyArray<unknown> = [undefined, Number.NaN, Infinity, -Infinity, 0, 0.5, 5.01, 6, "4"];

export const arbCount = fc.integer({ min: 0, max: 3 });
export const arbName = fc.constantFrom(...NAME_POOL);
export const arbTimestamp: fc.Arbitrary<string | null> = fc.oneof(
  fc.constant(null),
  fc.constantFrom(...TIMESTAMP_POOL.map(([value]) => value)),
);
export const arbEase: fc.Arbitrary<number | null> = fc.constantFrom(...EASE_POOL);
export const arbCreatedAt = fc.constantFrom(...CREATED_AT_POOL.map(([value]) => value));

/** Build a CompanyRow with pipeline defaults (zeros / nulls). */
export function pipelineRow(
  id: string,
  name: string,
  overrides: Partial<Omit<CompanyRow, "company">> & { createdAt?: string } = {},
): CompanyRow {
  const { createdAt = "2026-01-01T00:00:00.000Z", ...rest } = overrides;
  return {
    company: { id, name, careersUrl: null, createdAt, updatedAt: createdAt },
    watches: [],
    reviews: [],
    openPositionCount: 0,
    trackedJobCount: 0,
    wishlistJobCount: 0,
    appliedJobCount: 0,
    interviewingJobCount: 0,
    lastActivityAt: null,
    lastAppliedAt: null,
    wishlistEaseAverage: null,
    ...rest,
  };
}

/** Short ids over a tiny alphabet, so id tie-breaks (code-unit order) are exercised. */
const arbIds = (maxLength: number) =>
  fc.uniqueArray(fc.string({ unit: fc.constantFrom("a", "b", "c", "A", "1", "_"), minLength: 1, maxLength: 4 }), {
    maxLength,
    // Default size averages ~5 rows; "medium" yields longer lists and more ties.
    size: "medium",
  });

/** Well-formed pipeline fields for one row. */
const arbPipelineFields = fc.record({
  name: arbName,
  createdAt: arbCreatedAt,
  openPositionCount: arbCount,
  trackedJobCount: arbCount,
  wishlistJobCount: arbCount,
  appliedJobCount: arbCount,
  interviewingJobCount: arbCount,
  lastActivityAt: arbTimestamp,
  lastAppliedAt: arbTimestamp,
  wishlistEaseAverage: arbEase,
});

/** Rows with unique ids and well-formed pipeline aggregates drawn from the colliding pools. */
export const arbPipelineRows = (maxLength = 30): fc.Arbitrary<CompanyRow[]> =>
  arbIds(maxLength)
    .chain((ids) =>
      fc.tuple(...ids.map((id) => arbPipelineFields.map(({ name, ...rest }) => pipelineRow(id, name, rest)))),
    )
    .map((rows): CompanyRow[] => [...rows]);

const KEEP = Symbol("keep");
const DELETE = Symbol("delete");

/** Per field: keep the well-formed value, delete the field, or replace it with a malformed value. */
const arbMutation = (pool: ReadonlyArray<unknown>) =>
  fc.oneof(
    { arbitrary: fc.constant<unknown>(KEEP), weight: 2 },
    { arbitrary: fc.constant<unknown>(DELETE), weight: 1 },
    { arbitrary: fc.constantFrom(...pool.filter((v) => v !== undefined)), weight: 2 },
  );

/**
 * Rows whose new count/ease fields may each be deleted or replaced by a malformed
 * value (Property 7). Typed as CompanyRow but may violate the backend contract.
 */
export const arbMaybeMalformedRows = (maxLength = 30): fc.Arbitrary<CompanyRow[]> =>
  arbPipelineRows(maxLength).chain((rows) =>
    fc
      .tuple(
        ...rows.map((r) =>
          fc
            .record({
              wishlistJobCount: arbMutation(MALFORMED_COUNT_POOL),
              appliedJobCount: arbMutation(MALFORMED_COUNT_POOL),
              interviewingJobCount: arbMutation(MALFORMED_COUNT_POOL),
              wishlistEaseAverage: arbMutation(MALFORMED_EASE_POOL),
            })
            .map((mutations) => {
              const copy: Record<string, unknown> = { ...r };
              for (const [key, m] of Object.entries(mutations)) {
                if (m === KEEP) continue;
                if (m === DELETE) delete copy[key];
                else copy[key] = m;
              }
              return copy as unknown as CompanyRow;
            }),
        ),
      )
      .map((out): CompanyRow[] => [...out]),
  );
