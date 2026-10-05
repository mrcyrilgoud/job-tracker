// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { JobListItem } from "@/lib/schema";

const apiMocks = vi.hoisted(() => ({
  listJobs: vi.fn(),
  getJobsDashboard: vi.fn(),
  listCompanies: vi.fn(),
  reportRefreshFailed: vi.fn(),
  onRunSettled: vi.fn(() => () => {}),
}));

vi.mock("@/lib/api", () => ({ api: apiMocks }));
vi.mock("@/lib/RunMonitorContext", () => ({
  useRunMonitor: () => ({ onRunSettled: apiMocks.onRunSettled, reportRefreshFailed: apiMocks.reportRefreshFailed }),
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

function Location() {
  const location = useLocation();
  return createElement("output", { "data-testid": "location" }, location.pathname);
}

describe("JobsPage live search suggestions", () => {
  let root: Root;
  let host: HTMLDivElement;

  beforeEach(() => {
    vi.useFakeTimers();
    apiMocks.listJobs.mockReset();
    apiMocks.getJobsDashboard.mockReset();
    apiMocks.listCompanies.mockReset();
    apiMocks.listJobs.mockImplementation(async (filters?: { newFromWatch?: boolean }) => ({
      jobs: filters?.newFromWatch ? [] : [makeJob("sandbox-role", "Software Engineer, Sandboxing")],
    }));
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

  async function mount() {
    await act(async () => {
      root.render(createElement(MemoryRouter, { initialEntries: ["/"] }, createElement(Routes, null,
        createElement(Route, { path: "/", element: createElement(JobsPage) }),
        createElement(Route, { path: "/jobs/:id", element: createElement(Location) }),
      )));
      await Promise.resolve();
    });
    await act(async () => { await Promise.resolve(); });
  }

  function typeInto(input: HTMLInputElement, value: string) {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }

  it("waits for the debounce and renders matching suggestions from the filtered jobs response", async () => {
    await mount();
    const input = host.querySelector<HTMLInputElement>('input[aria-label="Search jobs"]')!;
    const initialSearchCalls = apiMocks.listJobs.mock.calls.filter(([filters]) => !filters?.newFromWatch).length;
    const dashboardCalls = apiMocks.getJobsDashboard.mock.calls.length;
    const companyCalls = apiMocks.listCompanies.mock.calls.length;

    await act(async () => {
      input.focus();
      typeInto(input, "san");
      await vi.advanceTimersByTimeAsync(249);
    });
    expect(apiMocks.listJobs.mock.calls.filter(([filters]) => !filters?.newFromWatch)).toHaveLength(initialSearchCalls);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
      await Promise.resolve();
    });
    expect(apiMocks.listJobs.mock.calls.filter(([filters]) => !filters?.newFromWatch)).toHaveLength(initialSearchCalls + 1);
    expect(host.querySelector('[role="option"]')?.textContent).toContain("Software Engineer, Sandboxing");
    expect(host.textContent).toContain("Thinking Machines Lab");
    expect(apiMocks.getJobsDashboard).toHaveBeenCalledTimes(dashboardCalls);
    expect(apiMocks.listCompanies).toHaveBeenCalledTimes(companyCalls);
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
    apiMocks.listJobs.mockImplementation(async (filters?: { newFromWatch?: boolean }) => ({
      jobs: filters?.newFromWatch ? [] : Array.from({ length: 8 }, (_, index) =>
        makeJob(`role-${index}`, `Sandbox Role ${index}`)),
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
    let resolveSan!: (value: { jobs: JobListItem[] }) => void;
    let inFlight = 0;
    let peakInFlight = 0;
    apiMocks.listJobs.mockImplementation((filters?: { newFromWatch?: boolean; search?: string }) => {
      if (filters?.newFromWatch) return Promise.resolve({ jobs: [] });
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
        return Promise.resolve({ jobs: [makeJob("latest", "Latest result")] }).finally(() => { inFlight -= 1; });
      }
      return Promise.resolve({ jobs: [makeJob("sandbox-role", "Software Engineer, Sandboxing")] });
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
    expect(apiMocks.listJobs.mock.calls.filter(([filters]) => filters?.search === "sand")).toHaveLength(0);

    await act(async () => {
      resolveSan({ jobs: [makeJob("stale", "Stale result")] });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(apiMocks.listJobs.mock.calls.filter(([filters]) => filters?.search === "sand")).toHaveLength(1);
    await act(async () => { await Promise.resolve(); });
    expect(peakInFlight).toBe(1);
    expect(host.textContent).toContain("Latest result");
    expect(host.textContent).not.toContain("Stale result");
  });
});
