/**
 * Strict Valid_Timestamp grammar shared with `src-tauri/src/instant.rs`
 * (design §1, companies-tab-sorting):
 *
 *   timestamp = date [ sep time [ offset ] ]
 *   date      = YYYY "-" MM "-" DD        ; 0001–9999, real calendar day
 *   sep       = "T" / "t" / " "
 *   time      = HH ":" mm [ ":" ss [ "." 1*9DIGIT ] ]
 *   offset    = "Z" / "z" / ( "+" / "-" ) HH ":" mm
 *
 * Date-only values denote 00:00:00 UTC. Date-times without an offset are UTC.
 * `Date.parse` is not used: it treats offset-less date-times as local time and
 * accepts many non-ISO strings.
 */

/** Instant with nanosecond precision: epoch milliseconds plus 0–999_999 extra nanoseconds. */
export type Instant = { readonly ms: number; readonly subMsNanos: number };

export type CalendarDate = { readonly year: number; readonly month: number; readonly day: number };

// ASCII digits only ([0-9], no `u` flag, no `\d`). No `m` flag, so `$` only
// matches at the true end of input and a trailing "\n" is rejected.
const TIMESTAMP_RE =
  /^([0-9]{4})-([0-9]{2})-([0-9]{2})(?:[Tt ]([0-9]{2}):([0-9]{2})(?::([0-9]{2})(?:\.([0-9]{1,9}))?)?(?:([Zz])|([+-])([0-9]{2}):([0-9]{2}))?)?$/;

function isLeapYear(year: number): boolean {
  return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
}

function daysInMonth(year: number, month: number): number {
  switch (month) {
    case 2:
      return isLeapYear(year) ? 29 : 28;
    case 4:
    case 6:
    case 9:
    case 11:
      return 30;
    default:
      return 31;
  }
}

/** Epoch ms for a UTC wall-clock value. Avoids `Date.UTC`, which maps years 0–99 to 1900–1999. */
function utcEpochMs(year: number, month: number, day: number, h = 0, mi = 0, s = 0, ms = 0): number {
  const d = new Date(0);
  d.setUTCFullYear(year, month - 1, day);
  d.setUTCHours(h, mi, s, ms);
  return d.getTime();
}

type ParsedTimestamp = { date: CalendarDate; instant: Instant; dateOnly: boolean };

function parseTimestamp(value: unknown): ParsedTimestamp | null {
  if (typeof value !== "string") return null;
  const m = TIMESTAMP_RE.exec(value);
  if (!m) return null;

  const year = Number(m[1]);
  const month = Number(m[2]);
  const day = Number(m[3]);
  if (year < 1 || month < 1 || month > 12 || day < 1 || day > daysInMonth(year, month)) {
    return null;
  }
  const date: CalendarDate = { year, month, day };

  if (m[4] === undefined) {
    return { date, instant: { ms: utcEpochMs(year, month, day), subMsNanos: 0 }, dateOnly: true };
  }

  const hour = Number(m[4]);
  const minute = Number(m[5]);
  const second = m[6] === undefined ? 0 : Number(m[6]);
  if (hour > 23 || minute > 59 || second > 59) return null;

  const fraction = (m[7] ?? "").padEnd(9, "0");
  const msPart = Number(fraction.slice(0, 3));
  const subMsNanos = Number(fraction.slice(3, 9));

  let offsetMinutes = 0;
  if (m[9] !== undefined) {
    const offH = Number(m[10]);
    const offM = Number(m[11]);
    if (offH > 23 || offM > 59) return null;
    offsetMinutes = (m[9] === "-" ? -1 : 1) * (offH * 60 + offM);
  }

  const ms = utcEpochMs(year, month, day, hour, minute, second, msPart) - offsetMinutes * 60_000;
  return { date, instant: { ms, subMsNanos }, dateOnly: false };
}

/** Strict parser for the Valid_Timestamp grammar. Returns null for any other input, including non-strings. */
export function parseInstant(value: unknown): Instant | null {
  return parseTimestamp(value)?.instant ?? null;
}

/** -1, 0, or 1 by (ms, subMsNanos). */
export function compareInstants(a: Instant, b: Instant): number {
  if (a.ms !== b.ms) return a.ms < b.ms ? -1 : 1;
  if (a.subMsNanos !== b.subMsNanos) return a.subMsNanos < b.subMsNanos ? -1 : 1;
  return 0;
}

// Gregorian calendar and Latin digits so formatToParts yields parseable numbers
// regardless of the user's default locale. Only date fields are requested, so
// hourCycle never matters.
const PARTS_LOCALE = "en-US-u-ca-gregory-nu-latn";

/** Calendar date of an epoch-ms instant in `timeZone` (undefined = user's zone). */
function calendarDateInZone(ms: number, timeZone?: string): CalendarDate {
  const parts = new Intl.DateTimeFormat(PARTS_LOCALE, {
    timeZone,
    era: "short",
    year: "numeric",
    month: "numeric",
    day: "numeric",
  }).formatToParts(new Date(ms));
  let year = 0;
  let month = 0;
  let day = 0;
  let bc = false;
  for (const part of parts) {
    if (part.type === "year") year = Number(part.value);
    else if (part.type === "month") month = Number(part.value);
    else if (part.type === "day") day = Number(part.value);
    else if (part.type === "era") bc = /^B/i.test(part.value);
  }
  // Astronomical year numbering: 1 BC is year 0.
  if (bc) year = 1 - year;
  return { year, month, day };
}

/**
 * Calendar date a Valid_Timestamp displays as (Req 6.4): a date-only value keeps its
 * literal date; a date-time value uses its date in `timeZone` (default: the user's).
 */
export function calendarDateOf(value: string, timeZone?: string): CalendarDate | null {
  const parsed = parseTimestamp(value);
  if (!parsed) return null;
  if (parsed.dateOnly) return parsed.date;
  return calendarDateInZone(parsed.instant.ms, timeZone);
}

function makeFormatter(locale: string | undefined, withYear: boolean): Intl.DateTimeFormat {
  const options: Intl.DateTimeFormatOptions = {
    timeZone: "UTC",
    month: "short",
    day: "numeric",
    ...(withYear ? { year: "numeric" } : {}),
  };
  try {
    return new Intl.DateTimeFormat(locale, options);
  } catch {
    // Structurally invalid locale tag: fall back to the default locale.
    return new Intl.DateTimeFormat(undefined, options);
  }
}

/**
 * "Mar 4" or "Mar 4, 2025" in `locale`. The year appears only when it differs from
 * `now`'s year in `timeZone`. Always formats from the CalendarDate with timeZone "UTC",
 * so equal CalendarDates give identical strings (Req 4.7).
 */
export function formatShortDate(
  date: CalendarDate,
  now: Date,
  locale?: string,
  timeZone?: string,
): string {
  const nowYear = calendarDateInZone(now.getTime(), timeZone).year;
  const withYear = date.year !== nowYear;
  const formatter = makeFormatter(locale, withYear);
  const at = new Date(utcEpochMs(date.year, date.month, date.day));
  if (!withYear) return formatter.format(at);
  // `year: "numeric"` prints years below 1000 without padding; the requirement
  // asks for a four-digit year.
  return formatter
    .formatToParts(at)
    .map((part) =>
      part.type === "year" && /^[0-9]{1,3}$/.test(part.value) ? part.value.padStart(4, "0") : part.value,
    )
    .join("");
}
