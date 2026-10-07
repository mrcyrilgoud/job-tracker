// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
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

  async function mount() {
    await act(async () => {
      root.render(<NewRolesPage />);
      await Promise.resolve();
    });
    await act(async () => { await Promise.resolve(); });
  }

  function button(label: string) {
    return [...host.querySelectorAll("button")].find((item) => item.textContent?.trim() === label) ?? null;
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
});
