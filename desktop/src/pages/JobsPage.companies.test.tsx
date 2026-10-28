// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { COMPANY_VIEW_STORAGE_KEY } from "@/lib/company-sidebar";
import type { CompanyRow, JobListItem } from "@/lib/schema";

const apiMocks = vi.hoisted(() => ({
  listJobsPage: vi.fn(),
  listJobIds: vi.fn(),
  getJobsDashboard: vi.fn(),
  listCompanies: vi.fn(),
  deleteJob: vi.fn(),
  deleteJobs: vi.fn(),
  unarchiveJobs: vi.fn(),
  reportRefreshFailed: vi.fn(),
  onRunSettled: vi.fn(),
}));

vi.mock("@/lib/api", () => ({ api: apiMocks }));
vi.mock("@/lib/RunMonitorContext", () => ({
  useRunMonitorActions: () => ({ onRunSettled: apiMocks.onRunSettled, reportRefreshFailed: apiMocks.reportRefreshFailed }),
}));

import { JobsPage } from "./JobsPage";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function makeJob(id: string, title: string, companyName: string): JobListItem {
  return {
    job: {
      id,
      title,
      status: "wishlist",
      postingState: "active",
      isNewFromWatch: false,
      isFavorite: false,
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
    companyName,
  } as JobListItem;
}

function makeRow(
  id: string,
  name: string,
  opts: { openPositionCount?: number; trackedJobCount?: number; watched?: boolean; createdAt?: string } = {},
): CompanyRow {
  return {
    company: { id, name, createdAt: opts.createdAt ?? "2026-01-01T00:00:00.000Z" },
    watches: opts.watched ? [{ id: `${id}-watch`, consecutiveSyncFailures: 0 }] : [],
    reviews: [],
    openPositionCount: opts.openPositionCount ?? 0,
    trackedJobCount: opts.trackedJobCount ?? 0,
  } as unknown as CompanyRow;
}

// Name order: Abby Care, AKASA, beta. Tracked: beta > AKASA > Abby Care.
const ROWS: CompanyRow[] = [
  makeRow("c-akasa", "AKASA", { openPositionCount: 3, trackedJobCount: 2, watched: true }),
  makeRow("c-abby", "Abby Care", { openPositionCount: 0, trackedJobCount: 1 }),
  makeRow("c-beta", "beta", { openPositionCount: 5, trackedJobCount: 4, watched: true }),
];

function LocationProbe() {
  const location = useLocation();
  return createElement("output", { "data-testid": "location-search" }, location.search);
}

describe("JobsPage companies sidebar integration", () => {
  let root: Root;
  let host: HTMLDivElement;
  let runSettledCallbacks: Array<() => void>;

  beforeEach(() => {
    // Node's built-in localStorage can shadow jsdom's with a non-functional object; use a fresh in-memory Storage.
    const store = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      get length() { return store.size; },
      key: (index: number) => [...store.keys()][index] ?? null,
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, String(value)); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
    } satisfies Storage);
    runSettledCallbacks = [];
    for (const mock of Object.values(apiMocks)) mock.mockReset();
    apiMocks.onRunSettled.mockImplementation((callback: () => void) => {
      runSettledCallbacks.push(callback);
      return () => {
        runSettledCallbacks = runSettledCallbacks.filter((entry) => entry !== callback);
      };
    });
    apiMocks.listJobsPage.mockImplementation(async (filters?: { companyId?: string }) => ({
      jobs: filters?.companyId === "c-akasa"
        ? [makeJob("akasa-role", "AKASA Platform Engineer", "AKASA")]
        : [makeJob("generic-role", "Generic Engineer", "beta")],
      nextCursor: null,
    }));
    apiMocks.listJobIds.mockResolvedValue({ ids: [] });
    apiMocks.getJobsDashboard.mockResolvedValue({ counts: { all: 7, favorites: 0 }, weeklyActivity: null });
    apiMocks.listCompanies.mockResolvedValue({ companies: ROWS });
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.unstubAllGlobals();
  });

  async function flush() {
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  async function mount(initialEntry = "/") {
    await act(async () => {
      root.render(createElement(MemoryRouter, { initialEntries: [initialEntry] },
        createElement(LocationProbe),
        createElement(Routes, null,
          createElement(Route, { path: "/", element: createElement(JobsPage) }),
        ),
      ));
      await Promise.resolve();
    });
    await flush();
  }

  async function unmountPage() {
    await act(async () => root.unmount());
    root = createRoot(host);
  }

  function locationSearch() {
    return host.querySelector('[data-testid="location-search"]')?.textContent ?? "";
  }

  function companiesSection(): HTMLElement {
    const heading = [...host.querySelectorAll("h2")].find((h) => h.textContent === "Companies")!;
    return heading.closest("section")!;
  }

  function companyLinks(): HTMLAnchorElement[] {
    return [...companiesSection().querySelectorAll<HTMLAnchorElement>("a")]
      .filter((a) => a.textContent !== "Manage");
  }

  function companyNames(): string[] {
    return companyLinks().map((a) => a.querySelector("span > span")?.textContent ?? "");
  }

  function activeCompanyLinks(): HTMLAnchorElement[] {
    return companyLinks().filter((a) => a.className.includes("bg-[var(--accent-soft)]"));
  }

  function button(label: string): HTMLButtonElement {
    return companiesSection().querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
  }

  function menuItem(text: string): HTMLButtonElement {
    return [...companiesSection().querySelectorAll<HTMLButtonElement>('[role^="menuitem"]')]
      .find((item) => item.textContent?.includes(text))!;
  }

  async function click(element: HTMLElement) {
    await act(async () => {
      element.click();
      await Promise.resolve();
    });
  }

  async function selectSort(label: string) {
    await click(button("Sort companies"));
    await click(menuItem(label));
  }

  async function toggleFilter(label: string) {
    if (button("Filter companies").getAttribute("aria-expanded") !== "true") await click(button("Filter companies"));
    await click(menuItem(label));
  }

  async function typeSearch(value: string) {
    const input = companiesSection().querySelector<HTMLInputElement>('input[aria-label="Search companies"]')!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await Promise.resolve();
    });
  }

  function storedView(): { sort: string; filters: string[] } | null {
    const raw = window.localStorage.getItem(COMPANY_VIEW_STORAGE_KEY);
    return raw ? JSON.parse(raw) : null;
  }

  const SCOPED_URL = "/?companyId=c-akasa&status=wishlist&sortBy=companyName&sortOrder=descending";

  it("sort, filter, and search keep every URL param including companyId (Req 5.7)", async () => {
    await mount(SCOPED_URL);
    const before = locationSearch();
    expect(before).toContain("companyId=c-akasa");

    await selectSort("Most tracked jobs");
    expect(companyNames()).toEqual(["beta", "AKASA", "Abby Care"]);
    expect(locationSearch()).toBe(before);

    await toggleFilter("Watched");
    expect(companyNames()).toEqual(["beta", "AKASA"]);
    expect(locationSearch()).toBe(before);

    await typeSearch("aka");
    expect(companyNames()).toEqual(["AKASA"]);
    expect(locationSearch()).toBe(before);
  });

  it("restores the stored view on mount without touching URL params (Req 5.7)", async () => {
    window.localStorage.setItem(
      COMPANY_VIEW_STORAGE_KEY,
      JSON.stringify({ version: 1, sort: "openPositionsDesc", filters: ["hasOpenPositions"] }),
    );
    await mount(SCOPED_URL);

    expect(companyNames()).toEqual(["beta", "AKASA"]);
    expect(button("Filter companies").textContent).toContain("1");
    expect(new URLSearchParams(locationSearch()).toString())
      .toBe(new URLSearchParams(SCOPED_URL.slice(1)).toString());
  });

  it("clicking a company sets companyId and keeps view state and search (Req 5.8)", async () => {
    await mount("/");
    await selectSort("Most tracked jobs");
    await toggleFilter("Watched");
    await click(button("Filter companies")); // close menu
    await typeSearch("a");
    expect(companyNames()).toEqual(["beta", "AKASA"]);

    const akasa = companyLinks().find((a) => a.textContent?.includes("AKASA"))!;
    await click(akasa);
    await flush();

    expect(new URLSearchParams(locationSearch()).get("companyId")).toBe("c-akasa");
    const input = companiesSection().querySelector<HTMLInputElement>('input[aria-label="Search companies"]')!;
    expect(input.value).toBe("a");
    expect(companyNames()).toEqual(["beta", "AKASA"]);
    expect(storedView()).toEqual({ version: 1, sort: "trackedJobsDesc", filters: ["watched"] });
    expect(activeCompanyLinks().map((a) => a.textContent)).toEqual([expect.stringContaining("AKASA")]);
  });

  it("a hidden active company keeps companyId and the scoped jobs list, and marks nothing active (Req 5.12)", async () => {
    await mount(SCOPED_URL);
    expect(activeCompanyLinks()).toHaveLength(1);
    expect(host.textContent).toContain("AKASA Platform Engineer");

    await typeSearch("beta");
    expect(companyNames()).toEqual(["beta"]);
    expect(activeCompanyLinks()).toHaveLength(0);
    expect(new URLSearchParams(locationSearch()).get("companyId")).toBe("c-akasa");
    expect(host.textContent).toContain("AKASA Platform Engineer");
    const lastCall = apiMocks.listJobsPage.mock.calls.at(-1)!;
    expect(lastCall[0]).toMatchObject({ companyId: "c-akasa" });
  });

  it("Reset clears search and filters, keeps sort and URL params (Req 7.3)", async () => {
    await mount(SCOPED_URL);
    const before = locationSearch();
    await selectSort("Name Z–A");
    await toggleFilter("Watched");
    await click(button("Filter companies"));
    await typeSearch("zzz");
    expect(host.textContent).toContain("No matching companies.");

    const reset = [...companiesSection().querySelectorAll<HTMLButtonElement>("button")]
      .find((b) => b.textContent === "Reset")!;
    await click(reset);

    expect(companyNames()).toEqual(["beta", "AKASA", "Abby Care"]);
    expect(storedView()).toEqual({ version: 1, sort: "nameDesc", filters: [] });
    expect(locationSearch()).toBe(before);
    expect(document.activeElement).toBe(
      companiesSection().querySelector('input[aria-label="Search companies"]'),
    );
  });

  it("listCompanies rejection shows the error state, leaves storage unchanged, and the dashboard still renders (Req 7.7)", async () => {
    const stored = JSON.stringify({ version: 1, sort: "recentlyAdded", filters: ["watched"] });
    window.localStorage.setItem(COMPANY_VIEW_STORAGE_KEY, stored);
    apiMocks.listCompanies.mockRejectedValue(new Error("boom"));
    await mount("/");

    expect(companiesSection().querySelector('[role="alert"]')?.textContent).toContain("Couldn");
    expect(companyLinks()).toHaveLength(0);
    expect(companiesSection().textContent).not.toContain("No companies yet.");
    expect(window.localStorage.getItem(COMPANY_VIEW_STORAGE_KEY)).toBe(stored);
    // Dashboard counts still render in the pipeline sidebar, and jobs still list.
    expect(host.textContent).toContain("7");
    expect(host.textContent).toContain("Generic Engineer");
  });

  it("shows no empty-state message while companies are loading (Req 7.6)", async () => {
    apiMocks.listCompanies.mockReturnValue(new Promise(() => {}));
    await mount("/");

    const text = companiesSection().textContent ?? "";
    expect(text).not.toContain("No companies yet.");
    expect(text).not.toContain("No matching companies.");
    expect(text).not.toContain("No companies match the selected filters.");
    expect(companyLinks()).toHaveLength(0);
  });

  it("a reload with new trackedJobCount values re-applies the same sort and filters (Req 2.15, 3.11, 4.5)", async () => {
    await mount("/");
    await selectSort("Most tracked jobs");
    await toggleFilter("Has tracked jobs");
    await click(button("Filter companies"));
    expect(companyNames()).toEqual(["beta", "AKASA", "Abby Care"]);
    expect(companyLinks().map((a) => a.textContent)).toEqual(["beta4", "AKASA2", "Abby Care1"]);

    apiMocks.listCompanies.mockResolvedValue({
      companies: [
        makeRow("c-akasa", "AKASA", { openPositionCount: 3, trackedJobCount: 9, watched: true }),
        makeRow("c-abby", "Abby Care", { openPositionCount: 0, trackedJobCount: 0 }),
        makeRow("c-beta", "beta", { openPositionCount: 5, trackedJobCount: 4, watched: true }),
      ],
    });
    const callsBefore = apiMocks.listCompanies.mock.calls.length;
    expect(runSettledCallbacks.length).toBeGreaterThan(0);
    await act(async () => {
      for (const callback of runSettledCallbacks) callback();
      await Promise.resolve();
    });
    await flush();

    expect(apiMocks.listCompanies.mock.calls.length).toBeGreaterThan(callsBefore);
    expect(companyLinks().map((a) => a.textContent)).toEqual(["AKASA9", "beta4"]);
    expect(button("Filter companies").textContent).toContain("1");
    expect(storedView()).toEqual({ version: 1, sort: "trackedJobsDesc", filters: ["hasTrackedJobs"] });

    // Remount restores the same view against the latest data.
    await unmountPage();
    await mount("/");
    expect(companyLinks().map((a) => a.textContent)).toEqual(["AKASA9", "beta4"]);
  });
});
