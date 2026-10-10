// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { JobListItem } from "@/lib/schema";

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

function makeJob(id: string, title: string, companyName = "Thinking Machines Lab"): JobListItem {
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

function makeArchivedJob(
  id: string,
  title: string,
  status: JobListItem["job"]["status"] = "archived",
): JobListItem {
  const item = makeJob(id, title);
  return { ...item, job: { ...item.job, status } };
}

function setupArchivedRows(rows: JobListItem[]) {
  let currentRows = rows;
  const matchingFor = (filters?: {
    newFromWatch?: boolean;
    isArchived?: boolean;
    search?: string;
  }) => {
    if (filters?.newFromWatch) return [];
    const matchingRows = filters?.isArchived ? currentRows : [makeJob("sandbox-role", "Software Engineer, Sandboxing")];
    const query = filters?.search?.toLowerCase();
    return query ? matchingRows.filter(({ job, companyName }) =>
      `${job.title} ${companyName}`.toLowerCase().includes(query)) : matchingRows;
  };
  apiMocks.listJobsPage.mockImplementation(async (filters) => ({ jobs: matchingFor(filters), nextCursor: null }));
  apiMocks.listJobIds.mockImplementation(async (filters) => ({ ids: matchingFor(filters).map(({ job }) => job.id) }));
  apiMocks.unarchiveJobs.mockImplementation(async (ids: string[]) => {
    currentRows = currentRows.filter((item) => !ids.includes(item.job.id));
    return { success: true, restoredCount: ids.length };
  });
  apiMocks.deleteJobs.mockImplementation(async (ids: string[]) => {
    currentRows = currentRows.filter((item) => !ids.includes(item.job.id));
    return { success: true, deletedCount: ids.length };
  });
  return { getRows: () => currentRows };
}

function Location() {
  const location = useLocation();
  const navigate = useNavigate();
  return createElement("div", null,
    createElement("output", { "data-testid": "location" }, location.pathname),
    createElement("output", { "data-testid": "location-search" }, location.search),
    createElement("button", { type: "button", "data-testid": "history-back", onClick: () => navigate(-1) }, "Back"),
    createElement("button", { type: "button", "data-testid": "history-forward", onClick: () => navigate(1) }, "Forward"),
  );
}

describe("JobsPage live search suggestions", () => {
  let root: Root;
  let host: HTMLDivElement;

  beforeEach(() => {
    vi.useFakeTimers();
    apiMocks.listJobsPage.mockReset();
    apiMocks.listJobIds.mockReset();
    apiMocks.getJobsDashboard.mockReset();
    apiMocks.listCompanies.mockReset();
    apiMocks.deleteJob.mockReset();
    apiMocks.deleteJobs.mockReset();
    apiMocks.unarchiveJobs.mockReset();
    apiMocks.deleteJob.mockResolvedValue({ success: true, id: "sandbox-role" });
    apiMocks.deleteJobs.mockResolvedValue({ success: true, deletedCount: 0 });
    apiMocks.unarchiveJobs.mockResolvedValue({ success: true, restoredCount: 0 });
    apiMocks.listJobsPage.mockImplementation(async (filters?: { newFromWatch?: boolean; isArchived?: boolean }) => ({
      jobs: filters?.newFromWatch ? [] : [makeJob("sandbox-role", "Software Engineer, Sandboxing")],
      nextCursor: null,
    }));
    apiMocks.listJobIds.mockResolvedValue({ ids: [] });
    apiMocks.getJobsDashboard.mockResolvedValue({ counts: { all: 1 }, weeklyActivity: null });
    apiMocks.listCompanies.mockResolvedValue({ companies: [] });
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.useRealTimers();
  });

  async function mount(initialEntry = "/") {
    await act(async () => {
      root.render(createElement(MemoryRouter, { initialEntries: [initialEntry] },
        createElement(Location),
        createElement(Routes, null,
          createElement(Route, { path: "/", element: createElement(JobsPage) }),
          createElement(Route, { path: "/jobs/:id", element: createElement("p", null, "Job details") }),
        ),
      ));
      await Promise.resolve();
    });
    await act(async () => { await Promise.resolve(); });
  }

  function typeInto(input: HTMLInputElement, value: string) {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }

  function selectValue(select: HTMLSelectElement, value: string) {
    const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")?.set;
    setter?.call(select, value);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  }

  it("waits for the debounce and renders matching suggestions from the filtered jobs response", async () => {
    await mount();
    const input = host.querySelector<HTMLInputElement>('input[aria-label="Search jobs"]')!;
    const initialSearchCalls = apiMocks.listJobsPage.mock.calls.filter(([filters]) => !filters?.newFromWatch).length;
    const dashboardCalls = apiMocks.getJobsDashboard.mock.calls.length;
    const companyCalls = apiMocks.listCompanies.mock.calls.length;

    await act(async () => {
      input.focus();
      typeInto(input, "san");
      await vi.advanceTimersByTimeAsync(249);
    });
    expect(apiMocks.listJobsPage.mock.calls.filter(([filters]) => !filters?.newFromWatch)).toHaveLength(initialSearchCalls);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
      await Promise.resolve();
    });
    expect(apiMocks.listJobsPage.mock.calls.filter(([filters]) => !filters?.newFromWatch)).toHaveLength(initialSearchCalls + 1);
    expect(host.querySelector('[role="option"]')?.textContent).toContain("Software Engineer, Sandboxing");
    expect(host.textContent).toContain("Thinking Machines Lab");
    expect(apiMocks.getJobsDashboard).toHaveBeenCalledTimes(dashboardCalls);
    expect(apiMocks.listCompanies).toHaveBeenCalledTimes(companyCalls);
  });

  it("does not fetch watch discoveries on the Jobs page", async () => {
    await mount();

    const fetchedWatchRoles = apiMocks.listJobsPage.mock.calls.some(([filters]) =>
      typeof filters === "object" && filters !== null &&
      "newFromWatch" in filters && filters.newFromWatch === true,
    );
    expect(fetchedWatchRoles).toBe(false);
    expect(host.textContent).not.toContain("new roles from your watches");
  });

  it("does not show suggestions before three characters and opens a selected job with the keyboard", async () => {
    await mount();
    const input = host.querySelector<HTMLInputElement>('input[aria-label="Search jobs"]')!;
    await act(async () => {
      input.focus();
      typeInto(input, "sa");
      await vi.advanceTimersByTimeAsync(250);
      await Promise.resolve();
    });
    expect(host.querySelector('[role="option"]')).toBeNull();

    await act(async () => {
      typeInto(input, "sandbox");
      await vi.advanceTimersByTimeAsync(250);
      await Promise.resolve();
    });
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(input.value).toBe("sandbox");
    expect(input.getAttribute("aria-expanded")).toBe("false");
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    });
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await Promise.resolve();
    });
    expect(host.querySelector('[data-testid="location"]')?.textContent).toBe("/jobs/sandbox-role");
  });

  it("caps the suggestion menu at five and opens a suggestion by mouse", async () => {
    apiMocks.listJobsPage.mockImplementation(async (filters?: { newFromWatch?: boolean }) => ({
      jobs: filters?.newFromWatch ? [] : Array.from({ length: 8 }, (_, index) =>
        makeJob(`role-${index}`, `Sandbox Role ${index}`)),
      nextCursor: null,
    }));
    await mount();
    const input = host.querySelector<HTMLInputElement>('input[aria-label="Search jobs"]')!;
    await act(async () => {
      input.focus();
      typeInto(input, "sandbox");
      await vi.advanceTimersByTimeAsync(250);
      await Promise.resolve();
    });
    expect(host.querySelectorAll('[role="option"]')).toHaveLength(5);
    await act(async () => {
      host.querySelector<HTMLButtonElement>('[role="option"]')!.click();
    });
    expect(host.querySelector('[data-testid="location"]')?.textContent).toBe("/jobs/role-0");
  });

  it("coalesces rapid edits while one search is pending and ignores the stale result", async () => {
    let resolveSan!: (value: { jobs: JobListItem[]; nextCursor: null }) => void;
    let inFlight = 0;
    let peakInFlight = 0;
    apiMocks.listJobsPage.mockImplementation((filters?: { newFromWatch?: boolean; search?: string }) => {
      if (filters?.newFromWatch) return Promise.resolve({ jobs: [], nextCursor: null });
      if (filters?.search === "san") {
        inFlight += 1;
        peakInFlight = Math.max(peakInFlight, inFlight);
        return new Promise((resolve) => {
          resolveSan = (value) => { inFlight -= 1; resolve(value); };
        });
      }
      if (filters?.search === "sand") {
        inFlight += 1;
        peakInFlight = Math.max(peakInFlight, inFlight);
        return Promise.resolve({ jobs: [makeJob("latest", "Latest result")], nextCursor: null }).finally(() => { inFlight -= 1; });
      }
      return Promise.resolve({ jobs: [makeJob("sandbox-role", "Software Engineer, Sandboxing")], nextCursor: null });
    });
    await mount();
    const input = host.querySelector<HTMLInputElement>('input[aria-label="Search jobs"]')!;
    await act(async () => {
      input.focus();
      typeInto(input, "san");
      await vi.advanceTimersByTimeAsync(250);
    });
    expect(resolveSan).toBeTypeOf("function");
    await act(async () => {
      typeInto(input, "sand");
      await vi.advanceTimersByTimeAsync(250);
    });
    expect(apiMocks.listJobsPage.mock.calls.filter(([filters]) => filters?.search === "sand")).toHaveLength(0);

    await act(async () => {
      resolveSan({ jobs: [makeJob("stale", "Stale result")], nextCursor: null });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(apiMocks.listJobsPage.mock.calls.filter(([filters]) => filters?.search === "sand")).toHaveLength(1);
    await act(async () => { await Promise.resolve(); });
    expect(peakInFlight).toBe(1);
    expect(host.textContent).toContain("Latest result");
    expect(host.textContent).not.toContain("Stale result");
  });

  it("appends cursor pages and resets to the first page when the search changes", async () => {
    const rows = Array.from({ length: 150 }, (_, index) =>
      makeJob(`paged-${String(index).padStart(3, "0")}`, index === 149 ? "Rare role" : `Paged role ${index}`));
    rows.forEach((item, index) => { item.job.updatedAt = `2026-01-01T00:${String(Math.floor(index / 60)).padStart(2, "0")}:${String(index % 60).padStart(2, "0")}.000Z`; });
    apiMocks.listJobsPage.mockImplementation(async (filters, cursor) => {
      const matching = [...(filters?.search ? rows.filter(({ job }) => job.title.toLowerCase().includes(String(filters.search).toLowerCase())) : rows)]
        .sort((left, right) => right.job.updatedAt.localeCompare(left.job.updatedAt) || right.job.id.localeCompare(left.job.id));
      if (filters?.search) return { jobs: matching, nextCursor: null };
      const start = cursor ? matching.findIndex(({ job }) => job.id === cursor.id) + 1 : 0;
      const page = matching.slice(start, start + 100);
      const last = page.at(-1);
      return {
        jobs: page,
        nextCursor: start + page.length < matching.length && last
          ? { updatedAt: last.job.updatedAt, id: last.job.id }
          : null,
      };
    });
    await mount();
    expect(host.querySelectorAll("ul.space-y-3 > li")).toHaveLength(100);
    const loadMore = [...host.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("Load more"))!;
    await act(async () => {
      loadMore.click();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(host.querySelectorAll("ul.space-y-3 > li")).toHaveLength(150);
    expect(apiMocks.listJobsPage.mock.calls.at(-1)?.[1]).toMatchObject({ id: "paged-050" });

    const input = host.querySelector<HTMLInputElement>('input[aria-label="Search jobs"]')!;
    await act(async () => {
      input.focus();
      typeInto(input, "rare");
      await vi.advanceTimersByTimeAsync(250);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(apiMocks.listJobsPage.mock.calls.at(-1)?.[0]?.search).toBe("rare");
    expect(apiMocks.listJobsPage.mock.calls.at(-1)?.[1]).toBeNull();
    expect(host.querySelectorAll("ul.space-y-3 > li")).toHaveLength(1);
    expect(host.textContent).toContain("Rare role");
  });

  it("keeps filter drafts on dismiss and applies them while preserving the current jobs scope", async () => {
    await mount("/?search=engineer&status=applied&companyId=company-1&view=board&salaryMin=100000&salaryMax=200000&postingState=active");
    const trigger = host.querySelector<HTMLButtonElement>('button[aria-label="Filters, 2 active"]')!;
    expect(trigger.textContent).toContain("2");

    await act(async () => trigger.click());
    const popover = host.querySelector<HTMLElement>('[role="dialog"][aria-labelledby="jobs-filter-heading"]')!;
    const minimum = popover.querySelector<HTMLInputElement>("#salary-min")!;
    const posting = popover.querySelector<HTMLSelectElement>("#posting-state")!;
    expect(minimum.value).toBe("100000");
    expect(popover.querySelector<HTMLInputElement>("#salary-max")?.value).toBe("200000");
    expect(popover.querySelector<HTMLSelectElement>("#posting-state")?.value).toBe("active");

    await act(async () => {
      typeInto(minimum, "125000");
      selectValue(posting, "");
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    expect(trigger.getAttribute("aria-label")).toBe("Filters, 2 active");

    await act(async () => trigger.click());
    expect(host.querySelector<HTMLInputElement>("#salary-min")?.value).toBe("125000");
    expect(host.querySelector<HTMLSelectElement>("#posting-state")?.value).toBe("");
    await act(async () => {
      host.querySelector<HTMLButtonElement>('[role="dialog"] button[type="submit"]')!.click();
      await Promise.resolve();
    });

    const applied = new URLSearchParams(host.querySelector('[data-testid="location-search"]')?.textContent ?? "");
    expect(applied.get("search")).toBe("engineer");
    expect(applied.get("salaryMin")).toBe("125000");
    expect(applied.get("salaryMax")).toBe("200000");
    expect(applied.get("postingState")).toBeNull();
    expect(applied.get("status")).toBe("applied");
    expect(applied.get("companyId")).toBe("company-1");
    expect(applied.get("view")).toBe("board");
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="Filters, 1 active"]')).not.toBeNull();

    await act(async () => host.querySelector<HTMLButtonElement>('[data-testid="history-back"]')!.click());
    expect(host.querySelector<HTMLInputElement>("#salary-min")).toBeNull();
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="Filters, 2 active"]')).not.toBeNull();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-testid="history-forward"]')!.click());
    await act(async () => trigger.click());
    expect(host.querySelector<HTMLInputElement>("#salary-min")?.value).toBe("125000");
    expect(host.querySelector<HTMLSelectElement>("#posting-state")?.value).toBe("");
  });

  it("validates annual income bounds before applying them", async () => {
    await mount();
    const trigger = host.querySelector<HTMLButtonElement>('button[aria-label="Filters"]')!;
    await act(async () => trigger.click());

    const minimum = host.querySelector<HTMLInputElement>("#salary-min")!;
    const maximum = host.querySelector<HTMLInputElement>("#salary-max")!;
    expect(host.querySelector('[role="dialog"] legend')?.textContent).toBe("Annual income (USD)");
    expect(minimum.step).toBe("1");

    await act(async () => {
      typeInto(minimum, "180000");
      typeInto(maximum, "120000");
      host.querySelector<HTMLButtonElement>('[role="dialog"] button[type="submit"]')!.click();
    });
    expect(host.querySelector('[role="alert"]')?.textContent).toBe("Minimum income must not exceed maximum income.");
    expect(host.querySelector('[data-testid="location-search"]')?.textContent).toBe("");

    await act(async () => {
      typeInto(minimum, "-1");
      host.querySelector<HTMLButtonElement>('[role="dialog"] button[type="submit"]')!.click();
    });
    expect(host.querySelector('[role="alert"]')?.textContent).toBe("Enter whole-dollar amounts of 0 or more.");

    await act(async () => {
      typeInto(minimum, "100.50");
      host.querySelector<HTMLButtonElement>('[role="dialog"] button[type="submit"]')!.click();
    });
    expect(host.querySelector('[role="alert"]')?.textContent).toBe("Enter whole-dollar amounts of 0 or more.");

    await act(async () => typeInto(maximum, "200000"));
    await act(async () => typeInto(minimum, "180000"));
    expect(host.querySelector('[role="alert"]')).toBeNull();
    await act(async () => {
      host.querySelector<HTMLButtonElement>('[role="dialog"] button[type="submit"]')!.click();
      await Promise.resolve();
    });

    const applied = new URLSearchParams(host.querySelector('[data-testid="location-search"]')?.textContent ?? "");
    expect(applied.get("salaryMin")).toBe("180000");
    expect(applied.get("salaryMax")).toBe("200000");
    expect(apiMocks.listJobsPage.mock.calls.some(([filters]) =>
      filters?.salaryMin === 180000 && filters?.salaryMax === 200000,
    )).toBe(true);
  });

  it("closes on outside click, keeps drafts, and Clear all resets the full jobs view", async () => {
    await mount("/?search=engineer&status=applied&companyId=company-1&view=board&salaryMin=100000&postingState=active");
    const trigger = host.querySelector<HTMLButtonElement>('button[aria-label="Filters, 2 active"]')!;

    await act(async () => trigger.click());
    const maximum = host.querySelector<HTMLInputElement>("#salary-max")!;
    await act(async () => {
      typeInto(maximum, "180000");
      document.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    });
    expect(host.querySelector('[role="dialog"]')).toBeNull();

    await act(async () => trigger.click());
    expect(host.querySelector<HTMLInputElement>("#salary-max")?.value).toBe("180000");
    await act(async () => {
      host.querySelector<HTMLAnchorElement>('[role="dialog"] a')!.click();
      await Promise.resolve();
    });

    expect(host.querySelector('[role="dialog"]')).toBeNull();
    expect(host.querySelector('[data-testid="location"]')?.textContent).toBe("/");
    expect(host.querySelector('[data-testid="location-search"]')?.textContent).toBe("");
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="Filters"]')?.textContent).toBe("Filters");
  });

  it("selects archived list rows and clears the selection as the search scope changes", async () => {
    setupArchivedRows([
      makeArchivedJob("role-1", "Software Engineer"),
      makeArchivedJob("role-2", "Research Engineer"),
    ]);
    await mount("/?archived=true");

    const firstCheckbox = host.querySelector<HTMLInputElement>('input[aria-label="Select Software Engineer at Thinking Machines Lab"]')!;
    const selectAll = host.querySelector<HTMLInputElement>('input[aria-label="Select all matching archived postings"]')!;
    await act(async () => firstCheckbox.click());

    expect(firstCheckbox.checked).toBe(true);
    expect(selectAll.checked).toBe(false);
    expect(selectAll.indeterminate).toBe(true);
    expect(host.textContent).toContain("1 selected");
    expect(firstCheckbox.closest("li")?.querySelector("a")?.classList.contains("is-selected")).toBe(true);

    const search = host.querySelector<HTMLInputElement>('input[aria-label="Search jobs"]')!;
    await act(async () => {
      search.focus();
      typeInto(search, "research");
      await Promise.resolve();
    });
    expect(host.textContent).not.toContain("1 selected");
    expect(firstCheckbox.closest("li")?.querySelector("a")?.classList.contains("is-selected")).toBe(false);
  });

  it("selects all archived board cards and restores them in one batch", async () => {
    const archived = setupArchivedRows([
      makeArchivedJob("role-1", "Software Engineer"),
      makeArchivedJob("role-2", "Research Engineer", "closed"),
    ]);
    await mount("/?archived=true&view=board");

    const selectAll = host.querySelector<HTMLInputElement>('input[aria-label="Select all matching archived postings"]')!;
    await act(async () => selectAll.click());
    expect(host.textContent).toContain("2 selected");
    expect(host.querySelectorAll<HTMLInputElement>('input[aria-label^="Select "][aria-label*=" at "]')).toHaveLength(2);
    expect(host.querySelector<HTMLInputElement>('input[aria-label="Select Software Engineer at Thinking Machines Lab"]')?.parentElement?.classList.contains("bg-[var(--accent-soft)]")).toBe(true);

    const restore = [...host.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("Restore selected"))!;
    await act(async () => {
      restore.click();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(apiMocks.unarchiveJobs).toHaveBeenCalledWith(["role-1", "role-2"]);
    expect(archived.getRows()).toHaveLength(0);
    expect(host.textContent).not.toContain("2 selected");
  });

  it("selects archived matching IDs beyond the loaded page for bulk restore", async () => {
    const rows = Array.from({ length: 105 }, (_, index) =>
      makeArchivedJob(`archive-${String(index).padStart(3, "0")}`, `Archived role ${index}`));
    apiMocks.listJobsPage.mockImplementation(async (_filters, cursor) => ({
      jobs: cursor ? rows.slice(100) : rows.slice(0, 100),
      nextCursor: cursor ? null : { updatedAt: rows[99].job.updatedAt, id: rows[99].job.id },
    }));
    apiMocks.listJobIds.mockResolvedValue({ ids: rows.map(({ job }) => job.id) });
    await mount("/?archived=true");
    expect(host.querySelectorAll("ul.space-y-3 > li")).toHaveLength(100);
    const firstRow = host.querySelector<HTMLInputElement>('input[aria-label="Select Archived role 0 at Thinking Machines Lab"]')!;
    await act(async () => firstRow.click());
    expect(host.textContent).toContain("1 selected");
    await act(async () => {
      [...host.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("Load more"))!.click();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(host.querySelectorAll("ul.space-y-3 > li")).toHaveLength(105);
    expect(host.querySelector<HTMLInputElement>('input[aria-label="Select Archived role 0 at Thinking Machines Lab"]')?.checked).toBe(true);
    await act(async () => {
      host.querySelector<HTMLInputElement>('input[aria-label="Select all matching archived postings"]')!.click();
      await Promise.resolve();
    });
    expect(host.textContent).toContain("105 selected");
    await act(async () => {
      [...host.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("Restore selected"))!.click();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(apiMocks.unarchiveJobs).toHaveBeenCalledWith(rows.map(({ job }) => job.id));
  });

  it("confirms bulk deletion with the selected count before sending one batch", async () => {
    setupArchivedRows([
      makeArchivedJob("role-1", "Software Engineer"),
      makeArchivedJob("role-2", "Research Engineer"),
    ]);
    await mount("/?archived=true");

    await act(async () => {
      host.querySelector<HTMLInputElement>('input[aria-label="Select all matching archived postings"]')!.click();
    });
    const deleteSelected = [...host.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("Delete selected"))!;
    await act(async () => deleteSelected.click());
    expect(host.querySelector('[role="dialog"]')?.textContent).toContain("Delete 2 job postings?");

    const confirm = [...host.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')]
      .find((button) => button.textContent?.includes("Delete permanently"))!;
    await act(async () => {
      confirm.click();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(apiMocks.deleteJobs).toHaveBeenCalledWith(["role-1", "role-2"]);
    expect(host.querySelector('[role="dialog"]')).toBeNull();
  });

  it("keeps selected postings available when a bulk restore fails", async () => {
    setupArchivedRows([makeArchivedJob("role-1", "Software Engineer")]);
    apiMocks.unarchiveJobs.mockRejectedValue(new Error("Batch restore failed"));
    await mount("/?archived=true");

    const checkbox = host.querySelector<HTMLInputElement>('input[aria-label="Select Software Engineer at Thinking Machines Lab"]')!;
    await act(async () => checkbox.click());
    const restore = [...host.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("Restore selected"))!;
    await act(async () => {
      restore.click();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(host.querySelector<HTMLInputElement>('input[aria-label="Select Software Engineer at Thinking Machines Lab"]')?.checked).toBe(true);
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Batch restore failed");
  });

  it("keeps the confirmation and selection when a bulk delete fails", async () => {
    setupArchivedRows([makeArchivedJob("role-1", "Software Engineer")]);
    apiMocks.deleteJobs.mockRejectedValue(new Error("Batch delete failed"));
    await mount("/?archived=true");

    await act(async () => {
      host.querySelector<HTMLInputElement>('input[aria-label="Select all matching archived postings"]')!.click();
    });
    await act(async () => {
      [...host.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("Delete selected"))!.click();
    });
    await act(async () => {
      [...host.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')]
        .find((button) => button.textContent?.includes("Delete permanently"))!.click();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(host.querySelector<HTMLInputElement>('input[aria-label="Select Software Engineer at Thinking Machines Lab"]')?.checked).toBe(true);
    expect(host.querySelector('[role="dialog"]')?.textContent).toContain("Delete permanently");
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Batch delete failed");
  });
});
