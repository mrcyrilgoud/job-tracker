import fc from "fast-check";
import { describe, expect, it } from "vitest";
import {
  COMPANY_FILTERS,
  COMPANY_SORT_OPTIONS,
  COMPANY_VIEW_STORAGE_KEY,
  DEFAULT_COMPANY_VIEW_STATE,
  applyCompanyFilters,
  clearCompanyFilters,
  compareByNameOrder,
  companyEmptyState,
  deriveCompanyList,
  loadCompanyViewState,
  saveCompanyViewState,
  sortCompanies,
  sortCountFor,
  parseCompanyViewState,
  serializeCompanyViewState,
  toggleCompanyFilter,
  type CompanyFilter,
  type CompanyViewState,
} from "./company-sidebar";
import { filterCompaniesBySearch } from "./job-filters";
import type { CompanyRow, CompanyWatch } from "./schema";

function row(
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
    ...rest,
  };
}

function watch(consecutiveSyncFailures: number): CompanyWatch {
  return {
    id: `w${consecutiveSyncFailures}`,
    companyId: "c",
    provider: "greenhouse",
    boardSlug: "slug",
    lastSyncedAt: null,
    consecutiveSyncFailures,
    lastSyncError: null,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  };
}

const ids = (rows: CompanyRow[]) => rows.map((r) => r.company.id);

// ---------------------------------------------------------------------------
// Shared fast-check arbitraries for the property tests in this file.
// ---------------------------------------------------------------------------

const PBT = { numRuns: 100 } as const;

/** Small alphabet with mixed case, accents, and duplicates so ties and case folding occur. */
const NAME_POOL = ["Abby Care", "abby care", "AKASA", "akasa", "Ákasa", "Beta", "beta", "Éclair", "eclair", "Zeta", "zeta", "Ω"];

/** A few repeated timestamps plus invalid values (treated as oldest). */
const CREATED_AT_POOL = [
  "2025-01-01T00:00:00.000Z",
  "2025-06-15T12:00:00.000Z",
  "2026-01-01T00:00:00.000Z",
  "2026-01-01T00:00:00.000Z",
  "not-a-date",
];

const arbName = fc.constantFrom(...NAME_POOL);
const arbCreatedAt = fc.constantFrom(...CREATED_AT_POOL);
const arbCount = fc.integer({ min: 0, max: 5 });
const arbWatches = fc.array(fc.integer({ min: 0, max: 2 }).map(watch), { maxLength: 2 });
const arbReviews = fc.integer({ min: 0, max: 2 }).map((n) =>
  Array.from({ length: n }, (_, i): CompanyRow["reviews"][number] => ({
    id: `r${i}`,
    companyId: "c",
    previousHash: null,
    currentHash: "h",
    summary: "changed",
    status: "pending",
    createdAt: "2026-01-01T00:00:00.000Z",
  })),
);
const arbSortOption = fc.constantFrom(...COMPANY_SORT_OPTIONS.map((o) => o.value));

/** Company rows with unique ids (short strings so id tie-breaks are exercised). */
const arbCompanyRows = (maxLength = 30) =>
  fc
    .uniqueArray(fc.string({ unit: fc.constantFrom("a", "b", "c", "A", "1", "_"), minLength: 1, maxLength: 4 }), {
      maxLength,
    })
    .chain((uniqueIds) =>
      fc.tuple(
        ...uniqueIds.map((id) =>
          fc
            .record({
              name: arbName,
              createdAt: arbCreatedAt,
              openPositionCount: arbCount,
              trackedJobCount: arbCount,
              watches: arbWatches,
              reviews: arbReviews,
            })
            .map(({ name, ...rest }) => row(id, name, rest)),
        ),
      ),
    )
    .map((rows): CompanyRow[] => [...rows]);

/** Independent oracle for the Glossary Company_Filter definitions. */
function oracle(r: CompanyRow, f: CompanyFilter): boolean {
  switch (f) {
    case "watched":
      return r.watches.length > 0;
    case "hasOpenPositions":
      return r.openPositionCount > 0;
    case "hasTrackedJobs":
      return r.trackedJobCount > 0;
    case "needsAttention":
      return r.watches.some((w) => w.consecutiveSyncFailures > 0) || r.reviews.length > 0;
  }
}

const ALL_FILTERS: CompanyFilter[] = ["watched", "hasOpenPositions", "hasTrackedJobs", "needsAttention"];
/** Any subset of the four Company_Filters (including empty). */
const arbFilterSet = fc.subarray(ALL_FILTERS).map((fs) => new Set<CompanyFilter>(fs));
const arbFilter = fc.constantFrom(...ALL_FILTERS);
/** Any valid Company_View_State. */
const arbViewState = fc.record({ sort: arbSortOption, filters: arbFilterSet });

/**
 * Search text: empty, whitespace-only, substrings of NAME_POOL with random case and
 * surrounding whitespace, and arbitrary short strings (usually matching nothing).
 */
const arbWhitespace = fc.string({ unit: fc.constantFrom(" ", "\t", "\n"), maxLength: 3 });
const arbSearch = fc.oneof(
  fc.constant(""),
  arbWhitespace,
  fc
    .tuple(arbName, fc.nat(), fc.nat(), fc.array(fc.boolean(), { maxLength: 12 }), arbWhitespace, arbWhitespace)
    .map(([name, a, b, upper, pre, post]) => {
      const start = a % (name.length + 1);
      const end = start + (b % (name.length - start + 1));
      const mixed = [...name.slice(start, end)]
        .map((ch, i) => (upper[i] ? ch.toUpperCase() : ch.toLowerCase()))
        .join("");
      return `${pre}${mixed}${post}`;
    }),
  fc.string({ maxLength: 4 }),
);

describe("company sidebar primitives", () => {
  it("exposes five sort options and four filters in spec order", () => {
    expect(COMPANY_SORT_OPTIONS.map((o) => o.value)).toEqual([
      "nameAsc",
      "nameDesc",
      "openPositionsDesc",
      "trackedJobsDesc",
      "recentlyAdded",
    ]);
    expect(COMPANY_FILTERS.map((f) => f.value)).toEqual([
      "watched",
      "hasOpenPositions",
      "hasTrackedJobs",
      "needsAttention",
    ]);
    expect(DEFAULT_COMPANY_VIEW_STATE.sort).toBe("nameAsc");
    expect(DEFAULT_COMPANY_VIEW_STATE.filters.size).toBe(0);
  });

  it("sorts names case-insensitively with id tie-break, and nameDesc is the exact reverse", () => {
    const rows = [row("3", "beta"), row("2", "AKASA"), row("1", "Abby Care"), row("0", "akasa")];
    const asc = sortCompanies(rows, "nameAsc");
    expect(asc.map((r) => r.company.name)).toEqual(["Abby Care", "akasa", "AKASA", "beta"]);
    expect(ids(sortCompanies(rows, "nameDesc"))).toEqual(ids(asc).reverse());
    expect(ids(rows)).toEqual(["3", "2", "1", "0"]); // input not mutated
  });

  it("sorts by counts descending with Name_Order for ties", () => {
    const rows = [
      row("a", "Zeta", { openPositionCount: 2, trackedJobCount: 0 }),
      row("b", "alpha", { openPositionCount: 0, trackedJobCount: 3 }),
      row("c", "Beta", { openPositionCount: 2, trackedJobCount: 3 }),
    ];
    expect(ids(sortCompanies(rows, "openPositionsDesc"))).toEqual(["c", "a", "b"]);
    expect(ids(sortCompanies(rows, "trackedJobsDesc"))).toEqual(["b", "c", "a"]);
  });

  it("sorts recentlyAdded newest first, invalid dates last", () => {
    const rows = [
      row("old", "Old", { createdAt: "2025-01-01T00:00:00Z" }),
      row("bad2", "B", { createdAt: "nope" }),
      row("new", "New", { createdAt: "2026-06-01T00:00:00Z" }),
      row("bad1", "A", { createdAt: "" }),
    ];
    expect(ids(sortCompanies(rows, "recentlyAdded"))).toEqual(["new", "old", "bad1", "bad2"]);
  });

  it("applies filters as a conjunction and preserves order", () => {
    const rows = [
      row("w", "Watched", { watches: [watch(0)], openPositionCount: 1 }),
      row("f", "Failing", { watches: [watch(2)] }),
      row("r", "Review", { reviews: [{} as CompanyRow["reviews"][number]], trackedJobCount: 1 }),
      row("n", "None"),
    ];
    expect(applyCompanyFilters(rows, new Set())).toEqual(rows);
    expect(ids(applyCompanyFilters(rows, new Set<CompanyFilter>(["watched"])))).toEqual(["w", "f"]);
    expect(ids(applyCompanyFilters(rows, new Set<CompanyFilter>(["needsAttention"])))).toEqual(["f", "r"]);
    expect(ids(applyCompanyFilters(rows, new Set<CompanyFilter>(["hasTrackedJobs"])))).toEqual(["r"]);
    expect(
      ids(applyCompanyFilters(rows, new Set<CompanyFilter>(["watched", "hasOpenPositions"]))),
    ).toEqual(["w"]);
  });

  it("returns sort counts only for count-based sorts", () => {
    const r = row("x", "X", { openPositionCount: 0, trackedJobCount: 4 });
    expect(sortCountFor(r, "openPositionsDesc")).toBe(0);
    expect(sortCountFor(r, "trackedJobsDesc")).toBe(4);
    expect(sortCountFor(r, "nameAsc")).toBeNull();
    expect(sortCountFor(r, "nameDesc")).toBeNull();
    expect(sortCountFor(r, "recentlyAdded")).toBeNull();
  });

  it("toggles one filter at a time and clears filters while keeping sort", () => {
    const start = { sort: "trackedJobsDesc" as const, filters: new Set<CompanyFilter>(["watched"]) };
    const toggled = toggleCompanyFilter(start, "needsAttention");
    expect(toggled.sort).toBe("trackedJobsDesc");
    expect([...toggled.filters].sort()).toEqual(["needsAttention", "watched"]);
    expect(start.filters.size).toBe(1); // not mutated
    expect([...toggleCompanyFilter(toggled, "needsAttention").filters]).toEqual(["watched"]);
    const cleared = clearCompanyFilters(toggled);
    expect(cleared.sort).toBe("trackedJobsDesc");
    expect(cleared.filters.size).toBe(0);
  });

  it("derives the visible list as search → filters → sort", () => {
    const rows = [
      row("1", "Zeta AI", { trackedJobCount: 1 }),
      row("2", "OpenAI", { trackedJobCount: 3, watches: [watch(0)] }),
      row("3", "Beta", { trackedJobCount: 5, watches: [watch(0)] }),
      row("4", "airbase", { watches: [watch(1)] }),
    ];
    const unrefined = deriveCompanyList(rows, "  ", DEFAULT_COMPANY_VIEW_STATE);
    expect(ids(unrefined.visible)).toEqual(["4", "3", "2", "1"]);
    expect(unrefined).toMatchObject({ total: 4, isRefined: false });

    const searched = deriveCompanyList(rows, " AI ", { sort: "trackedJobsDesc", filters: new Set() });
    expect(ids(searched.visible)).toEqual(["2", "1", "4"]);
    expect(searched.isRefined).toBe(true);

    const both = deriveCompanyList(rows, "ai", { sort: "nameAsc", filters: new Set<CompanyFilter>(["watched"]) });
    expect(ids(both.visible)).toEqual(["4", "2"]);
    expect(both).toMatchObject({ total: 4, isRefined: true });
    expect(ids(rows)).toEqual(["1", "2", "3", "4"]); // input not mutated
  });

  it("selects the empty state by precedence", () => {
    const base = { load: "ready" as const, total: 3, visibleCount: 0, search: "", filterCount: 1 };
    expect(companyEmptyState({ ...base, load: "loading", total: 0 })).toBe("loading");
    expect(companyEmptyState({ ...base, load: "error", total: 0 })).toBe("error");
    expect(companyEmptyState({ ...base, total: 0, search: "x" })).toBe("noCompanies");
    expect(companyEmptyState({ ...base, visibleCount: 2, search: "x" })).toBeNull();
    expect(companyEmptyState({ ...base, search: " x " })).toBe("noSearchMatch");
    expect(companyEmptyState({ ...base, search: "   " })).toBe("noFilterMatch");
  });
});

describe("company sidebar module examples", () => {
  it('orders "Abby Care" < "AKASA" < "beta" under nameAsc', () => {
    const rows = [row("b", "beta"), row("k", "AKASA"), row("a", "Abby Care")];
    expect(sortCompanies(rows, "nameAsc").map((r) => r.company.name)).toEqual(["Abby Care", "AKASA", "beta"]);
  });

  it("parseCompanyViewState(null) returns fresh defaults", () => {
    const first = parseCompanyViewState(null);
    expect(first.sort).toBe("nameAsc");
    expect(first.filters.size).toBe(0);
    expect(parseCompanyViewState(null).filters).not.toBe(first.filters);
  });

  it("loadCompanyViewState falls back to defaults for missing or throwing storage", () => {
    const throwing = {
      getItem: (): string | null => {
        throw new Error("SecurityError");
      },
    };
    for (const storage of [undefined, throwing]) {
      const view = loadCompanyViewState(storage);
      expect(view.sort).toBe("nameAsc");
      expect(view.filters.size).toBe(0);
    }
  });

  it("saveCompanyViewState returns false for missing or throwing storage", () => {
    const view: CompanyViewState = { sort: "openPositionsDesc", filters: new Set<CompanyFilter>(["watched"]) };
    const throwing = {
      setItem: (): void => {
        throw new Error("QuotaExceededError");
      },
    };
    expect(saveCompanyViewState(undefined, view)).toBe(false);
    expect(saveCompanyViewState(throwing, view)).toBe(false);
  });

  it("saveCompanyViewState writes under COMPANY_VIEW_STORAGE_KEY and loads back", () => {
    const store = new Map<string, string>();
    const storage = {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => void store.set(key, value),
    };
    const view: CompanyViewState = {
      sort: "trackedJobsDesc",
      filters: new Set<CompanyFilter>(["needsAttention", "watched"]),
    };
    expect(saveCompanyViewState(storage, view)).toBe(true);
    expect([...store.keys()]).toEqual([COMPANY_VIEW_STORAGE_KEY]);
    expect(store.get(COMPANY_VIEW_STORAGE_KEY)).toBe(serializeCompanyViewState(view));
    const loaded = loadCompanyViewState(storage);
    expect(loaded.sort).toBe("trackedJobsDesc");
    expect([...loaded.filters]).toEqual(["watched", "needsAttention"]);
  });
});

describe("company sidebar properties", () => {
  // Feature: companies-sidebar-sort-filter, Property 1: Sort order follows the option's key, then Name_Order
  // **Validates: Requirements 2.5, 2.7, 2.8, 2.9**
  it("Property 1: sort order follows the option's key, then Name_Order", () => {
    // Independent key oracle: higher keys come first; null means name-only.
    const keyOf = (r: CompanyRow, sort: string): number | null => {
      if (sort === "openPositionsDesc") return r.openPositionCount;
      if (sort === "trackedJobsDesc") return r.trackedJobCount;
      if (sort === "recentlyAdded") {
        const ms = new Date(r.company.createdAt).getTime();
        return Number.isFinite(ms) ? ms : Number.NEGATIVE_INFINITY;
      }
      return null;
    };

    fc.assert(
      fc.property(
        arbCompanyRows(),
        arbSortOption.filter((o) => o !== "nameDesc"),
        (rows, sort) => {
          const sorted = sortCompanies(rows, sort);
          for (let i = 1; i < sorted.length; i++) {
            const a = sorted[i - 1];
            const b = sorted[i];
            const ka = keyOf(a, sort);
            const kb = keyOf(b, sort);
            if (ka !== null && kb !== null) {
              expect(ka).toBeGreaterThanOrEqual(kb);
              if (ka !== kb) continue;
            }
            expect(compareByNameOrder(a, b)).toBeLessThan(0);
          }
        },
      ),
      PBT,
    );
  });

  // Feature: companies-sidebar-sort-filter, Property 2: Name Z–A is the exact reverse of Name A–Z
  // **Validates: Requirements 2.6**
  it("Property 2: Name Z–A is the exact reverse of Name A–Z", () => {
    fc.assert(
      fc.property(arbCompanyRows(), (rows) => {
        const asc = sortCompanies(rows, "nameAsc");
        const desc = sortCompanies(rows, "nameDesc");
        expect(ids(desc)).toEqual(ids(asc).reverse());
      }),
      PBT,
    );
  });

  // Feature: companies-sidebar-sort-filter, Property 3: Sorting is a deterministic, idempotent permutation
  // **Validates: Requirements 2.12, 2.13**
  describe("Property 3: sorting is a deterministic, idempotent permutation", () => {
    const sortedIds = (rows: CompanyRow[]) => [...ids(rows)].sort();

    const checkPermutation = (rows: CompanyRow[], shuffled: CompanyRow[], sort: (typeof COMPANY_SORT_OPTIONS)[number]["value"]) => {
      const snapshot = [...rows];
      const snapshotJson = JSON.stringify(rows);
      const once = sortCompanies(rows, sort);

      // Same length and same multiset of ids.
      expect(once).toHaveLength(rows.length);
      expect(sortedIds(once)).toEqual(sortedIds(rows));

      // Idempotence.
      expect(ids(sortCompanies(once, sort))).toEqual(ids(once));

      // Determinism regardless of input order.
      expect(ids(sortCompanies(shuffled, sort))).toEqual(ids(once));
      expect(ids(sortCompanies(rows, sort))).toEqual(ids(once));

      // Input not mutated (same element references, same order, same contents).
      expect(rows).toHaveLength(snapshot.length);
      rows.forEach((r, i) => expect(r).toBe(snapshot[i]));
      expect(JSON.stringify(rows)).toBe(snapshotJson);
    };

    it("holds for small lists and any shuffle", () => {
      fc.assert(
        fc.property(
          arbCompanyRows().chain((rows) =>
            fc.tuple(fc.constant(rows), fc.shuffledSubarray(rows, { minLength: rows.length, maxLength: rows.length })),
          ),
          arbSortOption,
          ([rows, shuffled], sort) => checkPermutation(rows, shuffled, sort),
        ),
        PBT,
      );
    });

    it("holds for a 10,000-row list", () => {
      const fields = fc.sample(
        fc.record({
          name: arbName,
          createdAt: arbCreatedAt,
          openPositionCount: arbCount,
          trackedJobCount: arbCount,
          watches: arbWatches,
          reviews: arbReviews,
        }),
        { numRuns: 10_000, seed: 42 },
      );
      const rows = fields.map(({ name, ...rest }, i) => row(`id-${i}`, name, rest));
      const [shuffled] = fc.sample(
        fc.shuffledSubarray(rows, { minLength: rows.length, maxLength: rows.length }),
        { numRuns: 1, seed: 7 },
      );
      for (const option of COMPANY_SORT_OPTIONS) {
        checkPermutation(rows, shuffled, option.value);
      }
    });
  });

  // Feature: companies-sidebar-sort-filter, Property 4: Filters are a conjunction of the glossary predicates
  // **Validates: Requirements 3.3, 3.4, 3.7**
  it("Property 4: filters are a conjunction of the glossary predicates", () => {
    fc.assert(
      fc.property(arbCompanyRows(), arbFilterSet, (rows, filters) => {
        const result = applyCompanyFilters(rows, filters);
        const expected = rows.filter((r) => [...filters].every((f) => oracle(r, f)));
        expect(ids(result)).toEqual(ids(expected));
        result.forEach((r, i) => expect(r).toBe(expected[i]));
        if (filters.size === 0) expect(ids(result)).toEqual(ids(rows));
      }),
      PBT,
    );
  });

  // Feature: companies-sidebar-sort-filter, Property 5: Toggling flips exactly one filter
  // **Validates: Requirements 3.2**
  it("Property 5: toggling flips exactly one filter", () => {
    fc.assert(
      fc.property(arbViewState, arbFilter, (view, f) => {
        const before = [...view.filters].sort();
        const once = toggleCompanyFilter(view, f);
        expect(once.sort).toBe(view.sort);

        const symDiff = ALL_FILTERS.filter((x) => view.filters.has(x) !== once.filters.has(x));
        expect(symDiff).toEqual([f]);

        const twice = toggleCompanyFilter(once, f);
        expect(twice.sort).toBe(view.sort);
        expect([...twice.filters].sort()).toEqual(before);

        // Input view state is not mutated.
        expect([...view.filters].sort()).toEqual(before);
      }),
      PBT,
    );
  });

  // Feature: companies-sidebar-sort-filter, Property 6: Filter activation order does not matter
  // **Validates: Requirements 3.8**
  it("Property 6: filter activation order does not matter", () => {
    fc.assert(
      fc.property(
        arbCompanyRows(),
        fc
          .subarray(ALL_FILTERS)
          .chain((fs) =>
            fc.tuple(
              fc.shuffledSubarray(fs, { minLength: fs.length, maxLength: fs.length }),
              fc.shuffledSubarray(fs, { minLength: fs.length, maxLength: fs.length }),
            ),
          ),
        arbSearch,
        (rows, [orderA, orderB], search) => {
          const viewA = orderA.reduce(toggleCompanyFilter, DEFAULT_COMPANY_VIEW_STATE);
          const viewB = orderB.reduce(toggleCompanyFilter, DEFAULT_COMPANY_VIEW_STATE);

          // Equivalent view states: same sort, set-equal filters (equal to F itself).
          expect(viewA.sort).toBe(viewB.sort);
          expect([...viewA.filters].sort()).toEqual([...viewB.filters].sort());
          expect([...viewA.filters].sort()).toEqual([...orderA].sort());

          // Identical derived visible lists.
          const visibleA = deriveCompanyList(rows, search, viewA).visible;
          const visibleB = deriveCompanyList(rows, search, viewB).visible;
          expect(ids(visibleA)).toEqual(ids(visibleB));
          visibleA.forEach((r, i) => expect(r).toBe(visibleB[i]));
        },
      ),
      PBT,
    );
  });

  // Feature: companies-sidebar-sort-filter, Property 7: Derived list matches the search + filter + sort model
  // **Validates: Requirements 5.1, 5.2, 5.3, 5.9, 5.10**
  it("Property 7: derived list matches the search + filter + sort model", () => {
    fc.assert(
      fc.property(arbCompanyRows(), arbSearch, arbViewState, (rows, search, view) => {
        const snapshot = ids(rows);
        const derived = deriveCompanyList(rows, search, view);

        // Independent search model (same locale-aware lowercasing as the app's search).
        const query = search.trim().toLocaleLowerCase();
        const model = rows.filter(
          (r) =>
            (query === "" || r.company.name.toLocaleLowerCase().includes(query)) &&
            [...view.filters].every((f) => oracle(r, f)),
        );
        const expected = sortCompanies(model, view.sort);

        expect(ids(derived.visible)).toEqual(ids(expected));
        derived.visible.forEach((r, i) => expect(r).toBe(expected[i]));

        // Every id appears exactly once.
        expect(new Set(ids(derived.visible)).size).toBe(derived.visible.length);

        expect(derived.total).toBe(rows.length);
        expect(derived.isRefined).toBe(view.filters.size > 0 || search.trim() !== "");

        // Input not mutated.
        expect(ids(rows)).toEqual(snapshot);
      }),
      PBT,
    );
  });

  // Feature: companies-sidebar-sort-filter, Property 8: Search and filters commute
  // **Validates: Requirements 5.4**
  it("Property 8: search and filters commute", () => {
    // Local adapter: the real search works on `{ name }` objects, not CompanyRow.
    const search = (rows: CompanyRow[], s: string): CompanyRow[] =>
      filterCompaniesBySearch(
        rows.map((r) => ({ name: r.company.name, row: r })),
        s,
      ).map((w) => w.row);

    fc.assert(
      fc.property(
        arbCompanyRows(),
        arbSearch,
        arbFilterSet,
        arbSortOption,
        (rows, s, filters, sort) => {
          const searchThenFilter = applyCompanyFilters(search(rows, s), filters);
          const filterThenSearch = search(applyCompanyFilters(rows, filters), s);

          expect(ids(searchThenFilter)).toEqual(ids(filterThenSearch));
          searchThenFilter.forEach((r, i) => expect(r).toBe(filterThenSearch[i]));

          const a = sortCompanies(searchThenFilter, sort);
          const b = sortCompanies(filterThenSearch, sort);
          expect(ids(a)).toEqual(ids(b));
          a.forEach((r, i) => expect(r).toBe(b[i]));
        },
      ),
      PBT,
    );
  });

  // Feature: companies-sidebar-sort-filter, Property 9: View state persistence round trip
  // **Validates: Requirements 6.4**
  it("Property 9: view state persistence round trip", () => {
    // A valid view built from a sort option and any sequence of filter toggles.
    const arbToggledView = fc
      .tuple(arbSortOption, fc.array(arbFilter, { maxLength: 12 }))
      .map(([sort, toggles]) =>
        toggles.reduce<CompanyViewState>(toggleCompanyFilter, { ...DEFAULT_COMPANY_VIEW_STATE, sort }),
      );
    const sorted = (s: ReadonlySet<CompanyFilter>) => [...s].sort();

    fc.assert(
      fc.property(fc.oneof(arbToggledView, arbViewState), (view) => {
        const serialized = serializeCompanyViewState(view);
        const parsed = parseCompanyViewState(serialized);

        // Equivalent: same sort, set-equal filters.
        expect(parsed.sort).toBe(view.sort);
        expect(sorted(parsed.filters)).toEqual(sorted(view.filters));

        // Stable: re-serializing the parsed state yields the identical string.
        expect(serializeCompanyViewState(parsed)).toBe(serialized);
      }),
      PBT,
    );

    // Two equivalent states (same sort, same filter set inserted in different orders) serialize identically.
    fc.assert(
      fc.property(
        arbSortOption,
        fc
          .subarray(ALL_FILTERS)
          .chain((fs) =>
            fc.tuple(
              fc.shuffledSubarray(fs, { minLength: fs.length, maxLength: fs.length }),
              fc.shuffledSubarray(fs, { minLength: fs.length, maxLength: fs.length }),
            ),
          ),
        (sort, [orderA, orderB]) => {
          const a: CompanyViewState = { sort, filters: new Set(orderA) };
          const b: CompanyViewState = { sort, filters: new Set(orderB) };
          expect(serializeCompanyViewState(a)).toBe(serializeCompanyViewState(b));
        },
      ),
      PBT,
    );
  });

  // Feature: companies-sidebar-sort-filter, Property 10: Parsing stored state never fails and sanitizes
  // **Validates: Requirements 6.5, 6.3, 2.10**
  it("Property 10: parsing stored state never fails and sanitizes", () => {
    const SORT_SET: ReadonlySet<unknown> = new Set(COMPANY_SORT_OPTIONS.map((o) => o.value));
    const FILTER_SET: ReadonlySet<unknown> = new Set(ALL_FILTERS);

    // Filter-array elements: known filters (possibly duplicated), unknown strings, and arbitrary JSON.
    const arbFilterElement = fc.oneof(arbFilter, arbFilter, fc.string(), fc.jsonValue());
    const arbStoredFilters = fc.oneof(fc.array(arbFilterElement, { maxLength: 12 }), fc.jsonValue());
    const arbStoredSort = fc.oneof(arbSortOption, fc.string(), fc.jsonValue());

    // JSON objects whose `sort` / `filters` fields are arbitrary (and possibly absent).
    const arbStoredObject = fc
      .record(
        {
          version: fc.jsonValue(),
          sort: arbStoredSort,
          filters: arbStoredFilters,
        },
        { requiredKeys: [] },
      )
      .map((obj) => ({ raw: JSON.stringify(obj), obj: obj as Record<string, unknown> | null }));

    // Arbitrary strings (mostly invalid JSON), null, and valid JSON that isn't a non-array object.
    const arbOther = fc.oneof(
      fc.string().map((raw) => ({ raw, obj: null })),
      fc.constant({ raw: null, obj: null }),
      fc
        .jsonValue()
        .filter((v) => typeof v !== "object" || v === null || Array.isArray(v))
        .map((v) => ({ raw: JSON.stringify(v), obj: null })),
    );

    fc.assert(
      fc.property(fc.oneof(arbStoredObject, arbOther), ({ raw, obj }) => {
        let parsed!: CompanyViewState;
        expect(() => {
          parsed = parseCompanyViewState(raw);
        }).not.toThrow();

        // Independently decide whether `raw` is a JSON non-array object (strings may happen to be one).
        let record: Record<string, unknown> | null = obj;
        if (record === null && raw !== null) {
          try {
            const v: unknown = JSON.parse(raw);
            if (typeof v === "object" && v !== null && !Array.isArray(v)) record = v as Record<string, unknown>;
          } catch {
            record = null;
          }
        }

        if (record === null) {
          expect(parsed.sort).toBe(DEFAULT_COMPANY_VIEW_STATE.sort);
          expect(parsed.filters.size).toBe(0);
          return;
        }

        const expectedSort = SORT_SET.has(record.sort) ? record.sort : "nameAsc";
        expect(parsed.sort).toBe(expectedSort);

        const expectedFilters = Array.isArray(record.filters)
          ? ALL_FILTERS.filter((f) => (record.filters as unknown[]).includes(f))
          : [];
        expect([...parsed.filters].sort()).toEqual([...expectedFilters].sort());
        for (const f of parsed.filters) expect(FILTER_SET.has(f)).toBe(true);
      }),
      PBT,
    );
  });

  // Feature: companies-sidebar-sort-filter, Property 11: Empty-state selection follows the precedence rule
  // **Validates: Requirements 7.1, 7.2, 7.4, 7.6**
  it("Property 11: empty-state selection follows the precedence rule", () => {
    const arbInput = fc
      .record({
        load: fc.constantFrom("loading" as const, "ready" as const, "error" as const),
        total: fc.nat({ max: 50 }),
        visibleFrac: fc.double({ min: 0, max: 1, noNaN: true }),
        // Bias toward visibleCount === 0 so the search/filter branches are exercised.
        forceZero: fc.boolean(),
        search: arbSearch,
        filterCount: fc.integer({ min: 0, max: 4 }),
      })
      .map(({ visibleFrac, forceZero, ...rest }) => ({
        ...rest,
        visibleCount: forceZero ? 0 : Math.floor(visibleFrac * rest.total),
      }));
    fc.assert(
      fc.property(arbInput, (input) => {
        const expected =
          input.load === "loading"
            ? "loading"
            : input.load === "error"
              ? "error"
              : input.total === 0
                ? "noCompanies"
                : input.visibleCount > 0
                  ? null
                  : input.search.trim() !== ""
                    ? "noSearchMatch"
                    : "noFilterMatch";
        expect(companyEmptyState(input)).toBe(expected);
      }),
      PBT,
    );
  });
});
