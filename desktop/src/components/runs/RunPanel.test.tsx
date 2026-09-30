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
});
