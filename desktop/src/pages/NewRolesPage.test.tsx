// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { JobListItem } from "@/lib/schema";

const apiMocks = vi.hoisted(() => ({
  listJobs: vi.fn(),
  approveWatchJob: vi.fn(),
  dismissWatchJob: vi.fn(),
}));
const runMocks = vi.hoisted(() => ({
  callbacks: [] as Array<() => void>,
  onRunSettled: vi.fn(),
  reportRefreshFailed: vi.fn(),
}));

vi.mock("@/lib/api", () => ({ api: apiMocks }));
vi.mock("@/lib/RunMonitorContext", () => ({
  useRunMonitor: () => ({
    onRunSettled: runMocks.onRunSettled,
    reportRefreshFailed: runMocks.reportRefreshFailed,
  }),
}));

import { NewRolesPage } from "./NewRolesPage";

function role(id: string, overrides: Partial<JobListItem["job"]> = {}): JobListItem {
  return {
    companyName: "Thinking Machines Lab",
    job: {
      id,
      companyId: "company-1",
      title: `Software Engineer ${id}`,
      url: `https://example.com/jobs/${id}`,
      canonicalUrl: `https://example.com/jobs/${id}`,
      sourceExternalId: id,
      status: "wishlist",
      appliedAt: null,
      postingState: "active",
      lastCheckedAt: null,
      lastCheckResult: null,
      source: "greenhouse",
      notes: null,
      description: null,
      location: "San Francisco",
      isNewFromWatch: true,
      watchDisposition: "new",
      missingFromSyncCount: 0,
      isFavorite: false,
      appeal: null,
      salaryMin: null,
      salaryMax: null,
      createdAt: "2026-10-01T00:00:00.000Z",
      updatedAt: "2026-10-01T00:00:00.000Z",
      ...overrides,
    },
  };
}

function RouteState() {
  const location = useLocation();
  const navigate = useNavigate();
  return (
    <>
      <output data-testid="route-search">{location.search}</output>
      <button type="button" onClick={() => navigate(-1)}>Back</button>
      <button type="button" onClick={() => navigate(1)}>Forward</button>
    </>
  );
}

describe("NewRolesPage", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.resetAllMocks();
    runMocks.callbacks.length = 0;
    runMocks.onRunSettled.mockImplementation((callback: () => void) => {
      runMocks.callbacks.push(callback);
      return () => {};
    });
    apiMocks.listJobs.mockResolvedValue({ jobs: [] });
    apiMocks.approveWatchJob.mockResolvedValue({ success: true });
    apiMocks.dismissWatchJob.mockResolvedValue({ success: true });
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.unstubAllGlobals();
  });

  async function mount(initialEntry = "/new-roles") {
    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={[initialEntry]}>
          <RouteState />
          <Routes>
            <Route path="/new-roles" element={<NewRolesPage />} />
          </Routes>
        </MemoryRouter>,
      );
      await Promise.resolve();
    });
    await act(async () => { await Promise.resolve(); });
  }

  function button(label: string) {
    return [...host.querySelectorAll("button")].find((item) => item.textContent?.trim() === label) ?? null;
  }

  function changeInput(input: HTMLInputElement, value: string) {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }

  function changeSelect(select: HTMLSelectElement, value: string) {
    const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")?.set;
    setter?.call(select, value);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  }

  it("loads every untriaged watch role without a preview limit", async () => {
    apiMocks.listJobs.mockResolvedValue({ jobs: [role("role-1")] });
    await mount();

    expect(apiMocks.listJobs).toHaveBeenCalledExactlyOnceWith({ newFromWatch: true });
    expect(host.textContent).toContain("Software Engineer role-1");
    expect(host.textContent).toContain("Thinking Machines Lab · San Francisco");
    expect(host.textContent).not.toContain("/year");
  });

  it("shows a loading state and then optional salary metadata", async () => {
    let finishLoad!: (result: { jobs: JobListItem[] }) => void;
    apiMocks.listJobs.mockReturnValueOnce(new Promise((resolve) => { finishLoad = resolve; }));

    await mount();
    expect(host.textContent).toContain("Loading new roles…");

    await act(async () => {
      finishLoad({ jobs: [role("role-1", { salaryMin: 120_000, salaryMax: 180_000 })] });
      await Promise.resolve();
    });
    expect(host.textContent).toContain("$120k–$180k/year");
  });

  it("shows an empty queue after a successful load", async () => {
    await mount();

    expect(host.textContent).toContain("Your watches are up to date. No new matches found.");
  });

  it("offers retry after an initial load error", async () => {
    apiMocks.listJobs
      .mockRejectedValueOnce(new Error("Watch roles unavailable"))
      .mockResolvedValueOnce({ jobs: [role("role-1")] });
    await mount();

    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Watch roles unavailable");
    await act(async () => {
      button("Retry")?.click();
      await Promise.resolve();
    });
    expect(host.textContent).toContain("Software Engineer role-1");
  });

  it("saves and dismisses roles, refreshing the full queue after each action", async () => {
    apiMocks.listJobs
      .mockResolvedValueOnce({ jobs: [role("role-1"), role("role-2")] })
      .mockResolvedValueOnce({ jobs: [role("role-2")] })
      .mockResolvedValueOnce({ jobs: [] });
    await mount();

    await act(async () => {
      button("Save to my list")?.click();
      await Promise.resolve();
    });
    expect(apiMocks.approveWatchJob).toHaveBeenCalledExactlyOnceWith("role-1");
    expect(host.textContent).toContain("Software Engineer role-2");

    await act(async () => {
      button("Not for me")?.click();
      await Promise.resolve();
    });
    expect(apiMocks.dismissWatchJob).toHaveBeenCalledExactlyOnceWith("role-2");
    expect(host.textContent).toContain("Your watches are up to date. No new matches found.");
  });

  it("refreshes the queue after a completed run", async () => {
    apiMocks.listJobs
      .mockResolvedValueOnce({ jobs: [role("role-1")] })
      .mockResolvedValueOnce({ jobs: [role("role-2")] });
    await mount();

    await act(async () => {
      runMocks.callbacks[0]?.();
      await Promise.resolve();
    });

    expect(apiMocks.listJobs).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain("Software Engineer role-2");
  });

  it("shows every role and reports the total role and company counts", async () => {
    const roles = Array.from({ length: 8 }, (_, index) => role(`role-${index + 1}`, {
      companyId: index % 2 === 0 ? "company-1" : "company-2",
    }));
    apiMocks.listJobs.mockResolvedValue({ jobs: roles });
    await mount();

    expect(host.textContent).toContain("8 roles · 2 companies");
    expect(host.textContent).toContain("Showing 8 of 8 roles");
    expect(host.querySelectorAll("li")).toHaveLength(8);
    expect(host.textContent).not.toContain("Show 2 more");
  });

  it("hydrates filters and grouped view from the route query", async () => {
    const roles = [
      role("role-1", { title: "Software Engineer", companyId: "company-1" }),
      role("role-2", { title: "Data Designer", companyId: "company-2" }),
    ];
    roles[1]!.companyName = "Acme";
    apiMocks.listJobs.mockResolvedValue({ jobs: roles });
    await mount("/new-roles?search=designer&companyId=company-2&groupBy=company");

    expect(host.querySelector<HTMLInputElement>('[aria-label="Search new roles"]')?.value).toBe("designer");
    const companySelect = host.querySelector<HTMLSelectElement>('[aria-label="Filter by company"]')!;
    expect(companySelect.value).toBe("company-2");
    expect(companySelect.options).toHaveLength(3);
    expect(host.querySelector<HTMLButtonElement>('button[aria-pressed="true"]')?.textContent).toContain("Group by company");
    expect(host.textContent).toContain("Acme");
    expect(host.textContent).toContain("Showing 1 of 2 roles");
  });

  it("updates URL filters, preserves grouping when clearing, and supports history navigation", async () => {
    apiMocks.listJobs.mockResolvedValue({
      jobs: [
        role("role-1", { title: "Software Engineer", companyId: "company-1" }),
        role("role-2", { title: "Data Analyst", companyId: "company-2" }),
      ],
    });
    await mount("/new-roles?groupBy=company");

    const search = host.querySelector<HTMLInputElement>('[aria-label="Search new roles"]')!;
    await act(async () => {
      changeInput(search, "Data");
      await vi.waitFor(() => expect(host.textContent).toContain("Showing 1 of 2 roles"));
    });
    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 300));
    });
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toContain("search=Data");

    const company = host.querySelector<HTMLSelectElement>('[aria-label="Filter by company"]')!;
    await act(async () => {
      changeSelect(company, "company-2");
      await Promise.resolve();
    });
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toContain("companyId=company-2");
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toContain("groupBy=company");

    await act(async () => {
      button("Clear filters")?.click();
      await Promise.resolve();
    });
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toBe("?groupBy=company");
    expect(host.querySelector<HTMLButtonElement>('button[aria-pressed="true"]')?.textContent).toContain("Group by company");
    expect(host.textContent).toContain("Showing 2 of 2 roles");

    await act(async () => {
      button("Back")?.click();
      await Promise.resolve();
    });
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toContain("search=Data");
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toContain("companyId=company-2");
    await act(async () => {
      button("Forward")?.click();
      await Promise.resolve();
    });
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toBe("?groupBy=company");
  });

  it("applies posting and salary filters and shows a filter count", async () => {
    const roles = [
      role("active", { postingState: "active", salaryMin: 100_000, salaryMax: 150_000 }),
      role("inactive", { postingState: "inactive", salaryMin: 140_000, salaryMax: 180_000 }),
      role("unknown", { postingState: "unknown", salaryMin: null, salaryMax: null }),
    ];
    apiMocks.listJobs.mockResolvedValue({ jobs: roles });
    await mount();

    await act(async () => {
      button("Filters")?.click();
      await Promise.resolve();
    });
    const postingState = host.querySelector<HTMLSelectElement>("#new-roles-posting-state")!;
    const salaryMin = host.querySelector<HTMLInputElement>("#new-roles-salary-min")!;
    const salaryMax = host.querySelector<HTMLInputElement>("#new-roles-salary-max")!;
    await act(async () => {
      changeSelect(postingState, "inactive");
      changeInput(salaryMin, "160000");
      changeInput(salaryMax, "170000");
      button("Apply")?.click();
      await Promise.resolve();
    });

    expect(host.textContent).toContain("Filters2");
    expect(host.textContent).toContain("Showing 1 of 3 roles");
    expect(host.textContent).toContain("inactive");
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toContain("postingState=inactive");
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toContain("salaryMin=160000");
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toContain("salaryMax=170000");
  });

  it("resets a company filter when triage removes its final queued role", async () => {
    apiMocks.listJobs
      .mockResolvedValueOnce({ jobs: [role("role-1", { companyId: "company-1" })] })
      .mockResolvedValueOnce({ jobs: [role("role-2", { companyId: "company-2" })] });
    await mount("/new-roles?companyId=company-1&search=Software&groupBy=company");

    await act(async () => {
      button("Save to my list")?.click();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(apiMocks.approveWatchJob).toHaveBeenCalledExactlyOnceWith("role-1");
    expect(host.textContent).toContain("Software Engineer role-2");
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toContain("search=Software");
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).toContain("groupBy=company");
    expect(host.querySelector('[data-testid="route-search"]')?.textContent).not.toContain("companyId=");
  });
});
