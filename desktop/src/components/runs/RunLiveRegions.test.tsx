// @vitest-environment jsdom
//
// Minimal render coverage for RunLiveRegions (Req 12.3, 12.4). Uses
// react-dom/client + act instead of @testing-library/react to avoid adding a
// dependency. The RunMonitor context is mocked so we can drive `announcements`
// directly. Fuller component coverage is task 13.6.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { LiveAnnouncements } from "@/lib/RunMonitorContext";

let announcements: LiveAnnouncements = { assertive: [] };

vi.mock("@/lib/RunMonitorContext", async () => {
  const actual = await vi.importActual<typeof import("@/lib/RunMonitorContext")>(
    "@/lib/RunMonitorContext",
  );
  return {
    ...actual,
    useRunMonitor: () => ({ announcements }) as ReturnType<typeof actual.useRunMonitor>,
  };
});

import { RunLiveRegions } from "./RunLiveRegions";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  announcements = { assertive: [] };
});

function render(): void {
  act(() => {
    root.render(<RunLiveRegions />);
  });
}

describe("RunLiveRegions", () => {
  it("renders a polite status region and an assertive alert region", () => {
    render();
    const polite = container.querySelector('[role="status"]');
    const assertive = container.querySelector('[role="alert"]');
    expect(polite).not.toBeNull();
    expect(polite?.getAttribute("aria-live")).toBe("polite");
    expect(assertive).not.toBeNull();
    expect(assertive?.getAttribute("aria-live")).toBe("assertive");
  });

  it("renders the polite text and each assertive line", () => {
    announcements = {
      polite: "Run jobs running. 2 of 5 postings checked.",
      assertive: ["Error checking Engineer at Acme", "Run jobs failed"],
    };
    render();
    const polite = container.querySelector('[role="status"]');
    const assertive = container.querySelector('[role="alert"]');
    expect(polite?.textContent).toContain("2 of 5 postings checked");
    expect(assertive?.textContent).toContain("Error checking Engineer at Acme");
    expect(assertive?.textContent).toContain("Run jobs failed");
  });

  it("renders both regions even when there is nothing to announce", () => {
    render();
    expect(container.querySelector('[role="status"]')).not.toBeNull();
    expect(container.querySelector('[role="alert"]')).not.toBeNull();
    expect(container.querySelector('[role="status"]')?.textContent).toBe("");
  });
});
