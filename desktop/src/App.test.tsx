// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "@/App";
import type { ParseResult, RunProgressEvent, RunSnapshot } from "@/lib/run-contract";

const mockApi = vi.hoisted(() => ({
  showMainWindow: vi.fn(),
  getCurrentRun: vi.fn(),
  getRun: vi.fn(),
  startRun: vi.fn(),
  retryRun: vi.fn(),
  cancelRun: vi.fn(),
  dismissRun: vi.fn(),
  listenRunProgress: vi.fn(),
}));
vi.mock("@/lib/api", () => ({ api: mockApi }));
vi.mock("@/lib/tauri", () => ({ isDesktopShell: () => true }));
// Pages stand in for backend-heavy screens; routing, header, run UI, and provider are real.
vi.mock("@/pages/JobsPage", () => ({ JobsPage: () => <h1>Jobs destination</h1> }));
vi.mock("@/pages/DocumentsPage", () => ({ DocumentsPage: () => <h1>Documents destination</h1> }));
vi.mock("@/pages/CompaniesPage", () => ({ CompaniesPage: () => <h1>Companies destination</h1> }));
vi.mock("@/pages/SettingsPage", () => ({ SettingsPage: () => <h1>Settings destination</h1> }));
vi.mock("@/pages/JobDetailPage", () => ({ JobDetailPage: () => null }));
vi.mock("@/pages/CompanyDetailPage", () => ({ CompanyDetailPage: () => null }));
vi.mock("@/pages/NewJobPage", () => ({ NewJobPage: () => null }));

function snapshot(status: "active" | "completed" = "completed"): RunSnapshot {
  const completed = status === "completed";
  return {
    version: 1, runId: "navigation-run", runType: "postingCheck", runStatus: status,
    seq: 2, stage: "postings", message: completed ? "Finished" : "Checking",
    current: completed ? 1 : 0, total: 1, done: completed,
    startedAt: "2026-10-01T00:00:00Z", elapsedMs: 1000,
    postingCounts: { queued: 0, active: completed ? 0 : 1, completed: completed ? 1 : 0, error: 0, canceled: 0 },
    postingTotal: 1,
    postings: [{ jobId: "saved-job", title: "Saved posting", companyName: "Acme", postingUrl: "https://example.com/job", status: completed ? "completed" : "active", postingState: "active" }],
    live: true, dismissed: false,
    ...(completed ? { summary: {
      runId: "navigation-run", runType: "postingCheck" as const, status: "completed" as const,
      startedAt: "2026-10-01T00:00:00Z", finishedAt: "2026-10-01T00:00:01Z",
      durationMs: 1000, postingOutcomes: { active: 1, closed: 0, unknown: 0, error: 0, canceled: 0 }, stateChanges: 1,
    } } : {}),
  };
}

let container: HTMLDivElement;
let root: Root;
let progress: (result: ParseResult<RunProgressEvent>) => void;
beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  window.history.replaceState(null, "", "/settings");
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  });
  mockApi.showMainWindow.mockResolvedValue(undefined);
  mockApi.getCurrentRun.mockResolvedValue(snapshot());
  mockApi.listenRunProgress.mockImplementation((handler: typeof progress) => {
    progress = handler;
    return Promise.resolve(() => {});
  });
  mockApi.dismissRun.mockResolvedValue(undefined);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function mount() { await act(async () => root.render(<App />)); }
async function click(element: Element | null) {
  expect(element).not.toBeNull();
  await act(async () => { (element as HTMLElement).click(); });
}
function button(label: string) {
  return [...container.querySelectorAll("button")].find((element) => element.textContent === label) ?? null;
}
function panel() { return container.querySelector('section[aria-label="Run status"]'); }

describe("top navigation while a posting run is displayed", () => {
  it.each([
    ["Jobs", "/"], ["Documents", "/documents"], ["Companies", "/companies"], ["Settings", "/settings"],
  ])("shows %s immediately and retains the completed run for reopening", async (label, path) => {
    await mount();
    expect(panel()).not.toBeNull();
    await click([...container.querySelectorAll("nav a")].find((link) => link.textContent === label) ?? null);
    expect(window.location.pathname).toBe(path);
    expect(container.querySelector("main h1")?.textContent).toBe(`${label} destination`);
    expect(panel()).toBeNull();
    expect(mockApi.dismissRun).not.toHaveBeenCalled();
    expect(mockApi.cancelRun).not.toHaveBeenCalled();
    await click(button("Show run status"));
    expect(panel()?.textContent).toContain("navigation-run");
    expect(panel()?.textContent).toContain("Saved posting");
    expect(mockApi.startRun).not.toHaveBeenCalled();
    expect(mockApi.getCurrentRun).toHaveBeenCalledTimes(1);
  });

  it("returns to Jobs through the app title without discarding the run", async () => {
    await mount();
    await click(container.querySelector('header > a[href="/"]'));
    expect(window.location.pathname).toBe("/");
    expect(panel()).toBeNull();
    await click(button("Show run status"));
    expect(panel()?.textContent).toContain("navigation-run");
    expect(mockApi.dismissRun).not.toHaveBeenCalled();
  });

  it("keeps an active run running and start controls disabled after navigation", async () => {
    mockApi.getCurrentRun.mockResolvedValue(snapshot("active"));
    await mount();
    await click(container.querySelector('nav a[href="/documents"]'));
    expect(panel()).toBeNull();
    expect((button("Check postings") as HTMLButtonElement).disabled).toBe(true);
    expect((button("Check career sources") as HTMLButtonElement).disabled).toBe(true);
    await click(button("Show run status"));
    expect(panel()?.textContent).toContain("navigation-run");
    expect(button("Cancel")).not.toBeNull();
    expect(mockApi.startRun).not.toHaveBeenCalled();
    expect(mockApi.cancelRun).not.toHaveBeenCalled();
  });

  it("reveals a newly started check after leaving an existing summary", async () => {
    const next = { ...snapshot("active"), runId: "next-run" };
    mockApi.startRun.mockResolvedValue({ runId: next.runId, snapshot: next });
    await mount();
    await click(container.querySelector('nav a[href="/companies"]'));
    expect(panel()).toBeNull();
    await click(button("Check postings"));
    expect(mockApi.startRun).toHaveBeenCalledExactlyOnceWith("postingCheck");
    expect(panel()?.textContent).toContain("next-run");
  });

  it("still dismisses a completed summary after reopening it", async () => {
    await mount();
    await click(container.querySelector('nav a[href="/documents"]'));
    await click(button("Show run status"));
    await click(button("Dismiss"));
    expect(mockApi.dismissRun).toHaveBeenCalledExactlyOnceWith("navigation-run");
    expect(panel()).toBeNull();
    expect(button("Show run status")).toBeNull();
    expect(container.querySelector("main h1")?.textContent).toBe("Documents destination");
  });
  it("receives completion while hidden without blocking the destination or losing announcements", async () => {
    mockApi.getCurrentRun.mockResolvedValue(snapshot("active"));
    await mount();
    const liveRegion = container.querySelector('[aria-live="polite"]');
    await click(container.querySelector('nav a[href="/documents"]'));
    const completed = snapshot();
    await act(async () => progress({ ok: true, value: {
      ...completed, seq: 3, emittedAt: "2026-10-01T00:00:01Z",
    } }));
    expect(panel()).toBeNull();
    expect(container.querySelector("main h1")?.textContent).toBe("Documents destination");
    expect(container.querySelector('[aria-live="polite"]')).toBe(liveRegion);
    expect(liveRegion?.textContent).toContain("completed");
    expect(container.querySelector("header")?.textContent).toContain("Completed");
    expect((button("Check postings") as HTMLButtonElement).disabled).toBe(false);
    await click(button("Show run status"));
    expect(panel()?.textContent).toContain("Saved posting");
    expect(panel()?.textContent).toContain("Completed");
  });

  it("keeps error notices visible in the header when the run panel is hidden", async () => {
    await mount();
    await click(container.querySelector('nav a[href="/documents"]'));
    await act(async () => progress({ ok: false, kind: "invalid", message: "Progress data was invalid" }));
    expect(panel()).toBeNull();
    expect(container.querySelector("header")?.textContent).toContain("Progress data was invalid");
    expect(button("Show run status")).not.toBeNull();
  });

  it("leaves the current panel visible when a navigation link is opened with a modifier", async () => {
    await mount();
    const link = container.querySelector('nav a[href="/documents"]')!;
    // Prevent jsdom's unsupported document navigation after React handles the modified click.
    document.addEventListener("click", (event) => event.preventDefault(), { once: true });
    await act(async () => link.dispatchEvent(new MouseEvent("click", {
      bubbles: true, cancelable: true, button: 0, metaKey: true,
    })));
    expect(window.location.pathname).toBe("/settings");
    expect(panel()).not.toBeNull();
  });

  it("hides reopened status after browser route navigation but preserves it for query-only changes", async () => {
    await mount();
    await click(container.querySelector('nav a[href="/documents"]'));
    await click(button("Show run status"));
    await act(async () => {
      window.history.pushState(null, "", "/documents?sort=name");
      window.dispatchEvent(new PopStateEvent("popstate"));
    });
    expect(panel()).not.toBeNull();
    await act(async () => {
      window.history.pushState(null, "", "/companies");
      window.dispatchEvent(new PopStateEvent("popstate"));
    });
    expect(panel()).toBeNull();
    expect(container.querySelector("main h1")?.textContent).toBe("Companies destination");
  });

});
