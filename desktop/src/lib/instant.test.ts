import fc from "fast-check";
import { describe, expect, it } from "vitest";
import corpus from "@/lib/__fixtures__/instant-corpus.json";
import { calendarDateOf, compareInstants, formatShortDate, parseInstant } from "@/lib/instant";

// Same generator shape as the Rust half in `src-tauri/src/instant.rs`.

type Offset = { kind: "Z" } | { kind: "z" } | { kind: "signed"; negative: boolean; hours: number; minutes: number };

type TimeParts = {
  sep: "T" | "t" | " ";
  hour: number;
  minute: number;
  /** `[second, fraction]`; fraction is `[digitCount, value]` with `value < 10 ** digitCount`. */
  seconds: [number, [number, number] | null] | null;
  offset: Offset | null;
};

type DateParts = [year: number, month: number, day: number];

function isLeapYear(y: number): boolean {
  return (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0;
}

function daysInMonth(y: number, m: number): number {
  return [31, isLeapYear(y) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1];
}

/**
 * Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
 * days_from_civil). Pure integer arithmetic, independent of `Date` and of
 * `instant.ts`, so years 0001–0099 are handled correctly.
 */
function daysFromCivil(y: number, m: number, d: number): number {
  const yy = m <= 2 ? y - 1 : y;
  const era = Math.floor(yy / 400);
  const yoe = yy - era * 400;
  const mp = (m + 9) % 12;
  const doy = Math.floor((153 * mp + 2) / 5) + d - 1;
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
  return era * 146097 + doe - 719468;
}

const dateArb: fc.Arbitrary<DateParts> = fc
  .tuple(fc.integer({ min: 1, max: 9999 }), fc.integer({ min: 1, max: 12 }))
  .chain(([y, m]) =>
    fc.integer({ min: 1, max: daysInMonth(y, m) }).map((d): DateParts => [y, m, d]),
  );

const fractionArb: fc.Arbitrary<[number, number]> = fc
  .integer({ min: 1, max: 9 })
  .chain((count) =>
    fc.integer({ min: 0, max: 10 ** count - 1 }).map((value): [number, number] => [count, value]),
  );

const offsetArb: fc.Arbitrary<Offset> = fc.oneof(
  fc.constant<Offset>({ kind: "Z" }),
  fc.constant<Offset>({ kind: "z" }),
  fc
    .record({
      negative: fc.boolean(),
      hours: fc.integer({ min: 0, max: 23 }),
      minutes: fc.integer({ min: 0, max: 59 }),
    })
    .map((o): Offset => ({ kind: "signed", ...o })),
);

const timeArb: fc.Arbitrary<TimeParts> = fc.record({
  sep: fc.constantFrom("T" as const, "t" as const, " " as const),
  hour: fc.integer({ min: 0, max: 23 }),
  minute: fc.integer({ min: 0, max: 59 }),
  seconds: fc.option(fc.tuple(fc.integer({ min: 0, max: 59 }), fc.option(fractionArb, { nil: null })), {
    nil: null,
  }),
  offset: fc.option(offsetArb, { nil: null }),
});

const pad = (n: number, width: number) => String(n).padStart(width, "0");

function formatParts([y, m, d]: DateParts, time: TimeParts | null): string {
  let s = `${pad(y, 4)}-${pad(m, 2)}-${pad(d, 2)}`;
  if (!time) return s;
  s += `${time.sep}${pad(time.hour, 2)}:${pad(time.minute, 2)}`;
  if (time.seconds) {
    const [sec, frac] = time.seconds;
    s += `:${pad(sec, 2)}`;
    if (frac) s += `.${pad(frac[1], frac[0])}`;
  }
  const o = time.offset;
  if (o?.kind === "Z") s += "Z";
  else if (o?.kind === "z") s += "z";
  else if (o?.kind === "signed") s += `${o.negative ? "-" : "+"}${pad(o.hours, 2)}:${pad(o.minutes, 2)}`;
  return s;
}

/** Instant computed from the components without going through the parser. */
function expectedInstant([y, m, d]: DateParts, time: TimeParts | null): { ms: number; subMsNanos: number } {
  const dayMs = daysFromCivil(y, m, d) * 86_400_000;
  if (!time) return { ms: dayMs, subMsNanos: 0 };
  const second = time.seconds ? time.seconds[0] : 0;
  const frac = time.seconds?.[1] ?? null;
  const nanos = frac ? frac[1] * 10 ** (9 - frac[0]) : 0;
  const o = time.offset;
  const offsetMinutes = o?.kind === "signed" ? (o.negative ? -1 : 1) * (o.hours * 60 + o.minutes) : 0;
  const ms =
    dayMs +
    ((time.hour * 60 + time.minute - offsetMinutes) * 60 + second) * 1000 +
    Math.floor(nanos / 1_000_000);
  return { ms, subMsNanos: nanos % 1_000_000 };
}

describe("parseInstant", () => {
  it("days-from-civil reference agrees with known epoch values", () => {
    expect(daysFromCivil(1970, 1, 1)).toBe(0);
    expect(daysFromCivil(2026, 10, 1) * 86_400_000).toBe(1790812800000);
    expect(daysFromCivil(1, 1, 1) * 86_400_000).toBe(-62135596800000);
  });

  /**
   * Feature: companies-tab-sorting, Property 5: Timestamp parsing round trip and
   * cross-language agreement. Formatting valid components into a grammar string and
   * parsing it yields the instant computed from the components (date-only at
   * 00:00:00 UTC, no offset treated as UTC).
   *
   * **Validates: Requirements 1.3, 4.5**
   */
  it("round-trips generated components to the expected instant", () => {
    fc.assert(
      fc.property(dateArb, fc.option(timeArb, { nil: null }), (date, time) => {
        const input = formatParts(date, time);
        expect(parseInstant(input), input).toEqual(expectedInstant(date, time));
      }),
      { numRuns: 100 },
    );
  });

  /**
   * Feature: companies-tab-sorting, Property 5: Timestamp parsing round trip and
   * cross-language agreement (shared corpus half, also read by `src-tauri/src/instant.rs`).
   *
   * **Validates: Requirements 1.3, 4.5**
   */
  it("matches the shared Rust/TS corpus", () => {
    expect(corpus.valid.length).toBeGreaterThan(0);
    expect(corpus.invalid.length).toBeGreaterThan(0);
    for (const entry of corpus.valid) {
      expect(parseInstant(entry.input), entry.input).toEqual({
        ms: entry.epochMs,
        subMsNanos: entry.subMsNanos,
      });
    }
    for (const input of corpus.invalid) {
      expect(parseInstant(input), JSON.stringify(input)).toBeNull();
    }
  });

  it("rejects non-string values", () => {
    for (const value of [undefined, null, 0, 1790812800000, {}, [], new Date(0)]) {
      expect(parseInstant(value)).toBeNull();
    }
  });
});

const ZONES = ["UTC", "America/Los_Angeles", "Asia/Tokyo", "Pacific/Kiritimati"] as const;

describe("calendarDateOf", () => {
  it("keeps the literal date of date-only values in every zone", () => {
    for (const zone of ZONES) {
      expect(calendarDateOf("2026-03-04", zone), zone).toEqual({ year: 2026, month: 3, day: 4 });
      expect(calendarDateOf("2024-02-29", zone), zone).toEqual({ year: 2024, month: 2, day: 29 });
      expect(calendarDateOf("2026-01-01", zone), zone).toEqual({ year: 2026, month: 1, day: 1 });
    }
  });

  it("shifts date-time values into the given zone", () => {
    // 03:00Z: LA (UTC-8) is still Mar 3; Tokyo (+9) and Kiritimati (+14) are Mar 4.
    const early = "2026-03-04T03:00:00Z";
    expect(calendarDateOf(early, "UTC")).toEqual({ year: 2026, month: 3, day: 4 });
    expect(calendarDateOf(early, "America/Los_Angeles")).toEqual({ year: 2026, month: 3, day: 3 });
    expect(calendarDateOf(early, "Asia/Tokyo")).toEqual({ year: 2026, month: 3, day: 4 });
    expect(calendarDateOf(early, "Pacific/Kiritimati")).toEqual({ year: 2026, month: 3, day: 4 });

    // 23:30Z: UTC and LA are Mar 4; Tokyo and Kiritimati are Mar 5.
    const late = "2026-03-04T23:30:00Z";
    expect(calendarDateOf(late, "UTC")).toEqual({ year: 2026, month: 3, day: 4 });
    expect(calendarDateOf(late, "America/Los_Angeles")).toEqual({ year: 2026, month: 3, day: 4 });
    expect(calendarDateOf(late, "Asia/Tokyo")).toEqual({ year: 2026, month: 3, day: 5 });
    expect(calendarDateOf(late, "Pacific/Kiritimati")).toEqual({ year: 2026, month: 3, day: 5 });
  });

  it("honors explicit offsets and treats offset-less date-times as UTC", () => {
    // 2025-12-31T20:00-05:00 is 2026-01-01T01:00Z.
    expect(calendarDateOf("2025-12-31T20:00-05:00", "UTC")).toEqual({ year: 2026, month: 1, day: 1 });
    expect(calendarDateOf("2025-12-31T20:00-05:00", "America/Los_Angeles")).toEqual({
      year: 2025,
      month: 12,
      day: 31,
    });
    expect(calendarDateOf("2026-03-04 23:30", "Asia/Tokyo")).toEqual({ year: 2026, month: 3, day: 5 });
  });

  it("returns null for values outside the grammar", () => {
    for (const value of ["", "2025-02-29", "2026-03-04T24:00Z", " 2026-03-04", "Mar 4, 2026"]) {
      expect(calendarDateOf(value, "UTC"), JSON.stringify(value)).toBeNull();
    }
  });
});

describe("formatShortDate", () => {
  const now = new Date("2026-06-15T12:00:00Z");

  it("omits the year when it matches now's year and includes it otherwise", () => {
    for (const zone of ZONES) {
      expect(formatShortDate({ year: 2026, month: 3, day: 4 }, now, "en-US", zone), zone).toBe("Mar 4");
      expect(formatShortDate({ year: 2025, month: 3, day: 4 }, now, "en-US", zone), zone).toBe(
        "Mar 4, 2025",
      );
    }
  });

  it("uses now's year in the given zone", () => {
    // 2026-01-01T05:00Z is still 2025-12-31 in Los Angeles.
    const newYearUtc = new Date("2026-01-01T05:00:00Z");
    const date = { year: 2025, month: 3, day: 4 };
    expect(formatShortDate(date, newYearUtc, "en-US", "UTC")).toBe("Mar 4, 2025");
    expect(formatShortDate(date, newYearUtc, "en-US", "America/Los_Angeles")).toBe("Mar 4");
  });

  it("formats the same CalendarDate identically regardless of zone", () => {
    const date = { year: 2026, month: 12, day: 31 };
    const outputs = ZONES.map((zone) => formatShortDate(date, now, "en-US", zone));
    expect(new Set(outputs)).toEqual(new Set(["Dec 31"]));
  });

  it("composes with calendarDateOf for date-only and date-time values", () => {
    expect(formatShortDate(calendarDateOf("2026-03-04", "Pacific/Kiritimati")!, now, "en-US", "Pacific/Kiritimati")).toBe(
      "Mar 4",
    );
    expect(
      formatShortDate(calendarDateOf("2026-03-04T03:00:00Z", "America/Los_Angeles")!, now, "en-US", "America/Los_Angeles"),
    ).toBe("Mar 3");
  });

  it("pads years below 1000 to four digits", () => {
    expect(formatShortDate({ year: 42, month: 3, day: 4 }, now, "en-US", "UTC")).toBe("Mar 4, 0042");
  });
});

describe("compareInstants", () => {
  const at = (value: string) => {
    const instant = parseInstant(value);
    expect(instant, value).not.toBeNull();
    return instant!;
  };

  it("orders a date-only value at 00:00:00 UTC", () => {
    expect(compareInstants(at("2026-10-01"), at("2026-10-01T00:00:01Z"))).toBe(-1);
    expect(compareInstants(at("2026-10-01T00:00:01Z"), at("2026-10-01"))).toBe(1);
    expect(compareInstants(at("2026-10-01"), at("2026-10-01T00:00:00Z"))).toBe(0);
  });

  it("breaks ties within a millisecond by sub-millisecond nanoseconds", () => {
    expect(compareInstants(at("2026-10-01T00:00:00.000000001Z"), at("2026-10-01T00:00:00Z"))).toBe(1);
    expect(compareInstants(at("2026-10-01T09:00:00+09:00"), at("2026-10-01T00:00:00z"))).toBe(0);
  });
});
