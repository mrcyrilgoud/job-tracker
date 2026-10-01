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

  it("exposes the two global checks and disables them while a run is active", () => {
    mock.state = { displayed: { runStatus: "active" } } as RunViewState;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    act(() => root?.render(<RunControls />));
    const buttons = [...container.querySelectorAll("button")];
    expect(buttons).toHaveLength(2);
    expect(buttons.map((button) => button.textContent)).toEqual([
      "Check postings",
      "Check career sources",
    ]);
    expect(buttons.every((button) => button.disabled)).toBe(true);
  });

  it("starts the requested run type", () => {
    mock.state = { displayed: null, startPending: false } as Partial<RunViewState>;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    act(() => root?.render(<RunControls />));

    const buttons = [...container.querySelectorAll("button")];
    act(() => {
      buttons[0]?.click();
      buttons[1]?.click();
    });

    expect(mock.start).toHaveBeenNthCalledWith(1, "postingCheck");
    expect(mock.start).toHaveBeenNthCalledWith(2, "careerCheck");
  });

  it("shows action failures when no run snapshot is available", () => {
    mock.state = {
      displayed: null,
      startPending: false,
      notice: { kind: "action", message: "Run action failed: unavailable", expiresAt: 1_000 },
    } as RunViewState;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    act(() => root?.render(<RunControls />));

    expect(container.textContent).toContain("Run action failed: unavailable");
    expect(container.querySelector('[role="status"]')?.textContent).toBe(
      "Run action failed: unavailable",
    );
  });

  it("disables both starts while the provider is awaiting a start response", () => {
    mock.state = { displayed: null, startPending: true } as RunViewState;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    act(() => root?.render(<RunControls />));

    const buttons = [...container.querySelectorAll("button")];
    expect(buttons).toHaveLength(2);
    expect(buttons.every((button) => button.disabled)).toBe(true);
    buttons[0]?.click();
    expect(mock.start).not.toHaveBeenCalled();
  });
});
