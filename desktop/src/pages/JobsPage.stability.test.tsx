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
  updateJob: vi.fn(),
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

function makeJob(id: string, companyName: string, companyStability: number | null): JobListItem {
  return {
    job: {
      id,
      title: `${companyName} role ${id}`,
      status: "wishlist",
      postingState: "active",
      isNewFromWatch: false,
      isFavorite: false,
      source: "manual",
      appeal: null,
      applicationEase: null,
      companyStability,
      appliedAt: null,
      lastCheckedAt: null,
      salaryMin: null,
      salaryMax: null,
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
    },
    companyName,
  } as JobListItem;
}

function makeRow(id: string, name: string, stabilityAverage: number | null, watched: boolean): CompanyRow {
  return {
    company: { id, name, createdAt: "2026-01-01T00:00:00.000Z" },
    watches: watched ? [{ id: `${id}-watch`, consecutiveSyncFailures: 0 }] : [],
    reviews: [],
    openPositionCount: 0,
    trackedJobCount: 1,
    wishlistJobCount: 1,
    appliedJobCount: 0,
    interviewingJobCount: 0,
    lastActivityAt: null,
    lastAppliedAt: null,
    wishlistEaseAverage: null,
    stabilityAverage,
    stabilityScoredJobCount: stabilityAverage === null ? 0 : 1,
  } as unknown as CompanyRow;
}

const JOBS = [makeJob("j1", "AKASA", 5), makeJob("j2", "Abby Care", 3), makeJob("j3", "beta", null)];
// Watched: AKASA, beta. Most stable among watched: AKASA (4.0), beta (2.0).
const ROWS = [
  makeRow("c-akasa", "AKASA", 4, true),
  makeRow("c-abby", "Abby Care", 3, false),
  makeRow("c-beta", "beta", 2, true),
];

function LocationProbe() {
  const location = useLocation();
  return createElement("output", { "data-testid": "location-search" }, location.search);
}

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    get length() { return store.size; },
    key: (index: number) => [...store.keys()][index] ?? null,
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => { store.set(key, String(value)); },
    removeItem: (key: string) => { store.delete(key); },
    clear: () => store.clear(),
  } satisfies Storage);
  for (const mock of Object.values(apiMocks)) mock.mockReset();
  apiMocks.onRunSettled.mockReturnValue(() => {});
  apiMocks.listJobsPage.mockImplementation(async () => ({
    jobs: JOBS.map((item) => ({ ...item, job: { ...item.job } })),
    nextCursor: null,
    totalCount: 2,
  }));
  apiMocks.listJobIds.mockResolvedValue({ ids: [] });
  apiMocks.getJobsDashboard.mockResolvedValue({ counts: { all: 3 }, weeklyActivity: null });
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
        createElement(Route, { path: "/jobs/:id", element: createElement("p", null, "Job details") }),
      ),
    ));
    await Promise.resolve();
  });
  await flush();
}

function locationParams() {
  return new URLSearchParams(host.querySelector('[data-testid="location-search"]')?.textContent ?? "");
}

async function changeSelect(select: HTMLSelectElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")?.set?.call(select, value);
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await Promise.resolve();
  });
}

async function click(element: HTMLElement) {
  await act(async () => {
    element.click();
    await Promise.resolve();
  });
}

function stabilitySelects(): HTMLSelectElement[] {
  return [...host.querySelectorAll<HTMLSelectElement>('select[id^="stability-"]')];
}

function stabilitySelect(jobId: string): HTMLSelectElement {
  return host.querySelector<HTMLSelectElement>(`#stability-${jobId}`)!;
}

function pageAlert(): string | null {
  return [...host.querySelectorAll('[role="alert"]')].map((el) => el.textContent).join("|") || null;
}

describe("JobsPage row stability control and sidebar refresh", () => {
  function companiesSection(): HTMLElement {
    const heading = [...host.querySelectorAll("h2")].find((h) => h.textContent === "Companies")!;
    return heading.closest("section")!;
  }

  function companyLinkText(): string[] {
    return [...companiesSection().querySelectorAll<HTMLAnchorElement>("a")]
      .filter((a) => a.textContent !== "Manage")
      .map((a) => {
        const clone = a.cloneNode(true) as HTMLAnchorElement;
        clone.querySelectorAll(".sr-only").forEach((el) => el.remove());
        return clone.textContent ?? "";
      });
  }

  function sidebarButton(label: string): HTMLButtonElement {
    return companiesSection().querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
  }

  function menuItem(text: string): HTMLButtonElement {
    return [...companiesSection().querySelectorAll<HTMLButtonElement>('[role^="menuitem"]')]
      .find((item) => item.textContent?.includes(text))!;
  }

  async function useMostStableWatchedView() {
    await click(sidebarButton("Sort companies"));
    await click(menuItem("Most stable company"));
    await click(sidebarButton("Filter companies"));
    await click(menuItem("Watched"));
    await click(sidebarButton("Filter companies"));
    expect(companyLinkText()).toEqual(["AKASA4.0", "beta2.0"]);
  }

  it("sends only companyStability, shows the value optimistically, applies the response, and refetches companies once (Req 5.8, 5.9)", async () => {
    await mount("/");
    const jobsCalls = apiMocks.listJobsPage.mock.calls.length;
    const companyCalls = apiMocks.listCompanies.mock.calls.length;
    let resolveUpdate!: (value: unknown) => void;
    apiMocks.updateJob.mockReturnValue(new Promise((resolve) => { resolveUpdate = resolve; }));

    await changeSelect(stabilitySelect("j3"), "4");
    expect(apiMocks.updateJob).toHaveBeenCalledTimes(1);
    expect(apiMocks.updateJob.mock.calls[0]).toEqual(["j3", { companyStability: 4 }]);
    expect(stabilitySelect("j3").value).toBe("4");
    expect(apiMocks.listCompanies.mock.calls.length).toBe(companyCalls);

    // The server's returned value wins over the optimistic one.
    await act(async () => {
      resolveUpdate({ detail: { job: { ...JOBS[2].job, companyStability: 3, updatedAt: "2026-02-01T00:00:00.000Z" } } });
      await Promise.resolve();
    });
    await flush();

    expect(stabilitySelect("j3").value).toBe("3");
    expect(stabilitySelect("j1").value).toBe("5");
    expect(stabilitySelect("j2").value).toBe("3");
    expect(apiMocks.listCompanies.mock.calls.length).toBe(companyCalls + 1);
    expect(apiMocks.listJobsPage.mock.calls.length).toBe(jobsCalls);
    expect(pageAlert()).toBeNull();
  });

  it("keeps the row in place under the stability sort until the next reload (Req 6.13)", async () => {
    await mount("/?sortBy=companyStability&sortOrder=descending");
    const jobsCalls = apiMocks.listJobsPage.mock.calls.length;
    expect(stabilitySelects().map((s) => s.id)).toEqual(["stability-j1", "stability-j2", "stability-j3"]);
    apiMocks.updateJob.mockResolvedValue({ detail: { job: { ...JOBS[2].job, companyStability: 5 } } });

    await changeSelect(stabilitySelect("j3"), "5");
    await flush();

    expect(stabilitySelects().map((s) => [s.id, s.value])).toEqual([
      ["stability-j1", "5"],
      ["stability-j2", "3"],
      ["stability-j3", "5"],
    ]);
    expect(apiMocks.listJobsPage.mock.calls.length).toBe(jobsCalls);
  });

  it("restores the previous value, shows the error, and skips the refetch when the update fails (Req 5.10)", async () => {
    await mount("/");
    const companyCalls = apiMocks.listCompanies.mock.calls.length;
    apiMocks.updateJob.mockRejectedValueOnce(new Error("disk full"));

    await changeSelect(stabilitySelect("j2"), "1");
    await flush();
    expect(stabilitySelect("j2").value).toBe("3");
    expect(pageAlert()).toContain("disk full");

    apiMocks.updateJob.mockRejectedValueOnce("no message");
    await changeSelect(stabilitySelect("j2"), "");
    await flush();
    expect(apiMocks.updateJob.mock.calls[1]).toEqual(["j2", { companyStability: null }]);
    expect(stabilitySelect("j2").value).toBe("3");
    expect(pageAlert()).toContain("Failed to update company stability");
    expect(apiMocks.listCompanies.mock.calls.length).toBe(companyCalls);
  });

  it("re-orders the sidebar from refetched rows and keeps its sort and filters (Req 9.13, 9.14, 9.15)", async () => {
    await mount("/");
    await useMostStableWatchedView();
    apiMocks.updateJob.mockResolvedValue({ detail: { job: { ...JOBS[2].job, companyStability: 5 } } });
    apiMocks.listCompanies.mockResolvedValue({
      companies: [makeRow("c-akasa", "AKASA", 4, true), makeRow("c-abby", "Abby Care", 3, false), makeRow("c-beta", "beta", 4.5, true)],
    });

    await changeSelect(stabilitySelect("j3"), "5");
    await flush();

    expect(companyLinkText()).toEqual(["beta4.5", "AKASA4.0"]);
    expect(sidebarButton("Filter companies").textContent).toContain("1");
    expect(JSON.parse(window.localStorage.getItem(COMPANY_VIEW_STORAGE_KEY)!)).toEqual({
      version: 1,
      sort: "mostStable",
      filters: ["watched"],
    });
  });

  it("a rejected company refetch keeps the old rows and shows no error (Req 9.13)", async () => {
    await mount("/");
    await useMostStableWatchedView();
    apiMocks.updateJob.mockResolvedValue({ detail: { job: { ...JOBS[2].job, companyStability: 5 } } });
    apiMocks.listCompanies.mockRejectedValue(new Error("refetch failed"));

    await changeSelect(stabilitySelect("j3"), "5");
    await flush();

    expect(stabilitySelect("j3").value).toBe("5");
    expect(companyLinkText()).toEqual(["AKASA4.0", "beta2.0"]);
    expect(pageAlert()).toBeNull();
    expect(companiesSection().textContent).not.toContain("Couldn");
  });
});

describe("JobsPage stability sort, minimum filter, and count", () => {
  function sortSelect() {
    return host.querySelector<HTMLSelectElement>("#jobs-sort-field")!;
  }

  async function openFilters() {
    const trigger = [...host.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.getAttribute("aria-controls") === "jobs-filter-popover")!;
    await click(trigger);
    return host.querySelector<HTMLElement>('[role="dialog"][aria-labelledby="jobs-filter-heading"]')!;
  }

  async function applyMinStability(value: string) {
    const popover = await openFilters();
    await changeSelect(popover.querySelector<HTMLSelectElement>("#jobs-min-stability")!, value);
    await click(popover.querySelector<HTMLButtonElement>('button[type="submit"]')!);
    await flush();
  }

  function lastListCall() {
    return apiMocks.listJobsPage.mock.calls.at(-1)!;
  }

  it.each(["/", "/?status=wishlist", "/?status=applied", "/?favorites=true&view=list", "/?archived=true"])(
    "offers one Company stability sort option and writes it to the URL on %s (Req 6.1, 6.9)",
    async (entry) => {
      await mount(entry);
      const matches = [...sortSelect().options].filter((o) => o.textContent === "Company stability");
      expect(matches.map((o) => o.value)).toEqual(["companyStability"]);

      await changeSelect(sortSelect(), "companyStability");
      await flush();
      expect(locationParams().get("sortBy")).toBe("companyStability");
      expect(locationParams().get("sortOrder")).toBe("descending");
      expect(sortSelect().value).toBe("companyStability");
      expect(lastListCall()[3]).toEqual({ field: "companyStability", direction: "descending" });
    },
  );

  it("offers Any, 1+ … 5 and writes minStability, requesting the first page (Req 7.1, 7.5, 7.14)", async () => {
    await mount("/?sortBy=companyStability&sortOrder=ascending");
    const popover = await openFilters();
    const select = popover.querySelector<HTMLSelectElement>("#jobs-min-stability")!;
    expect([...select.options].map((o) => [o.value, o.textContent])).toEqual([
      ["", "Any"], ["1", "1+"], ["2", "2+"], ["3", "3+"], ["4", "4+"], ["5", "5"],
    ]);
    expect(select.value).toBe("");
    await click(popover.querySelector<HTMLButtonElement>('button[type="submit"]')!);

    await applyMinStability("3");
    expect(locationParams().get("minStability")).toBe("3");
    expect(locationParams().get("sortBy")).toBe("companyStability");
    expect(lastListCall()[0]).toMatchObject({ minCompanyStability: 3 });
    expect(lastListCall()[1] ?? null).toBeNull();

    await applyMinStability("");
    expect(locationParams().has("minStability")).toBe(false);
    expect(lastListCall()[0].minCompanyStability).toBeUndefined();
    expect(lastListCall()[1] ?? null).toBeNull();
  });

  it("Clear all resets the minimum filter and removes minStability (Req 7.9)", async () => {
    await mount("/?minStability=4");
    const popover = await openFilters();
    expect(popover.querySelector<HTMLSelectElement>("#jobs-min-stability")!.value).toBe("4");
    const clearAll = [...popover.querySelectorAll<HTMLAnchorElement>("a")].find((a) => a.textContent === "Clear all")!;
    await click(clearAll);
    await flush();

    expect(locationParams().has("minStability")).toBe(false);
    expect(lastListCall()[0].minCompanyStability).toBeUndefined();
    const reopened = await openFilters();
    expect(reopened.querySelector<HTMLSelectElement>("#jobs-min-stability")!.value).toBe("");
  });

  it("shows the totalCount while the minimum filter is set, and hides it when unfiltered (Req 7.13)", async () => {
    await mount("/?minStability=3");
    expect(host.textContent).toContain("2 matching jobs");

    await act(async () => root.unmount());
    root = createRoot(host);
    await mount("/");
    expect(host.textContent).not.toContain("matching jobs");
  });
});
