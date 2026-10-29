/**
 * Property tests for Sort_Annotations (companies-tab-sorting Properties 11–13).
 */
import fc from "fast-check";
import { describe, expect, it } from "vitest";
import {
  EASE_POOL,
  MALFORMED_COUNT_POOL,
  MALFORMED_EASE_POOL,
  arbCreatedAt,
  arbName,
  arbTimestamp,
  pipelineRow,
} from "./__fixtures__/company-row-arbitraries";
import { formatEase, sortAnnotationFor } from "./company-sidebar";
import type { CompanyRow } from "./schema";

const ZONES = ["UTC", "America/Los_Angeles", "Asia/Tokyo", "Pacific/Kiritimati"] as const;
const LOCALE = "en-US";
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

type Ymd = { year: number; month: number; day: number };
/** Expected meaning of a timestamp, written by hand: literal date, instant in epoch ms, or invalid. */
type Expected = { dateOnly: Ymd } | { ms: number } | null;

/** 2026-01-01T00:00:00Z */
const JAN1_2026 = 1767225600000;
/** 2026-10-01T00:00:00Z */
const OCT1_2026 = 1790812800000;
/** 2025-03-04T23:30:00Z */
const MAR4_2025_2330 = 1741131000000;
const HOUR = 3_600_000;

const TIMESTAMPS: ReadonlyArray<readonly [string, Expected]> = [
  // Date-only values: the literal date in every zone.
  ["2026-10-01", { dateOnly: { year: 2026, month: 10, day: 1 } }],
  ["2025-03-04", { dateOnly: { year: 2025, month: 3, day: 4 } }],
  ["2026-01-01", { dateOnly: { year: 2026, month: 1, day: 1 } }],
  ["2025-12-31", { dateOnly: { year: 2025, month: 12, day: 31 } }],
  ["2024-02-29", { dateOnly: { year: 2024, month: 2, day: 29 } }],
  // Date-times: the same instants written several ways.
  ["2026-10-01T00:00:00Z", { ms: OCT1_2026 }],
  ["2026-10-01T09:00:00+09:00", { ms: OCT1_2026 }],
  ["2026-09-30T17:00:00-07:00", { ms: OCT1_2026 }],
  ["2026-10-01 00:00", { ms: OCT1_2026 }],
  ["2026-10-01T00:00:00.000000001Z", { ms: OCT1_2026 }],
  ["2026-10-01T23:59:59.999Z", { ms: 1790899199999 }],
  ["2025-03-04T23:30:00Z", { ms: MAR4_2025_2330 }],
  ["2025-03-05T08:30:00+09:00", { ms: MAR4_2025_2330 }],
  ["2025-03-04 23:30", { ms: MAR4_2025_2330 }],
  // Around the 2025/2026 year boundary.
  ["2025-12-31T23:30:00Z", { ms: JAN1_2026 - HOUR / 2 }],
  ["2026-01-01T05:00:00-08:00", { ms: JAN1_2026 + 13 * HOUR }],
  ["2026-01-01T10:00:00z", { ms: JAN1_2026 + 10 * HOUR }],
  ["2025-12-31t12:00:00+14:00", { ms: JAN1_2026 - 26 * HOUR }],
  // Not Valid_Timestamps.
  ["pending", null],
  ["2026-02-30", null],
  [" 2026-10-01", null],
  ["March 4", null],
  ["2026-10-01T24:00", null],
  ["", null],
];

/** `now` values near year boundaries in the fixed zones, plus mid-year values. */
const NOWS: ReadonlyArray<number> = [
  JAN1_2026 - 4 * HOUR, // 2025-12-31T20:00Z: 2026 in Tokyo and Kiritimati, 2025 in UTC and LA
  JAN1_2026 + 5 * HOUR, // 2026-01-01T05:00Z: 2025 in LA
  JAN1_2026 + 9 * HOUR, // 2026-01-01T09:00Z: 2026 everywhere
  1750000000000, // 2025-06-15
  1781000000000, // 2026-06-09
  1720000000000, // 2024-07-03
  1800000000000, // 2027-01-15
];

/** Calendar date of an instant in `timeZone`, from Intl with hand-written epoch ms. */
function zonedYmd(ms: number, timeZone: string): Ymd {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    year: "numeric",
    month: "numeric",
    day: "numeric",
  }).formatToParts(new Date(ms));
  const get = (type: string) => Number(parts.find((p) => p.type === type)?.value);
  return { year: get("year"), month: get("month"), day: get("day") };
}

function expectedYmd(expected: Expected, timeZone: string): Ymd | null {
  if (expected === null) return null;
  return "dateOnly" in expected ? expected.dateOnly : zonedYmd(expected.ms, timeZone);
}

/** "Mon D" or "Mon D, YYYY", built without Intl date formatting. */
function expectedText(date: Ymd, nowMs: number, timeZone: string): string {
  const base = `${MONTHS[date.month - 1]} ${date.day}`;
  return date.year === zonedYmd(nowMs, timeZone).year ? base : `${base}, ${date.year}`;
}

const DATE_SORTS = [
  { sort: "recentActivity", field: "lastActivityAt", prefix: "Last activity" },
  { sort: "recentlyApplied", field: "lastAppliedAt", prefix: "Last applied" },
] as const;

const arbEntry = fc.constantFrom(...TIMESTAMPS);
/** null, a missing field, or a pooled string. */
const arbValue = fc.oneof(
  { arbitrary: fc.constant<"missing" | null>(null), weight: 1 },
  { arbitrary: fc.constant<"missing" | null>("missing"), weight: 1 },
  { arbitrary: arbEntry, weight: 6 },
);

function rowWith(field: "lastActivityAt" | "lastAppliedAt", value: "missing" | null | string): CompanyRow {
  const row = pipelineRow("c1", "Acme", value === "missing" ? {} : { [field]: value });
  if (value === "missing") delete (row as Partial<CompanyRow>)[field];
  return row;
}

describe("sortAnnotationFor date annotations", () => {
  it("Property 11: date annotations show the calendar date consistently", () => {
    // Feature: companies-tab-sorting, Property 11: Date annotations show the calendar date consistently
    /** **Validates: Requirements 4.7, 4.8, 6.4, 6.5** */
    fc.assert(
      fc.property(
        fc.constantFrom(...DATE_SORTS),
        fc.constantFrom(...ZONES),
        fc.constantFrom(...NOWS),
        arbValue,
        arbEntry,
        (spec, timeZone, nowMs, value, other) => {
          const ctx = { now: new Date(nowMs), locale: LOCALE, timeZone };
          const isEntry = typeof value === "object" && value !== null;
          const raw = isEntry ? value[0] : value;
          const expected = isEntry ? value[1] : null;
          const annotation = sortAnnotationFor(rowWith(spec.field, raw), spec.sort, ctx);

          const date = expectedYmd(expected, timeZone);
          if (date === null) {
            expect(annotation).toBeNull();
            return;
          }
          const text = expectedText(date, nowMs, timeZone);
          expect(annotation).toEqual({ text, label: `${spec.prefix} ${text}` });
          expect(/\b\d{4}\b/.test(text)).toBe(date.year !== zonedYmd(nowMs, timeZone).year);

          // Two rows with the same calendar date get identical annotations.
          const otherDate = expectedYmd(other[1], timeZone);
          const otherAnnotation = sortAnnotationFor(rowWith(spec.field, other[0]), spec.sort, ctx);
          if (otherDate && otherDate.year === date.year && otherDate.month === date.month && otherDate.day === date.day) {
            expect(otherAnnotation).toEqual(annotation);
          }
        },
      ),
      { numRuns: 300 },
    );
  });
});

describe("formatEase and the ease annotation", () => {
  it("Property 12: ease annotation rounds the exact mean half away from zero", () => {
    // Feature: companies-tab-sorting, Property 12: Ease annotation rounds the exact mean half away from zero
    /** **Validates: Requirements 6.6** */
    const arbNK = fc
      .integer({ min: 1, max: 50 })
      .chain((n) => fc.tuple(fc.constant(n), fc.integer({ min: n, max: 5 * n })));
    fc.assert(
      fc.property(arbNK, ([n, k]) => {
        // Exact mean k/n in tenths, rounded half up, from integers only.
        const tenths = Math.floor((20 * k + n) / (2 * n));
        const expected = `${Math.floor(tenths / 10)}.${tenths % 10}`;
        const mean = k / n; // same double Rust produces for ease_sum / ease_count
        expect(formatEase(mean)).toBe(expected);
        const row = pipelineRow("c1", "Acme", { wishlistJobCount: n, wishlistEaseAverage: mean });
        const ctx = { now: new Date(OCT1_2026), locale: LOCALE, timeZone: "UTC" };
        expect(sortAnnotationFor(row, "easiestWishlist", ctx)).toEqual({
          text: expected,
          label: `Average ease ${expected}`,
        });
      }),
      { numRuns: 300 },
    );
  });
});

/** A well-formed count (0..1000, biased toward 0 and 1) or a malformed value that sanitizes to 0. */
type CountInput = { raw: unknown; sanitized: number };
const arbWellFormedCount = fc.oneof(
  fc.constantFrom(0, 1),
  fc.integer({ min: 0, max: 1000 }),
);
const arbNewCount: fc.Arbitrary<CountInput> = fc.oneof(
  { arbitrary: arbWellFormedCount.map((n) => ({ raw: n, sanitized: n })), weight: 3 },
  { arbitrary: fc.constantFrom(...MALFORMED_COUNT_POOL).map((raw) => ({ raw, sanitized: 0 })), weight: 1 },
);

/** Hand-written expected ease text for each pooled average (one decimal, halves up). */
const EASE_TEXT = new Map<unknown, string>([
  [1, "1.0"],
  [3, "3.0"],
  [4.25, "4.3"],
  [4.6, "4.6"],
  [14 / 3, "4.7"],
  [4.67, "4.7"],
  [4.666, "4.7"],
  [5, "5.0"],
]);
const arbEaseInput: fc.Arbitrary<unknown> = fc.constantFrom<unknown>(...EASE_POOL, ...MALFORMED_EASE_POOL);

const COUNT_SORTS = [
  { sort: "wishlistJobsDesc", field: "wishlistJobCount", singular: "wishlist job", plural: "wishlist jobs" },
  { sort: "wishlistJobsAsc", field: "wishlistJobCount", singular: "wishlist job", plural: "wishlist jobs" },
  { sort: "appliedJobsDesc", field: "appliedJobCount", singular: "applied job", plural: "applied jobs" },
  { sort: "appliedJobsAsc", field: "appliedJobCount", singular: "applied job", plural: "applied jobs" },
  { sort: "interviewingJobsDesc", field: "interviewingJobCount", singular: "interview", plural: "interviews" },
  { sort: "openPositionsDesc", field: "openPositionCount", singular: "open position", plural: "open positions" },
  { sort: "trackedJobsDesc", field: "trackedJobCount", singular: "tracked job", plural: "tracked jobs" },
] as const;

const NEW_COUNT_FIELDS = ["wishlistJobCount", "appliedJobCount", "interviewingJobCount"] as const;

/** Assign a field, deleting it when the generated value is `undefined` (missing field). */
function setField(row: CompanyRow, field: string, value: unknown): void {
  const record = row as unknown as Record<string, unknown>;
  if (value === undefined) delete record[field];
  else record[field] = value;
}

const arbAnnotationCase = fc.record({
  name: arbName,
  createdAt: arbCreatedAt,
  openPositionCount: arbWellFormedCount,
  trackedJobCount: arbWellFormedCount,
  wishlist: arbNewCount,
  applied: arbNewCount,
  interviewing: arbNewCount,
  ease: arbEaseInput,
  lastActivityAt: arbTimestamp,
  lastAppliedAt: arbTimestamp,
  timeZone: fc.constantFrom(...ZONES),
  nowMs: fc.constantFrom(...NOWS),
});

describe("sortAnnotationFor count annotations", () => {
  it("Property 13: count annotations show the sanitized count with a pluralized label", () => {
    // Feature: companies-tab-sorting, Property 13: Count annotations show the sanitized count with a pluralized label
    /** **Validates: Requirements 6.1, 6.2, 6.3, 6.7, 6.8, 6.9** */
    fc.assert(
      fc.property(arbAnnotationCase, (c) => {
        const row = pipelineRow("c1", c.name, {
          createdAt: c.createdAt,
          openPositionCount: c.openPositionCount,
          trackedJobCount: c.trackedJobCount,
          lastActivityAt: c.lastActivityAt,
          lastAppliedAt: c.lastAppliedAt,
        });
        const inputs = { wishlistJobCount: c.wishlist, appliedJobCount: c.applied, interviewingJobCount: c.interviewing };
        for (const field of NEW_COUNT_FIELDS) setField(row, field, inputs[field].raw);
        setField(row, "wishlistEaseAverage", c.ease);
        const before = structuredClone(row);
        const ctx = { now: new Date(c.nowMs), locale: LOCALE, timeZone: c.timeZone };

        const expectedCount: Record<(typeof COUNT_SORTS)[number]["field"], number> = {
          wishlistJobCount: c.wishlist.sanitized,
          appliedJobCount: c.applied.sanitized,
          interviewingJobCount: c.interviewing.sanitized,
          // Existing counts: the raw number, unchanged from before this feature (Req 6.8).
          openPositionCount: c.openPositionCount,
          trackedJobCount: c.trackedJobCount,
        };

        for (const spec of COUNT_SORTS) {
          const n = expectedCount[spec.field];
          const text = String(n);
          expect(sortAnnotationFor(row, spec.sort, ctx)).toEqual({
            text,
            label: `${text} ${n === 1 ? spec.singular : spec.plural}`,
          });
        }

        // Name-based sorts never annotate.
        for (const sort of ["nameAsc", "nameDesc", "recentlyAdded"] as const) {
          expect(sortAnnotationFor(row, sort, ctx)).toBeNull();
        }

        // Ease: "Average ease {text}" for a well-formed average, null otherwise.
        const easeText = EASE_TEXT.get(c.ease);
        expect(sortAnnotationFor(row, "easiestWishlist", ctx)).toEqual(
          easeText === undefined ? null : { text: easeText, label: `Average ease ${easeText}` },
        );

        // Annotating never mutates the row.
        expect(row).toEqual(before);
      }),
      { numRuns: 300 },
    );
  });
});
