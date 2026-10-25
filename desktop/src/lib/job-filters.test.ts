import { describe, expect, it } from "vitest";

import { filterCompaniesBySearch, filterDraftFromUrl, validateSalaryRange } from "./job-filters";

describe("job filter draft", () => {
  it("hydrates both controls from the active URL filters", () => {
    expect(filterDraftFromUrl("OpenAI", "active")).toEqual({
      search: "OpenAI",
      postingState: "active",
      salaryMin: "",
      salaryMax: "",
      statuses: [],
      dateField: "",
      dateFrom: "",
      dateTo: "",
      location: "",
      remoteMode: "any",
    });
  });

  it("clears control values when URL navigation removes the filters", () => {
    expect(filterDraftFromUrl(null, null)).toEqual({
      search: "",
      postingState: "",
      salaryMin: "",
      salaryMax: "",
      statuses: [],
      dateField: "",
      dateFrom: "",
      dateTo: "",
      location: "",
      remoteMode: "any",
    });
  });

  it("hydrates annual salary bounds from the URL", () => {
    expect(filterDraftFromUrl("role", "active", "120000", "180000")).toEqual({
      search: "role",
      postingState: "active",
      salaryMin: "120000",
      salaryMax: "180000",
      statuses: [],
      dateField: "",
      dateFrom: "",
      dateTo: "",
      location: "",
      remoteMode: "any",
    });
  });
});

describe("company sidebar filter", () => {
  const companies = [
    { id: "1", name: "OpenAI" },
    { id: "2", name: "Anthropic" },
    { id: "3", name: "Acme Labs" },
  ];

  it("matches company names case-insensitively after trimming the query", () => {
    expect(filterCompaniesBySearch(companies, "  AI ")).toEqual([
      { id: "1", name: "OpenAI" },
    ]);
  });

  it("keeps every company when the query is blank", () => {
    expect(filterCompaniesBySearch(companies, "  ")).toEqual(companies);
  });
});

describe("annual income range validation", () => {
  it("allows either open bound and valid whole-dollar amounts", () => {
    expect(validateSalaryRange("", "")).toBeNull();
    expect(validateSalaryRange("120000", "")).toBeNull();
    expect(validateSalaryRange("", "180000")).toBeNull();
    expect(validateSalaryRange("120000", "180000")).toBeNull();
  });

  it("rejects negative, fractional, and unsafe amounts", () => {
    expect(validateSalaryRange("-1", "")).toBe("Enter whole-dollar amounts of 0 or more.");
    expect(validateSalaryRange("100.50", "")).toBe("Enter whole-dollar amounts of 0 or more.");
    expect(validateSalaryRange("999999999999999999999", "")).toBe("Enter whole-dollar amounts of 0 or more.");
  });

  it("rejects a minimum greater than the maximum", () => {
    expect(validateSalaryRange("180000", "120000")).toBe("Minimum income must not exceed maximum income.");
  });
});
import fc from "fast-check";

import {
  countActiveFilterDimensions,
  encodeStatusesToUrl,
  isValidCalendarDate,
  parseDateFilterFromUrl,
  parseRemoteModeFromUrl,
  parseStatusesFromUrl,
  validateDateRange,
  type DateField,
} from "./job-filters";
import { jobStatuses, type JobStatus, type RemoteMode } from "@/lib/schema";

const dateFields: readonly DateField[] = ["appliedAt", "postingDate", "lastCheckedAt"];
const remoteModes: readonly RemoteMode[] = ["any", "remoteOnly", "onsiteOnly"];

/** A random subset of jobStatuses in canonical order (no dups). */
const canonicalStatusSetArb = fc
  .subarray([...jobStatuses], { minLength: 0, maxLength: jobStatuses.length })
  .map((set) => jobStatuses.filter((status) => set.includes(status)));

/** A valid YYYY-MM-DD calendar date derived from fc.date(). */
const calendarDateArb = fc
  .date({ min: new Date("1900-01-01T00:00:00Z"), max: new Date("2999-12-31T00:00:00Z"), noInvalidDate: true })
  .map((date) => date.toISOString().slice(0, 10));

describe("job-filters property tests", () => {
  // Feature: enhanced-filters, Property 3: URL Filter_State round-trip and normalization
  it("round-trips and normalizes a valid Filter_State through URL params", () => {
    fc.assert(
      fc.property(
        // statuses given as an arbitrary (possibly unordered, possibly duplicated) input list.
        fc.array(fc.constantFrom(...jobStatuses), { minLength: 0, maxLength: 20 }),
        fc.option(
          fc.record({
            field: fc.constantFrom(...dateFields),
            from: fc.option(calendarDateArb, { nil: "" }),
            to: fc.option(calendarDateArb, { nil: "" }),
          }),
          { nil: undefined },
        ),
        fc.option(fc.string(), { nil: "" }),
        fc.constantFrom(...remoteModes),
        (statusInput, dateInput, locationInput, remoteMode) => {
          // Expected normalized status set: dedup + canonical jobStatuses order.
          const statusSet = new Set<string>(statusInput);
          const normalizedStatuses: JobStatus[] = jobStatuses.filter((s) => statusSet.has(s));

          // Normalized date dimension: only active when a field is present. Treat
          // blank bounds as unset. A field with no bounds is still carried (field
          // param encodes/decodes), but from/to are "".
          const expectedField = dateInput?.field;
          const expectedFrom = dateInput?.from ?? "";
          const expectedTo = dateInput?.to ?? "";

          // Normalized location: trim; empty encodes to absent and decodes to empty.
          const expectedLocation = (locationInput ?? "").trim();

          // --- Encode exactly as JobsPage does. ---
          const params = new URLSearchParams();
          encodeStatusesToUrl(params, statusInput as JobStatus[]);
          if (expectedField) {
            params.set("dateField", expectedField);
            if (expectedFrom) params.set("dateFrom", expectedFrom);
            if (expectedTo) params.set("dateTo", expectedTo);
          }
          if (expectedLocation) params.set("location", expectedLocation);
          if (remoteMode !== "any") params.set("remote", remoteMode);

          // --- Inactive dimensions encode to an absent param. ---
          if (normalizedStatuses.length === 0) expect(params.has("statuses")).toBe(false);
          if (!expectedField) expect(params.has("dateField")).toBe(false);
          if (!expectedFrom) expect(params.has("dateFrom")).toBe(false);
          if (!expectedTo) expect(params.has("dateTo")).toBe(false);
          if (!expectedLocation) expect(params.has("location")).toBe(false);
          if (remoteMode === "any") expect(params.has("remote")).toBe(false);

          // --- Decode via the parse helpers. ---
          const decodedStatuses = parseStatusesFromUrl(params);
          const decodedDate = parseDateFilterFromUrl(params);
          const decodedLocation = params.get("location")?.trim() ?? "";
          const decodedRemote = parseRemoteModeFromUrl(params);

          // --- Decoded normalized form equals the original normalized form. ---
          expect(decodedStatuses).toEqual(normalizedStatuses);
          // Each status appears at most once.
          expect(new Set(decodedStatuses).size).toBe(decodedStatuses.length);

          expect(decodedDate.field).toBe(expectedField);
          expect(decodedDate.from).toBe(expectedFrom);
          expect(decodedDate.to).toBe(expectedTo);

          expect(decodedLocation).toBe(expectedLocation);
          expect(decodedRemote).toBe(remoteMode);
        },
      ),
      { numRuns: 100 },
    );
  });

  // Feature: enhanced-filters, Property 4: Restore robustness against malformed input
  it("never throws and yields only in-domain values from arbitrary params", () => {
    // Keys that carry filter meaning, so junk values exercise the real parse paths.
    const knownKeys = ["statuses", "dateField", "dateFrom", "dateTo", "remote", "location"];
    const keyArb = fc.oneof(fc.constantFrom(...knownKeys), fc.string());

    fc.assert(
      fc.property(
        fc.array(fc.tuple(keyArb, fc.string()), { maxLength: 30 }),
        (entries) => {
          const params = new URLSearchParams();
          for (const [key, value] of entries) params.append(key, value);

          // Must never throw.
          const statuses = parseStatusesFromUrl(params);
          const dateFilter = parseDateFilterFromUrl(params);
          const remote = parseRemoteModeFromUrl(params);

          // Statuses ⊆ jobStatuses with no dups.
          for (const status of statuses) {
            expect((jobStatuses as readonly string[]).includes(status)).toBe(true);
          }
          expect(new Set(statuses).size).toBe(statuses.length);

          // Date field ∈ domain or undefined.
          if (dateFilter.field !== undefined) {
            expect((dateFields as readonly string[]).includes(dateFilter.field)).toBe(true);
          }
          // from/to are either "" or a valid calendar date.
          expect(dateFilter.from === "" || isValidCalendarDate(dateFilter.from)).toBe(true);
          expect(dateFilter.to === "" || isValidCalendarDate(dateFilter.to)).toBe(true);

          // Remote ∈ {any, remoteOnly, onsiteOnly}.
          expect((remoteModes as readonly string[]).includes(remote)).toBe(true);
        },
      ),
      { numRuns: 100 },
    );
  });

  // Feature: enhanced-filters, Property 6: Date-range validation
  it("rejects a date range iff a bound is invalid or start > end", () => {
    // Mix valid calendar dates with junk and empty strings.
    const boundArb = fc.oneof(calendarDateArb, fc.string(), fc.constant(""));

    fc.assert(
      fc.property(boundArb, boundArb, (from, to) => {
        const start = from.trim();
        const end = to.trim();
        const startInvalid = start !== "" && !isValidCalendarDate(start);
        const endInvalid = end !== "" && !isValidCalendarDate(end);
        const outOfOrder = start !== "" && end !== "" && !startInvalid && !endInvalid && start > end;

        const shouldReject = startInvalid || endInvalid || outOfOrder;
        const result = validateDateRange(from, to);

        if (shouldReject) {
          expect(result).not.toBeNull();
        } else {
          expect(result).toBeNull();
        }
      }),
      { numRuns: 100 },
    );
  });

  // Feature: enhanced-filters, Property 11: Active-filter count
  it("counts active filter dimensions exactly and within 0..=3", () => {
    fc.assert(
      fc.property(
        canonicalStatusSetArb,
        fc.oneof(fc.constant<"" | DateField>(""), fc.constantFrom(...dateFields)),
        fc.oneof(calendarDateArb, fc.string(), fc.constant("")),
        fc.oneof(calendarDateArb, fc.string(), fc.constant("")),
        fc.string(),
        fc.constantFrom(...remoteModes),
        (statuses, dateField, dateFrom, dateTo, location, remoteMode) => {
          const input = { statuses, dateField, dateFrom, dateTo, location, remoteMode };

          const statusDim = statuses.length > 0 ? 1 : 0;
          const dateDim =
            dateField !== "" && (dateFrom.trim() !== "" || dateTo.trim() !== "") ? 1 : 0;
          const locationDim = location.trim() !== "" || remoteMode !== "any" ? 1 : 0;
          const expected = statusDim + dateDim + locationDim;

          const count = countActiveFilterDimensions(input);
          expect(count).toBe(expected);
          expect(count).toBeGreaterThanOrEqual(0);
          expect(count).toBeLessThanOrEqual(3);
        },
      ),
      { numRuns: 100 },
    );
  });
});
