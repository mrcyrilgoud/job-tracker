// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fc from "fast-check";

import { jobStatuses, type JobListItem, type JobListSortField, type JobStatus } from "@/lib/schema";
import { jobStatusPresentation } from "@/lib/ui";

const apiMocks = vi.hoisted(() => ({
  listJobsPage: vi.fn(),
  listJobIds: vi.fn(),
  getJobsDashboard: vi.fn(),
  listCompanies: vi.fn(),
  deleteJob: vi.fn(),
  deleteJobs: vi.fn(),
  unarchiveJobs: vi.fn(),
  reportRefreshFailed: vi.fn(),
  onRunSettled: vi.fn(() => () => {}),
}));

vi.mock("@/lib/api", () => ({ api: apiMocks }));
vi.mock("@/lib/RunMonitorContext", () => ({
  useRunMonitorActions: () => ({ onRunSettled: apiMocks.onRunSettled, reportRefreshFailed: apiMocks.reportRefreshFailed }),
}));

import { JobsPage } from "./JobsPage";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function makeJob(id: string, status: JobStatus, isFavorite: boolean): JobListItem {
  return {
    job: {
      id,
      title: `Role ${id}`,
      status,
      postingState: "active",
      isNewFromWatch: false,
      isFavorite,
      source: "manual",
      appeal: null,
      applicationEase: null,
      appliedAt: null,
      lastCheckedAt: null,
      salaryMin: null,
      salaryMax: null,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    },
    companyName: "Thinking Machines Lab",
  } as JobListItem;
}

function Location() {
  const location = useLocation();
  return createElement("output", { "data-testid": "location-search" }, location.search);
}

type HeaderFilter =
  | { kind: "all" }
  | { kind: "favorites" }
  | { kind: "archived" }
  | { kind: "status"; status: JobStatus };

type HeaderVariant = {
  filter: HeaderFilter;
  jobCount: number;
  hasNextCursor: boolean;
  sortField: JobListSortField;
  isAppliedTab: boolean;
};

const sortFields: JobListSortField[] = ["appliedAt", "postingDate", "appeal", "companyName"];

const headerVariantArb: fc.Arbitrary<HeaderVariant> = fc.record({
  filter: fc.oneof(
    fc.constant<HeaderFilter>({ kind: "all" }),
    fc.constant<HeaderFilter>({ kind: "favorites" }),
    fc.constant<HeaderFilter>({ kind: "archived" }),
    fc.constantFrom(...jobStatuses).map((status): HeaderFilter => ({ kind: "status", status })),
  ),
  jobCount: fc.integer({ min: 0, max: 120 }),
  hasNextCursor: fc.boolean(),
  sortField: fc.constantFrom(...sortFields),
  isAppliedTab: fc.boolean(),
});

function variantToUrl(variant: HeaderVariant): string {
  const params = new URLSearchParams();
  if (variant.isAppliedTab) params.set("status", "applied");
  else if (variant.filter.kind === "status") params.set("status", variant.filter.status);
  if (variant.filter.kind === "favorites") params.set("favorites", "true");
  if (variant.filter.kind === "archived") params.set("archived", "true");
  params.set("sortBy", variant.sortField);
  params.set("sortOrder", "descending");
  const query = params.toString();
  return query ? `/?${query}` : "/";
}

// --- Preservation oracles (copied from the unfixed JobsPage.tsx header expressions) ---

function expectedHeading(params: URLSearchParams): string {
  const status = jobStatuses.find((value) => value === params.get("status"));
  if (params.get("favorites") === "true") return "Favorites Board";
  if (params.get("archived") === "true") return "Archived Postings";
  return status ? jobStatusPresentation(status).label : "All active jobs";
}

function expectedSubtitle(params: URLSearchParams, count: number, hasNext: boolean): string {
  if (params.get("favorites") === "true") {
    if (count === 0) return "No starred roles yet.";
    if (hasNext) return "Showing 100+ priority roles on your favorite board.";
    return `${count} ${count === 1 ? "priority role" : "priority roles"} on your favorite board.`;
  }
  if (params.get("archived") === "true") {
    if (count === 0) return "No archived roles.";
    if (hasNext) return "Showing 100+ archived roles saved for reference.";
    return `${count} ${count === 1 ? "archived role" : "archived roles"} saved for reference.`;
  }
  if (count === 0) return "Nothing here yet.";
  if (hasNext) return "Showing 100+ roles on your radar.";
  return `${count} ${count === 1 ? "role" : "roles"} on your radar.`;
}

type Sort = { field: JobListSortField; direction: "ascending" | "descending" };

function tabDefault(appliedTab: boolean): Sort {
  return appliedTab ? { field: "appliedAt", direction: "descending" } : { field: "postingDate", direction: "descending" };
}

function defaultDirection(field: JobListSortField): Sort["direction"] {
  return field === "companyName" ? "ascending" : "descending";
}

function modelSort(params: URLSearchParams): Sort {
  const appliedTab = params.get("status") === "applied";
  const field = params.get("sortBy") as JobListSortField | null;
  if (field === null || !sortFields.includes(field) || (!appliedTab && field === "appliedAt")) return tabDefault(appliedTab);
  const dir = params.get("sortOrder");
  return { field, direction: dir === "ascending" || dir === "descending" ? dir : defaultDirection(field) };
}

function modelWriteSort(params: URLSearchParams, spec: Sort) {
  params.delete("sort");
  params.delete("appliedSort");
  params.delete("appliedOrder");
  const def = tabDefault(params.get("status") === "applied");
  if (spec.field === def.field && spec.direction === def.direction) {
    params.delete("sortBy");
    params.delete("sortOrder");
  } else {
    params.set("sortBy", spec.field);
    params.set("sortOrder", spec.direction);
  }
}

type HeaderAction =
  | { kind: "field"; field: JobListSortField }
  | { kind: "order"; direction: Sort["direction"] }
  | { kind: "view"; view: "list" | "board" };

const headerActionArb: fc.Arbitrary<HeaderAction> = fc.oneof(
  fc.constantFrom(...sortFields).map((field): HeaderAction => ({ kind: "field", field })),
  fc.constantFrom("ascending" as const, "descending" as const).map((direction): HeaderAction => ({ kind: "order", direction })),
  fc.constantFrom("list" as const, "board" as const).map((view): HeaderAction => ({ kind: "view", view })),
);

function modelApply(params: URLSearchParams, action: HeaderAction): URLSearchParams {
  const next = new URLSearchParams(params);
  if (action.kind === "view") next.set("view", action.view);
  else if (action.kind === "field") modelWriteSort(next, { field: action.field, direction: defaultDirection(action.field) });
  else modelWriteSort(next, { field: modelSort(params).field, direction: action.direction });
  return next;
}

function searchString(params: URLSearchParams): string {
  const s = params.toString();
  return s ? `?${s}` : "";
}

const TOOLBAR_CLASS = "flex flex-wrap items-center gap-2 self-start md:self-auto";
const H1_CLASS = "font-display text-3xl font-semibold tracking-tight";
const SUBTITLE_CLASS = "mt-1 text-sm text-[var(--muted)]";
const TOGGLE_BASE = "inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition-colors";
const TOGGLE_ACTIVE = `${TOGGLE_BASE} bg-[var(--accent-soft)] text-[var(--accent-ink)]`;
const TOGGLE_INACTIVE = `${TOGGLE_BASE} text-[var(--muted)] hover:text-[var(--foreground)]`;
const SORT_HINT = "Based on when the job was added to the tracker.";

const HEADER_CLASSES = ["flex", "flex-col", "gap-4", "md:flex-row", "md:flex-wrap", "md:items-end", "md:justify-between"];
const TITLE_BLOCK_CLASSES = ["max-w-full", "md:shrink-0"];

describe("JobsPage header title/toolbar layout contract", () => {
  let root: Root;
  let host: HTMLDivElement;

  beforeEach(() => {
    for (const mock of Object.values(apiMocks)) mock.mockReset();
    apiMocks.onRunSettled.mockImplementation(() => () => {});
    apiMocks.listJobIds.mockResolvedValue({ ids: [] });
    apiMocks.getJobsDashboard.mockResolvedValue({ counts: { all: 0 }, weeklyActivity: null });
    apiMocks.listCompanies.mockResolvedValue({ companies: [] });
    apiMocks.deleteJob.mockResolvedValue({ success: true, id: "" });
    apiMocks.deleteJobs.mockResolvedValue({ success: true, deletedCount: 0 });
    apiMocks.unarchiveJobs.mockResolvedValue({ success: true, restoredCount: 0 });
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  function mockListFor(variant: HeaderVariant) {
    const status: JobStatus = variant.isAppliedTab
      ? "applied"
      : variant.filter.kind === "status" ? variant.filter.status : "wishlist";
    const jobs = Array.from({ length: variant.jobCount }, (_, i) =>
      makeJob(`job-${i}`, status, variant.filter.kind === "favorites"));
    apiMocks.listJobsPage.mockImplementation(async (filters?: { newFromWatch?: boolean }) => ({
      jobs: filters?.newFromWatch ? [] : jobs,
      nextCursor: variant.hasNextCursor ? "cursor-next" : null,
    }));
    apiMocks.getJobsDashboard.mockResolvedValue({ counts: { all: variant.jobCount }, weeklyActivity: null });
  }

  async function mount(initialEntry: string) {
    await act(async () => {
      root.render(createElement(MemoryRouter, { initialEntries: [initialEntry] },
        createElement(Location),
        createElement(Routes, null,
          createElement(Route, { path: "/", element: createElement(JobsPage) }),
        ),
      ));
      await Promise.resolve();
    });
    await act(async () => { await Promise.resolve(); });
  }

  function headerParts() {
    const h1 = host.querySelector("h1");
    expect(h1).not.toBeNull();
    const titleBlock = h1!.parentElement!.parentElement!;
    const header = titleBlock.parentElement!;
    const addJob = [...host.querySelectorAll("a")].find((a) => a.textContent?.trim() === "+ Add job");
    expect(addJob).toBeDefined();
    const toolbar = addJob!.parentElement!;
    return { titleBlock, header, toolbar };
  }

  /**
   * Property 1: Bug Condition - Title block keeps its natural width when the header row is crowded
   * jsdom has no layout, so this guards the className contract that produces the fix.
   * **Validates: Requirements 1.1, 1.2, 1.3, 1.4, 2.1, 2.2, 2.3, 2.4**
   */
  it("keeps md:flex-wrap on the header and md:shrink-0 + max-w-full on the title block for every variant", async () => {
    await fc.assert(
      fc.asyncProperty(headerVariantArb, async (variant) => {
        mockListFor(variant);
        await mount(variantToUrl(variant));
        const { titleBlock, header, toolbar } = headerParts();
        expect(toolbar.parentElement).toBe(header);
        for (const cls of HEADER_CLASSES) expect(header.classList.contains(cls), `header has ${cls}`).toBe(true);
        for (const cls of TITLE_BLOCK_CLASSES) expect(titleBlock.classList.contains(cls), `title block has ${cls}`).toBe(true);
        await act(async () => root.unmount());
        root = createRoot(host);
      }),
      { numRuns: 25 },
    );
  });

  function currentSearch() {
    return host.querySelector("[data-testid='location-search']")!.textContent ?? "";
  }

  function assertToolbarState(params: URLSearchParams) {
    const { toolbar } = headerParts();
    const appliedTab = params.get("status") === "applied";
    const sort = modelSort(params);
    const sortSelect = host.querySelector<HTMLSelectElement>("#jobs-sort-field")!;
    const orderSelect = host.querySelector<HTMLSelectElement>("#jobs-sort-order")!;
    expect([...sortSelect.options].map((o) => o.value)).toEqual(
      appliedTab ? ["appliedAt", "postingDate", "appeal", "companyName"] : ["postingDate", "appeal", "companyName"],
    );
    expect([...sortSelect.options].some((o) => o.textContent === "Date applied")).toBe(appliedTab);
    expect(sortSelect.value).toBe(sort.field);
    expect(orderSelect.value).toBe(sort.direction);

    const hint = toolbar.querySelector(".jobs-sort-hint");
    if (sort.field === "postingDate") {
      expect(hint?.textContent?.trim()).toBe(SORT_HINT);
      expect(sortSelect.getAttribute("aria-describedby")).toBe(hint!.id);
    } else {
      expect(hint).toBeNull();
      expect(sortSelect.hasAttribute("aria-describedby")).toBe(false);
    }

    const view = params.get("view") ?? (params.get("favorites") === "true" ? "board" : "list");
    const buttons = [...toolbar.querySelectorAll("button")];
    const listButton = buttons.find((b) => b.textContent?.trim() === "List")!;
    const boardButton = buttons.find((b) => b.textContent?.trim() === "Board")!;
    expect(listButton.className).toBe(view === "list" ? TOGGLE_ACTIVE : TOGGLE_INACTIVE);
    expect(boardButton.className).toBe(view === "board" ? TOGGLE_ACTIVE : TOGGLE_INACTIVE);
  }

  /**
   * Property 2: Preservation - heading, subtitle, badges, classes and Add job link are unchanged
   * **Validates: Requirements 3.1, 3.2, 3.4, 3.6, 3.7, 3.8, 3.9**
   */
  it("preserves heading/subtitle strings, badges, header classes and the Add job link for every variant", async () => {
    await fc.assert(
      fc.asyncProperty(headerVariantArb, async (variant) => {
        mockListFor(variant);
        const url = variantToUrl(variant);
        await mount(url);
        const params = new URLSearchParams(url.split("?")[1] ?? "");
        const { titleBlock, toolbar } = headerParts();
        const h1 = titleBlock.querySelector("h1")!;
        const subtitle = titleBlock.querySelector("p")!;

        expect(h1.textContent).toBe(expectedHeading(params));
        expect(subtitle.textContent?.trim()).toBe(expectedSubtitle(params, variant.jobCount, variant.hasNextCursor));
        expect(h1.className).toBe(H1_CLASS);
        expect(subtitle.className).toBe(SUBTITLE_CLASS);
        expect(toolbar.className).toBe(TOOLBAR_CLASS);

        const badge = h1.previousElementSibling;
        if (params.get("favorites") === "true") {
          expect(badge?.classList.contains("bg-amber-100")).toBe(true);
          expect(badge?.querySelector("svg")).not.toBeNull();
        } else if (params.get("archived") === "true") {
          expect(badge?.classList.contains("bg-[var(--surface-muted)]")).toBe(true);
          expect(badge?.querySelector("svg")).not.toBeNull();
        } else {
          expect(badge).toBeNull();
        }

        const addJob = [...toolbar.querySelectorAll("a")].find((a) => a.textContent?.trim() === "+ Add job")!;
        expect(addJob.getAttribute("href")).toBe("/jobs/new");

        assertToolbarState(params);
        await act(async () => root.unmount());
        root = createRoot(host);
      }),
      { numRuns: 25 },
    );
  });

  /**
   * Property 2: Preservation - Sort by / Order / List / Board interactions produce the same URL and styling
   * **Validates: Requirements 3.3, 3.4, 3.5**
   */
  it("preserves location.search and toggle styling across generated Sort by / Order / view interactions", async () => {
    await fc.assert(
      fc.asyncProperty(headerVariantArb, fc.array(headerActionArb, { minLength: 1, maxLength: 6 }), async (variant, actions) => {
        mockListFor(variant);
        const url = variantToUrl(variant);
        await mount(url);
        let params = new URLSearchParams(url.split("?")[1] ?? "");
        expect(currentSearch()).toBe(searchString(params));
        assertToolbarState(params);

        for (const raw of actions) {
          const appliedTab = params.get("status") === "applied";
          const action: HeaderAction = raw.kind === "field" && raw.field === "appliedAt" && !appliedTab
            ? { kind: "field", field: "postingDate" }
            : raw;
          const { toolbar } = headerParts();
          await act(async () => {
            if (action.kind === "view") {
              const label = action.view === "list" ? "List" : "Board";
              [...toolbar.querySelectorAll("button")].find((b) => b.textContent?.trim() === label)!.click();
            } else {
              const select = host.querySelector<HTMLSelectElement>(action.kind === "field" ? "#jobs-sort-field" : "#jobs-sort-order")!;
              select.value = action.kind === "field" ? action.field : action.direction;
              select.dispatchEvent(new Event("change", { bubbles: true }));
            }
          });
          params = modelApply(params, action);
          expect(currentSearch()).toBe(searchString(params));
          assertToolbarState(params);
        }

        await act(async () => root.unmount());
        root = createRoot(host);
      }),
      { numRuns: 25 },
    );
  });
});
