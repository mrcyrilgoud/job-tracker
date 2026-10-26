// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { StatusQuickChange } from "./StatusQuickChange";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

it("keeps retry inside a linked card instead of navigating away", () => {
  const retry = vi.fn();
  const navigate = vi.fn();
  act(() => root.render(
    <a href="/jobs/example" onClick={navigate}>
      <StatusQuickChange
        currentStatus="wishlist"
        onStatusChange={vi.fn()}
        error="Unable to save"
        onRetry={retry}
      />
    </a>,
  ));

  const retryButton = [...container.querySelectorAll("button")].find((button) => button.textContent === "Retry");
  expect(retryButton).toBeTruthy();
  act(() => retryButton!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true })));

  expect(retry).toHaveBeenCalledOnce();
  expect(navigate).not.toHaveBeenCalled();
});

it("opens a menu with all non-archived statuses and invokes the selected status", () => {
  const change = vi.fn();
  act(() => root.render(
    <StatusQuickChange currentStatus="wishlist" onStatusChange={change} />,
  ));

  const trigger = container.querySelector("button")!;
  act(() => trigger.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true })));

  const menu = document.body.querySelector('[role="menu"]')!;
  expect(menu).toBeTruthy();
  expect(menu.textContent).toContain("Applied");
  expect(menu.textContent).toContain("Closed");
  expect(menu.textContent).not.toContain("Archived");

  const applied = [...menu.querySelectorAll('[role="menuitemradio"]')].find((item) => item.textContent?.includes("Applied"));
  expect(applied).toBeTruthy();
  act(() => applied!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true })));
  expect(change).toHaveBeenCalledWith("applied");
});
