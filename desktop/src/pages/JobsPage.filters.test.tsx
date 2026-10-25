// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { jobStatuses, type JobListItem } from "@/lib/schema";

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

describe("JobsPage enhanced-filters popover", () => {
  let root: Root;
  let host: HTMLDivElement;

  beforeEach(() => {
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
    apiMocks.listJobsPage.mockImplementation(async (filters?: { newFromWatch?: boolean }) => ({
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

  async function openPopover() {
    const trigger = [...host.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.getAttribute("aria-controls") === "jobs-filter-popover")!;
    await act(async () => {
      trigger.click();
      await Promise.resolve();
    });
    return trigger;
  }

  function popover() {
    return host.querySelector<HTMLElement>('[role="dialog"][aria-labelledby="jobs-filter-heading"]')!;
  }

  function locationSearch() {
    return new URLSearchParams(host.querySelector('[data-testid="location-search"]')?.textContent ?? "");
  }

  it("exposes a checkbox for each of the eight job statuses (Req 1.1)", async () => {
    await mount();
    await openPopover();

    const checkboxes = jobStatuses.map((statusValue) =>
      popover().querySelector<HTMLInputElement>(`#status-filter-${statusValue}`));
    expect(checkboxes).toHaveLength(8);
    for (const checkbox of checkboxes) {
      expect(checkbox).not.toBeNull();
      expect(checkbox!.type).toBe("checkbox");
      expect(checkbox!.checked).toBe(false);
    }
  });

  it("offers the three date fields plus a no-filter option and start/end date inputs (Req 2.1, 2.2)", async () => {
    await mount();
    await openPopover();

    const dateField = popover().querySelector<HTMLSelectElement>("#date-field")!;
    expect(dateField).not.toBeNull();
    expect([...dateField.options].map((option) => option.value)).toEqual([
      "",
      "appliedAt",
      "postingDate",
      "lastCheckedAt",
    ]);

    const from = popover().querySelector<HTMLInputElement>("#date-from")!;
    const to = popover().querySelector<HTMLInputElement>("#date-to")!;
    expect(from.type).toBe("date");
    expect(to.type).toBe("date");
  });

  it("exposes a location text input capped at 200 characters (Req 3.1)", async () => {
    await mount();
    await openPopover();

    const location = popover().querySelector<HTMLInputElement>("#location-filter")!;
    expect(location).not.toBeNull();
    expect(location.type).toBe("text");
    expect(location.maxLength).toBe(200);
  });

  it("offers the three remote modes in a single-select control (Req 3.2)", async () => {
    await mount();
    await openPopover();

    const remote = popover().querySelector<HTMLSelectElement>("#remote-mode")!;
    expect(remote).not.toBeNull();
    expect([...remote.options].map((option) => option.value)).toEqual([
      "any",
      "remoteOnly",
      "onsiteOnly",
    ]);
  });

  it("preserves the active sort and favorites selections when submitting a status change (Req 5.3, 5.4)", async () => {
    await mount("/?favorites=true&sortBy=companyName&sortOrder=descending");
    await openPopover();

    const checkbox = popover().querySelector<HTMLInputElement>("#status-filter-applied")!;
    await act(async () => {
      checkbox.click();
      await Promise.resolve();
    });
    await act(async () => {
      popover().querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
      await Promise.resolve();
    });

    const applied = locationSearch();
    expect(applied.get("statuses")).toBe("applied");
    expect(applied.get("favorites")).toBe("true");
    expect(applied.get("sortBy")).toBe("companyName");
    expect(applied.get("sortOrder")).toBe("descending");
  });

  it("preserves the active sort and archived selections when submitting a status change (Req 5.3, 5.4)", async () => {
    await mount("/?archived=true&sortBy=appeal&sortOrder=ascending");
    await openPopover();

    const checkbox = popover().querySelector<HTMLInputElement>("#status-filter-interviewing")!;
    await act(async () => {
      checkbox.click();
      await Promise.resolve();
    });
    await act(async () => {
      popover().querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
      await Promise.resolve();
    });

    const applied = locationSearch();
    expect(applied.get("statuses")).toBe("interviewing");
    expect(applied.get("archived")).toBe("true");
    expect(applied.get("sortBy")).toBe("appeal");
    expect(applied.get("sortOrder")).toBe("ascending");
  });

  it("preserves sort, favorites, and archived while adding location and remote filters (Req 3.1, 3.2, 5.3, 5.4)", async () => {
    await mount("/?favorites=true&archived=true&sortBy=companyName&sortOrder=descending");
    await openPopover();

    const location = popover().querySelector<HTMLInputElement>("#location-filter")!;
    const remote = popover().querySelector<HTMLSelectElement>("#remote-mode")!;
    await act(async () => {
      typeInto(location, "Berlin");
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")?.set;
      setter?.call(remote, "remoteOnly");
      remote.dispatchEvent(new Event("change", { bubbles: true }));
      await Promise.resolve();
    });
    await act(async () => {
      popover().querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
      await Promise.resolve();
    });

    const applied = locationSearch();
    expect(applied.get("location")).toBe("Berlin");
    expect(applied.get("remote")).toBe("remoteOnly");
    expect(applied.get("favorites")).toBe("true");
    expect(applied.get("archived")).toBe("true");
    expect(applied.get("sortBy")).toBe("companyName");
    expect(applied.get("sortOrder")).toBe("descending");
  });
});
