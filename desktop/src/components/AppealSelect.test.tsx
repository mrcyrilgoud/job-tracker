// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AppealSelect, APPEAL_SCALE_LABEL } from "./AppealSelect";

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
it("offers only unscored and 1–5, and emits numeric scores or null", () => {
  const change = vi.fn();
  act(() => root.render(<AppealSelect id="appeal" value={null} onChange={change} />));
  const select = container.querySelector("select")!;
  expect(select.getAttribute("aria-label")).toBe(APPEAL_SCALE_LABEL);
  expect([...select.options].map(option => option.value)).toEqual(["", "1", "2", "3", "4", "5"]);
  for (const value of ["1", "2", "3", "4", "5", ""]) {
    act(() => {
      select.value = value;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
  }
  expect(change.mock.calls.map(([value]) => value)).toEqual([1, 2, 3, 4, 5, null]);
});
it("keeps a compact score editable without activating the job card link", () => {
  const navigate = vi.fn();
  act(() => root.render(<a href="/jobs/example" onClick={navigate}>
    <AppealSelect id="appeal" compact value={5} onChange={vi.fn()} />
  </a>));
  const select = container.querySelector("select")!;
  expect(select.value).toBe("5");
  act(() => select.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true })));
  expect(navigate).not.toHaveBeenCalled();
});
