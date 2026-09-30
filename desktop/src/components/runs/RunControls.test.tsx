// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { RunViewState } from "@/lib/run-state";
import { RunControls } from "./RunControls";

const mock = vi.hoisted(() => ({ start: vi.fn(), state: { displayed: null } as Partial<RunViewState> }));
vi.mock("@/lib/RunMonitorContext", () => ({ useRunMonitor: () => mock }));

describe("RunControls", () => {
  let root: ReturnType<typeof createRoot> | undefined;
  let container: HTMLDivElement | undefined;
  afterEach(() => {
    if (root) act(() => root?.unmount());
    container?.remove();
    mock.start.mockReset();
  });

  it("disables both start buttons while a run is active", () => {
    mock.state = { displayed: { runStatus: "active" } } as RunViewState;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    act(() => root?.render(<RunControls />));
    const buttons = [...container.querySelectorAll("button")];
    expect(buttons).toHaveLength(2);
    expect(buttons.every((button) => button.disabled)).toBe(true);
  });
});
