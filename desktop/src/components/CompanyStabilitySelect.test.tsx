// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { CompanyStabilitySelect, COMPANY_STABILITY_SCALE_LABEL } from "./CompanyStabilitySelect";

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

it("offers unscored and 1–5 with stability labels, and emits numeric scores or null", () => {
  const change = vi.fn();
  act(() => root.render(<CompanyStabilitySelect id="company-stability" value={null} onChange={change} />));
  const select = container.querySelector("select")!;
  expect(COMPANY_STABILITY_SCALE_LABEL).toBe("Company stability (1–5, 5=most stable)");
  expect(select.getAttribute("aria-label")).toBe(COMPANY_STABILITY_SCALE_LABEL);
  expect([...select.options].map((option) => option.value)).toEqual(["", "1", "2", "3", "4", "5"]);
  expect([...select.options].map((option) => option.text)).toEqual([
    "Unscored",
    "1 — least stable",
    "2",
    "3",
    "4",
    "5 — most stable",
  ]);
  for (const value of ["1", "2", "3", "4", "5", ""]) {
    act(() => {
      select.value = value;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
  }
  expect(change.mock.calls.map(([value]) => value)).toEqual([1, 2, 3, 4, 5, null]);
});
