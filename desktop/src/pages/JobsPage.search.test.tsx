// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from "react-router-dom";
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
});
