// @vitest-environment jsdom
//
// Unit coverage for RunMonitorProvider. Uses react-dom/client + act rather than
// @testing-library/react to avoid adding a dependency. `@/lib/api` and
// `@/lib/tauri` are mocked, and timers are faked so the reconciler and pollers
// stay deterministic. Fuller integration timing is task 13.7.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

import type { ParseResult, RunProgressEvent, RunSnapshot } from "@/lib/run-contract";

// ---- Mocks ----

let progressHandler: ((r: ParseResult<RunProgressEvent>) => void) | null = null;
const unlisten = vi.fn();

vi.mock("@/lib/tauri", () => ({
  isDesktopShell: () => true,
  DESKTOP_SHELL_REQUIRED: "shell required",
}));

vi.mock("@/lib/api", () => ({
  api: {
    getCurrentRun: vi.fn(),
    getRun: vi.fn(),
    startRun: vi.fn(),
    retryRun: vi.fn(),
    cancelRun: vi.fn(),
    dismissRun: vi.fn(),
    listenRunProgress: vi.fn((handler: (r: ParseResult<RunProgressEvent>) => void) => {
      progressHandler = handler;
      return Promise.resolve(unlisten);
    }),
  },
}));

import { api } from "@/lib/api";
import { RunMonitorProvider, useRunMonitor, useRunMonitorActions, type RunMonitorContextValue } from "@/lib/RunMonitorContext";

const mockApi = api as unknown as Record<string, Mock>;

// ---- Fixtures ----

function counts(over: Partial<RunSnapshot["postingCounts"]> = {}): RunSnapshot["postingCounts"] {
  return { queued: 0, active: 0, completed: 0, error: 0, canceled: 0, ...over };
}

function snapshot(over: Partial<RunSnapshot> = {}): RunSnapshot {
  return {
    version: 1,
    runId: "run-1",
    runType: "postingCheck",
    runStatus: "active",
    seq: 1,
    stage: "postings",
    message: "Checking",
    current: 0,
    total: 2,
    done: false,
    startedAt: "2026-01-01T00:00:00Z",
    elapsedMs: 0,
    postingCounts: counts({ active: 2 }),
    postingTotal: 2,
    postings: [],
    live: true,
    dismissed: false,
    ...over,
  };
}

function event(over: Partial<RunProgressEvent> = {}): RunProgressEvent {
  return {
    version: 1,
    runId: "run-1",
    runType: "postingCheck",
    runStatus: "queued",
    seq: 1,
    emittedAt: "2026-01-01T00:00:00Z",
    stage: "postings",
    message: "Queued",
    current: 0,
    total: 2,
    done: false,
    startedAt: "2026-01-01T00:00:00Z",
    elapsedMs: 0,
    postingCounts: counts({ queued: 2 }),
    postingTotal: 2,
    postings: [],
    ...over,
  };
}

// ---- Harness: capture the context value out of the provider ----

let container: HTMLDivElement;
let root: Root;
let captured: RunMonitorContextValue;
let actionsRenderCount = 0;

function Capture(): null {
  captured = useRunMonitor();
  return null;
}

function CaptureActions(): null {
  useRunMonitorActions();
  actionsRenderCount += 1;
  return null;
}

async function mount(): Promise<void> {
  await act(async () => {
    root.render(
      <RunMonitorProvider>
        <>
          <Capture />
          <CaptureActions />
        </>
      </RunMonitorProvider>,
    );
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  progressHandler = null;
  actionsRenderCount = 0;
  unlisten.mockClear();
  for (const fn of Object.values(mockApi)) fn.mockReset();
  mockApi.getCurrentRun.mockResolvedValue(null);
  mockApi.listenRunProgress.mockImplementation(
    (handler: (r: ParseResult<RunProgressEvent>) => void) => {
      progressHandler = handler;
      return Promise.resolve(unlisten);
    },
  );
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

describe("RunMonitorProvider", () => {
  it("restores the current run on mount", async () => {
    mockApi.getCurrentRun.mockResolvedValue(snapshot());
    await mount();
    await act(async () => {
      await Promise.resolve();
    });
    expect(mockApi.getCurrentRun).toHaveBeenCalledTimes(1);
    expect(captured.state.displayed?.runId).toBe("run-1");
  });

  it("applies validated progress events", async () => {
    await mount();
    // A seq=1 queued event creates the displayed run.
    await act(async () => {
      progressHandler?.({ ok: true, value: event() });
    });
    expect(captured.state.displayed?.runId).toBe("run-1");
    // A follow-up active event advances it.
    await act(async () => {
      progressHandler?.({
        ok: true,
        value: event({ seq: 2, runStatus: "active", previousRunStatus: "queued" }),
      });
    });
    expect(captured.state.displayed?.runStatus).toBe("active");
    expect(captured.state.lastSeq).toBe(2);
  });

  it("does not rerender actions-only consumers for progress events", async () => {
    await mount();
    const initialRenders = actionsRenderCount;
    await act(async () => {
      progressHandler?.({ ok: true, value: event() });
      progressHandler?.({ ok: true, value: event({ seq: 2, runStatus: "active", previousRunStatus: "queued" }) });
    });
    expect(captured.state.displayed?.runStatus).toBe("active");
    expect(actionsRenderCount).toBe(initialRenders);
  });

  it("maps operation_in_progress:runner on start to rejectedInProgress", async () => {
    await mount();
    mockApi.startRun.mockRejectedValue("operation_in_progress:runner");
    await act(async () => {
      await captured.start("postingCheck");
    });
    expect(captured.state.displayed).toBeNull();
    expect(captured.state.notice?.kind).toBe("inProgress");
  });

  it("shows a visible notice when a run action fails", async () => {
    await mount();
    mockApi.startRun.mockRejectedValue("run_failed:runner_interrupted");
    await act(async () => {
      await captured.start("postingCheck");
    });
    expect(captured.state.notice?.kind).toBe("action");
    expect(captured.state.notice?.message).toContain("run_failed:runner_interrupted");
  });

  it("fires onRunSettled exactly once when the run becomes terminal", async () => {
    await mount();
    const settled = vi.fn();
    act(() => {
      captured.onRunSettled(settled);
    });
    await act(async () => {
      progressHandler?.({ ok: true, value: event() });
    });
    await act(async () => {
      progressHandler?.({
        ok: true,
        value: event({ seq: 2, runStatus: "completed", done: true }),
      });
    });
    // A duplicate/late terminal event must not fire it again.
    await act(async () => {
      progressHandler?.({
        ok: true,
        value: event({ seq: 3, runStatus: "completed", done: true }),
      });
    });
    expect(settled).toHaveBeenCalledTimes(1);
    expect(settled.mock.calls[0][0].runId).toBe("run-1");
  });

  it("unsubscribes the listener on unmount", async () => {
    await mount();
    await act(async () => {
      await Promise.resolve();
    });
    act(() => root.unmount());
    expect(unlisten).toHaveBeenCalled();
  });
});
