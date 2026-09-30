// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { RunViewState } from "@/lib/run-state";
import type { PostingProgress } from "@/lib/run-contract";
import { RunPostingList } from "./RunPostingList";

const mock = vi.hoisted(() => ({
  setFilter: vi.fn(), toggleRetry: vi.fn(), selectAllNeedingAttention: vi.fn(),
  state: {} as RunViewState,
}));
vi.mock("@/lib/RunMonitorContext", () => ({ useRunMonitor: () => mock }));

const state = (postings: PostingProgress[]): RunViewState => ({
  ...({ displayed: {
    version: 1, runId: "r", runType: "postingCheck", runStatus: "completed", seq: 2,
    stage: "postings", message: "done", current: 1, total: 1, done: true,
    startedAt: "2026-01-01T00:00:00Z", elapsedMs: 1,
    postingCounts: { queued: 0, active: 0, completed: 1, error: 0, canceled: 0 }, postingTotal: 1,
    postings, live: true, dismissed: false,
  }, filter: "all", retrySelection: new Set(), notice: undefined } as unknown as RunViewState),
} as RunViewState);

describe("RunPostingList", () => {
  let root: ReturnType<typeof createRoot> | undefined;
  let container: HTMLDivElement | undefined;
  afterEach(() => { if (root) act(() => root?.unmount()); container?.remove(); });

  it("keeps a focused row control mounted across posting updates", () => {
    const posting = { jobId: "j1", title: "Engineer", companyName: "Acme", postingUrl: "https://example.com/j1", status: "error" as const, reason: "timeout" };
    mock.state = state([posting]);
    container = document.createElement("div"); document.body.appendChild(container); root = createRoot(container);
    act(() => root?.render(<RunPostingList />));
    const checkbox = container.querySelector("input") as HTMLInputElement;
    checkbox.focus();
    expect(document.activeElement).toBe(checkbox);
    mock.state = state([{ ...posting, reason: "still unavailable" }]);
    act(() => root?.render(<RunPostingList />));
    expect(document.activeElement).toBe(container.querySelector("input"));
  });
});
