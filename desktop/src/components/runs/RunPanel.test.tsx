// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { RunViewState } from "@/lib/run-state";
import { RunPanel } from "./RunPanel";

const mock = vi.hoisted(() => ({ state: {} as RunViewState, cancel: vi.fn(), retry: vi.fn(), announcements: { assertive: [] } }));
vi.mock("@/lib/RunMonitorContext", () => ({ useRunMonitor: () => mock }));

describe("RunPanel", () => {
  let root: ReturnType<typeof createRoot> | undefined;
  let container: HTMLDivElement | undefined;
  afterEach(() => { if (root) act(() => root?.unmount()); container?.remove(); });

  it("renders every stage outcome as text", () => {
    mock.state = { displayed: {
      version: 1, runId: "r", runType: "jobsCycle", runStatus: "active", seq: 1,
      stage: "postings", message: "running", current: 0, total: 1, done: false,
      startedAt: "2026-01-01T00:00:00Z", elapsedMs: 0,
      stages: [
        { name: "postings", outcome: "not_started", current: 0, total: 1 },
        { name: "watches", outcome: "in_progress", current: 0, total: 1 },
        { name: "careers", outcome: "succeeded", current: 1, total: 1 },
        { name: "csv", outcome: "failed", current: 0, total: 1, error: "failed" },
      ],
      postingCounts: { queued: 1, active: 0, completed: 0, error: 0, canceled: 0 }, postingTotal: 1,
      postings: [], live: true, dismissed: false,
    }, notice: undefined, retrySelection: new Set(), filter: "all", lastSeq: 1, needsReconcile: false, announcedErrors: new Set() } as unknown as RunViewState;
    container = document.createElement("div"); document.body.appendChild(container); root = createRoot(container);
    act(() => root?.render(<RunPanel />));
    expect(container.textContent).toContain("Not started");
    expect(container.textContent).toContain("In progress");
    expect(container.textContent).toContain("Succeeded");
    expect(container.textContent).toContain("Failed");
  });

  it("hides posting-specific details for a career check", () => {
    mock.state = { displayed: {
      version: 1, runId: "career-run", runType: "careerCheck", runStatus: "completed", seq: 2,
      stage: "careers", message: "done", current: 1, total: 1, done: true,
      startedAt: "2026-01-01T00:00:00Z", elapsedMs: 0,
      stages: [{ name: "careers", outcome: "succeeded", current: 1, total: 1 }],
      postingCounts: { queued: 0, active: 0, completed: 0, error: 0, canceled: 0 }, postingTotal: 0,
      postings: [{ jobId: "should-not-render", title: "Hidden posting", companyName: "Acme", postingUrl: "https://example.com/hidden", status: "error", reason: "hidden" }],
      summary: {
        runId: "career-run", runType: "careerCheck", status: "completed", startedAt: "2026-01-01T00:00:00Z",
        finishedAt: "2026-01-01T00:00:01Z", durationMs: 1,
        postingOutcomes: { active: 0, closed: 0, unknown: 0, error: 0, canceled: 0 }, stateChanges: 0,
      },
      live: true, dismissed: false,
    }, notice: undefined, retrySelection: new Set(["should-not-render"]), filter: "all", lastSeq: 2, needsReconcile: false, announcedErrors: new Set() } as unknown as RunViewState;
    container = document.createElement("div"); document.body.appendChild(container); root = createRoot(container);
    act(() => root?.render(<RunPanel />));

    expect(container.textContent).toContain("Check career sources");
    expect(container.textContent).not.toContain("Run summary");
    expect(container.textContent).not.toContain("Filter postings");
    expect(container.textContent).not.toContain("Hidden posting");
    expect(container.textContent).not.toContain("Retry selected");
  });
});
