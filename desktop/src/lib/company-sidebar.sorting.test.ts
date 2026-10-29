import fc from "fast-check";
import { describe, expect, it } from "vitest";
import {
  COMPANY_FILTERS,
  COMPANY_SORT_OPTIONS,
  applyCompanyFilters,
  deriveCompanyList,
  matchesCompanyFilter,
  sortAnnotationFor,
  sortCompanies,
  type CompanyFilter,
  type CompanySortOption,
} from "./company-sidebar";
import type { CareersPageReview, CompanyRow, CompanyWatch } from "./schema";
import {
  CREATED_AT_POOL,
  EASE_POOL,
  MALFORMED_COUNT_POOL,
  MALFORMED_EASE_POOL,
  NAME_POOL,
  TIMESTAMP_POOL,
  arbMaybeMalformedRows,
  arbPipelineRows,
  expectedCreatedAtMs,
  expectedInstantNs,
  pipelineRow,
} from "./__fixtures__/company-row-arbitraries";

const PBT = { numRuns: 100 } as const;
const ids = (rows: readonly CompanyRow[]) => rows.map((r) => r.company.id);

// ---------------------------------------------------------------------------
// Reference ordering written from the requirement text (Req 3, 4, 5, 8.5).
// Deliberately independent of the key-vector implementation: no sanitizers,
// no parser (instants come from the hand-written pool), no shared helpers.
// ---------------------------------------------------------------------------

const refCollator = new Intl.Collator(undefined, { sensitivity: "accent" });

/** Name_Order: locale-aware case-insensitive name, then id by code unit. */
function refNameOrder(a: CompanyRow, b: CompanyRow): number {
  const byName = refCollator.compare(a.company.name, b.company.name);
  if (byName !== 0) return byName;
  return a.company.id < b.company.id ? -1 : a.company.id > b.company.id ? 1 : 0;
}

/** Higher value first; equal values fall through. */
function higherFirst(x: number | bigint, y: number | bigint): number {
  return x > y ? -1 : x < y ? 1 : 0;
}

/** "Highest to lowest, Name_Order for equal counts." */
function countDesc(get: (r: CompanyRow) => number) {
  return (a: CompanyRow, b: CompanyRow) => higherFirst(get(a), get(b)) || refNameOrder(a, b);
}

/** "Count ≥ 1 lowest to highest with Name_Order ties, then all zero-count companies in Name_Order." */
function countAscZeroLast(get: (r: CompanyRow) => number) {
  return (a: CompanyRow, b: CompanyRow) => {
    const ca = get(a);
    const cb = get(b);
    if ((ca === 0) !== (cb === 0)) return ca === 0 ? 1 : -1;
    if (ca === 0) return refNameOrder(a, b);
    return (ca < cb ? -1 : ca > cb ? 1 : 0) || refNameOrder(a, b);
  };
}

/** "Valid timestamps newest to oldest with Name_Order ties, then null/invalid in Name_Order." */
function newestFirst(get: (r: CompanyRow) => unknown) {
  return (a: CompanyRow, b: CompanyRow) => {
    const ta = expectedInstantNs(get(a));
    const tb = expectedInstantNs(get(b));
    if ((ta === null) !== (tb === null)) return ta === null ? 1 : -1;
    if (ta === null || tb === null) return refNameOrder(a, b);
    return higherFirst(ta, tb) || refNameOrder(a, b);
  };
}

/** "Non-null averages highest first, then higher Wishlist_Job_Count, then Name_Order; null averages last in Name_Order." */
function easiestWishlist(a: CompanyRow, b: CompanyRow): number {
  const ea = a.wishlistEaseAverage;
  const eb = b.wishlistEaseAverage;
  if ((ea === null) !== (eb === null)) return ea === null ? 1 : -1;
  if (ea === null || eb === null) return refNameOrder(a, b);
  return higherFirst(ea, eb) || higherFirst(a.wishlistJobCount, b.wishlistJobCount) || refNameOrder(a, b);
}

const REFERENCE: Record<CompanySortOption, (a: CompanyRow, b: CompanyRow) => number> = {
  nameAsc: refNameOrder,
  nameDesc: (a, b) => refNameOrder(b, a),
  recentlyAdded: (a, b) =>
    higherFirst(expectedCreatedAtMs(a.company.createdAt), expectedCreatedAtMs(b.company.createdAt)) ||
    refNameOrder(a, b),
  openPositionsDesc: countDesc((r) => r.openPositionCount),
  trackedJobsDesc: countDesc((r) => r.trackedJobCount),
  wishlistJobsDesc: countDesc((r) => r.wishlistJobCount),
  wishlistJobsAsc: countAscZeroLast((r) => r.wishlistJobCount),
  appliedJobsDesc: countDesc((r) => r.appliedJobCount),
  appliedJobsAsc: countAscZeroLast((r) => r.appliedJobCount),
  interviewingJobsDesc: countDesc((r) => r.interviewingJobCount),
  recentActivity: newestFirst((r) => r.lastActivityAt),
  recentlyApplied: newestFirst((r) => r.lastAppliedAt),
  easiestWishlist,
};

const arbSortOption = fc.constantFrom(...COMPANY_SORT_OPTIONS.map((o) => o.value));

describe("companies-tab-sorting sort ordering", () => {
  // Feature: companies-tab-sorting, Property 6: Each Sort_Option orders rows by its specification
  /** **Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 4.1, 4.2, 4.3, 4.4, 4.6, 5.1, 5.2, 5.3, 5.4, 8.5** */
  it("Property 6: each Sort_Option orders rows by its specification", () => {
    fc.assert(
      fc.property(arbPipelineRows(), arbSortOption, (rows, sort) => {
        const expected = ids([...rows].sort(REFERENCE[sort]));
        expect(ids(sortCompanies(rows, sort))).toEqual(expected);
      }),
      PBT,
    );
  });

  // Feature: companies-tab-sorting, Property 6: Each Sort_Option orders rows by its specification
  /** **Validates: Requirements 3.6, 3.7** */
  it("Property 6: Asc and Desc count sorts reverse distinct non-zero counts and keep Name_Order for equal counts", () => {
    const pairs = [
      ["wishlistJobsAsc", "wishlistJobsDesc", (r: CompanyRow) => r.wishlistJobCount],
      ["appliedJobsAsc", "appliedJobsDesc", (r: CompanyRow) => r.appliedJobCount],
    ] as const;
    fc.assert(
      fc.property(arbPipelineRows(), fc.constantFrom(...pairs), (rows, [asc, desc, count]) => {
        const posAsc = new Map(sortCompanies(rows, asc).map((r, i) => [r.company.id, i]));
        const posDesc = new Map(sortCompanies(rows, desc).map((r, i) => [r.company.id, i]));
        for (const a of rows) {
          for (const b of rows) {
            if (a === b) continue;
            const ca = count(a);
            const cb = count(b);
            const aFirstAsc = posAsc.get(a.company.id)! < posAsc.get(b.company.id)!;
            const aFirstDesc = posDesc.get(a.company.id)! < posDesc.get(b.company.id)!;
            if (ca >= 1 && cb >= 1 && ca !== cb) expect(aFirstAsc).toBe(!aFirstDesc);
            if (ca === cb) {
              const aFirstByName = refNameOrder(a, b) < 0;
              expect(aFirstAsc).toBe(aFirstByName);
              expect(aFirstDesc).toBe(aFirstByName);
            }
          }
        }
      }),
      PBT,
    );
  });
});

// ---------------------------------------------------------------------------
// Property 7: malformed aggregates. The sanitized equivalent is built from the
// fixture pools (a field is malformed iff it is missing or drawn from the
// malformed pool), not from the sanitizers under test.
// ---------------------------------------------------------------------------

const COUNT_FIELDS = ["wishlistJobCount", "appliedJobCount", "interviewingJobCount"] as const;

/** Same row with each missing/malformed count → 0 and missing/malformed ease → null. */
function sanitizedEquivalent(row: CompanyRow): CompanyRow {
  const raw = row as unknown as Record<string, unknown>;
  const out: Record<string, unknown> = { ...raw };
  for (const key of COUNT_FIELDS) {
    if (!(key in raw) || MALFORMED_COUNT_POOL.includes(raw[key])) out[key] = 0;
  }
  if (!("wishlistEaseAverage" in raw) || MALFORMED_EASE_POOL.includes(raw.wishlistEaseAverage)) {
    out.wishlistEaseAverage = null;
  }
  return out as unknown as CompanyRow;
}

const NEW_FILTERS: readonly CompanyFilter[] = ["hasWishlistJobs", "hasAppliedJobs"];
const ANNOTATION_CTX = { now: new Date("2026-10-05T12:00:00Z"), locale: "en-US", timeZone: "UTC" };

describe("companies-tab-sorting malformed aggregates", () => {
  // Feature: companies-tab-sorting, Property 7: Malformed aggregates are treated as their sanitized values
  /** **Validates: Requirements 3.10, 5.5** */
  it("Property 7: malformed aggregates are treated as their sanitized values", () => {
    fc.assert(
      fc.property(arbMaybeMalformedRows(), arbSortOption, fc.subarray([...NEW_FILTERS]), (rows, sort, active) => {
        const clean = rows.map(sanitizedEquivalent);

        // Sorting: same id order as the sanitized rows, and every row stays listed.
        const sorted = ids(sortCompanies(rows, sort));
        expect(sorted).toEqual(ids(sortCompanies(clean, sort)));
        expect([...sorted].sort()).toEqual([...ids(rows)].sort());

        // Filtering: the new filters see the sanitized counts.
        rows.forEach((row, i) => {
          for (const filter of NEW_FILTERS) {
            expect(matchesCompanyFilter(row, filter)).toBe(matchesCompanyFilter(clean[i], filter));
          }
        });
        const filters = new Set(active);
        expect(ids(applyCompanyFilters(rows, filters))).toEqual(ids(applyCompanyFilters(clean, filters)));

        // Annotations: identical to the sanitized row's annotation.
        rows.forEach((row, i) => {
          expect(sortAnnotationFor(row, sort, ANNOTATION_CTX)).toEqual(sortAnnotationFor(clean[i], sort, ANNOTATION_CTX));
        });
      }),
      PBT,
    );
  });
});

// ---------------------------------------------------------------------------
// Property 8: sorting is a deterministic, idempotent, non-mutating permutation.
// ---------------------------------------------------------------------------

/** Checks every clause of Property 8 for one input list and one permutation of it. */
function checkSortInvariants(rows: CompanyRow[], permuted: CompanyRow[], sort: CompanySortOption): void {
  const inputRefs = [...rows];
  const snapshot = structuredClone(rows);

  const sorted = sortCompanies(rows, sort);

  // Permutation: same length and same id multiset.
  expect(sorted).toHaveLength(rows.length);
  expect([...ids(sorted)].sort()).toEqual([...ids(rows)].sort());

  // Idempotent: sorting the output again gives the identical sequence.
  expect(ids(sortCompanies(sorted, sort))).toEqual(ids(sorted));

  // Deterministic: any permutation of the input sorts to the identical sequence.
  expect(ids(sortCompanies(permuted, sort))).toEqual(ids(sorted));

  // Non-mutating: input array order, row identities, and every field unchanged.
  expect(rows).toHaveLength(inputRefs.length);
  rows.forEach((row, i) => expect(row).toBe(inputRefs[i]));
  expect(rows).toEqual(snapshot);
}

/** Rows plus a full-length shuffle of the same rows. */
const withPermutation = (arbRows: fc.Arbitrary<CompanyRow[]>) =>
  arbRows.chain((rows) =>
    fc
      .shuffledSubarray(rows, { minLength: rows.length, maxLength: rows.length })
      .map((permuted) => [rows, permuted] as const),
  );

/** Deterministic linear congruential generator (seeded), for the large example. */
function lcg(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state;
  };
}

function pick<T>(next: () => number, pool: readonly T[]): T {
  return pool[next() % pool.length];
}

/** 10,000 rows with unique ids and heavily colliding aggregates, built from a fixed seed. */
function largeRows(count: number, seed: number): CompanyRow[] {
  const next = lcg(seed);
  const rows: CompanyRow[] = [];
  for (let i = 0; i < count; i++) {
    rows.push(
      pipelineRow(`id-${(next() % 1_000_000).toString(36)}-${i}`, pick(next, NAME_POOL), {
        createdAt: pick(next, CREATED_AT_POOL)[0],
        openPositionCount: next() % 4,
        trackedJobCount: next() % 4,
        wishlistJobCount: next() % 4,
        appliedJobCount: next() % 4,
        interviewingJobCount: next() % 4,
        lastActivityAt: next() % 5 === 0 ? null : pick(next, TIMESTAMP_POOL)[0],
        lastAppliedAt: next() % 5 === 0 ? null : pick(next, TIMESTAMP_POOL)[0],
        wishlistEaseAverage: pick(next, EASE_POOL),
      }),
    );
  }
  return rows;
}

/** Seeded Fisher–Yates shuffle into a new array. */
function seededShuffle<T>(items: readonly T[], seed: number): T[] {
  const next = lcg(seed);
  const out = [...items];
  for (let i = out.length - 1; i > 0; i--) {
    const j = next() % (i + 1);
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

describe("companies-tab-sorting sort invariants", () => {
  // Feature: companies-tab-sorting, Property 8: Sorting is a deterministic, idempotent, non-mutating permutation
  /** **Validates: Requirements 3.9, 8.1, 8.2, 8.3, 8.4, 8.7** */
  it("Property 8: sorting is a deterministic, idempotent, non-mutating permutation", () => {
    fc.assert(
      fc.property(withPermutation(arbPipelineRows()), arbSortOption, ([rows, permuted], sort) => {
        checkSortInvariants(rows, permuted, sort);
      }),
      PBT,
    );
  });

  // Feature: companies-tab-sorting, Property 8: Sorting is a deterministic, idempotent, non-mutating permutation
  /** **Validates: Requirements 3.9, 8.1, 8.2, 8.3, 8.4, 8.7** */
  it("Property 8: invariants hold for rows with malformed aggregates", () => {
    fc.assert(
      fc.property(withPermutation(arbMaybeMalformedRows()), arbSortOption, ([rows, permuted], sort) => {
        checkSortInvariants(rows, permuted, sort);
      }),
      PBT,
    );
  });

  // Feature: companies-tab-sorting, Property 8: Sorting is a deterministic, idempotent, non-mutating permutation
  /** **Validates: Requirements 8.1, 8.2, 8.3, 8.4, 8.7** */
  it(
    "Property 8: a seeded 10,000-row list satisfies the invariants under all 13 Sort_Options",
    () => {
      const rows = largeRows(10_000, 0x5eed);
      const permuted = seededShuffle(rows, 0xc0ffee);
      expect(COMPANY_SORT_OPTIONS).toHaveLength(13);
      for (const { value } of COMPANY_SORT_OPTIONS) {
        checkSortInvariants(rows, permuted, value);
      }
    },
    60_000,
  );
});

// ---------------------------------------------------------------------------
// Property 9: the derived list model. Search, filter predicates, and ordering are
// written from the requirement text, not taken from the implementation.
// ---------------------------------------------------------------------------

function watchFixture(companyId: string, index: number, failures: number): CompanyWatch {
  return {
    id: `${companyId}-w${index}`,
    companyId,
    provider: "greenhouse",
    boardSlug: `board-${index}`,
    lastSyncedAt: null,
    consecutiveSyncFailures: failures,
    lastSyncError: failures > 0 ? "boom" : null,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
  };
}

function reviewFixture(companyId: string, index: number): CareersPageReview {
  return {
    id: `${companyId}-r${index}`,
    companyId,
    previousHash: null,
    currentHash: `hash-${index}`,
    summary: "Careers page changed",
    status: "pending",
    createdAt: "2026-01-01T00:00:00.000Z",
  };
}

/** Pipeline rows plus 0–2 watches (each with 0 or some sync failures) and 0–1 pending reviews. */
const arbRowsWithWatches: fc.Arbitrary<CompanyRow[]> = arbPipelineRows().chain((rows) =>
  fc
    .tuple(
      ...rows.map((row) =>
        fc
          .record({
            failures: fc.array(fc.constantFrom(0, 0, 1, 3), { maxLength: 2 }),
            reviewCount: fc.integer({ min: 0, max: 1 }),
          })
          .map(({ failures, reviewCount }): CompanyRow => {
            const id = row.company.id;
            return {
              ...row,
              watches: failures.map((f, i) => watchFixture(id, i, f)),
              reviews: Array.from({ length: reviewCount }, (_, i) => reviewFixture(id, i)),
            };
          }),
      ),
    )
    .map((out): CompanyRow[] => [...out]),
);

/** Search text: empty, whitespace, name fragments in mixed case with padding, or a non-matching string. */
const arbSearch: fc.Arbitrary<string> = fc.oneof(
  fc.constant(""),
  fc.constant("   "),
  fc
    .tuple(fc.constantFrom(...NAME_POOL), fc.nat(), fc.nat(), fc.boolean(), fc.boolean())
    .map(([name, a, b, upper, pad]) => {
      const start = a % name.length;
      const end = start + 1 + (b % (name.length - start));
      const fragment = name.slice(start, end);
      const cased = upper ? fragment.toUpperCase() : fragment.toLowerCase();
      return pad ? `  ${cased} ` : cased;
    }),
  fc.constantFrom("zzz", "xyz", "Acme"),
);

const arbFilterSubset = fc.subarray(COMPANY_FILTERS.map((f) => f.value));

/** Company_Filter predicates from the requirement text (prior spec + Req 7.3, 7.4). */
const REF_FILTER: Record<CompanyFilter, (r: CompanyRow) => boolean> = {
  watched: (r) => r.watches.length > 0,
  hasOpenPositions: (r) => r.openPositionCount > 0,
  hasTrackedJobs: (r) => r.trackedJobCount > 0,
  needsAttention: (r) => r.watches.some((w) => w.consecutiveSyncFailures > 0) || r.reviews.length > 0,
  hasWishlistJobs: (r) => r.wishlistJobCount >= 1,
  hasAppliedJobs: (r) => r.appliedJobCount >= 1,
};

/** Company_Search: case-insensitive substring match on the trimmed query; empty query matches all. */
function refMatchesSearch(row: CompanyRow, search: string): boolean {
  const query = search.trim().toLocaleLowerCase();
  return query === "" || row.company.name.toLocaleLowerCase().includes(query);
}

describe("companies-tab-sorting derived list", () => {
  // Feature: companies-tab-sorting, Property 9: Derived list matches the search, six-filter, and sort model
  /** **Validates: Requirements 3.8, 5.6, 7.3, 7.4, 7.5** */
  it("Property 9: derived list matches the search, six-filter, and sort model", () => {
    fc.assert(
      fc.property(arbRowsWithWatches, arbSearch, arbFilterSubset, arbSortOption, (rows, search, active, sort) => {
        const view = { sort, filters: new Set<CompanyFilter>(active) };
        const derived = deriveCompanyList(rows, search, view);

        const expected = rows
          .filter((r) => refMatchesSearch(r, search) && active.every((f) => REF_FILTER[f](r)))
          .sort(REFERENCE[sort]);

        expect(ids(derived.visible)).toEqual(ids(expected));
        // Each matching company exactly once.
        expect(new Set(ids(derived.visible)).size).toBe(derived.visible.length);
        expect(derived.total).toBe(rows.length);
        expect(derived.isRefined).toBe(active.length > 0 || search.trim() !== "");
      }),
      PBT,
    );
  });
});

describe("companies-tab-sorting filtered relative order", () => {
  // Feature: companies-tab-sorting, Property 10: Filtering preserves strict relative order
  /** **Validates: Requirements 7.6** */
  it("Property 10: filtering preserves strict relative order", () => {
    fc.assert(
      fc.property(arbRowsWithWatches, arbSearch, arbFilterSubset, arbSortOption, (rows, search, active, sort) => {
        const unfiltered = deriveCompanyList(rows, search, { sort, filters: new Set<CompanyFilter>() }).visible;
        const filtered = deriveCompanyList(rows, search, { sort, filters: new Set<CompanyFilter>(active) }).visible;

        const fullPos = new Map(unfiltered.map((r, i) => [r.company.id, i]));
        const byId = new Map(unfiltered.map((r) => [r.company.id, r]));

        // Every visible company is in the unfiltered list.
        for (const r of filtered) expect(fullPos.has(r.company.id)).toBe(true);

        for (let i = 0; i < filtered.length; i++) {
          for (let j = i + 1; j < filtered.length; j++) {
            const a = byId.get(filtered[i].company.id)!;
            const b = byId.get(filtered[j].company.id)!;
            // If B is strictly before A under the active sort, the filtered list must not put A first.
            const bStrictlyFirst =
              fullPos.get(b.company.id)! < fullPos.get(a.company.id)! && REFERENCE[sort](b, a) < 0;
            expect(bStrictlyFirst).toBe(false);
            // Visible order follows unfiltered order.
            expect(fullPos.get(a.company.id)!).toBeLessThan(fullPos.get(b.company.id)!);
          }
        }
      }),
      PBT,
    );
  });
});
