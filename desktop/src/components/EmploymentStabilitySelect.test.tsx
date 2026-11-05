// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  EMPLOYMENT_STABILITY_OPTIONS,
  EMPLOYMENT_STABILITY_SCALE_LABEL,
  EmploymentStabilitySelect,
} from "./EmploymentStabilitySelect";

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

it("offers the exact unscored and employment-stability options", () => {
  const onChange = vi.fn();
  act(() => root.render(<EmploymentStabilitySelect id="employment-stability" value={null} onChange={onChange} />));

  const select = container.querySelector<HTMLSelectElement>("select")!;
  expect(EMPLOYMENT_STABILITY_SCALE_LABEL).toBe("Employment stability (1–5, 5=most secure)");
  expect(select.getAttribute("aria-label")).toBe(EMPLOYMENT_STABILITY_SCALE_LABEL);
  expect([...select.options].map((option) => ({ value: option.value, text: option.text }))).toEqual(
    EMPLOYMENT_STABILITY_OPTIONS.map((option) => ({ value: option.value, text: option.label })),
  );
  expect([...select.options].map((option) => option.value)).toEqual(["", "1", "2", "3", "4", "5"]);

  for (const value of ["1", "2", "3", "4", "5", ""]) {
    act(() => {
      select.value = value;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
  }
  expect(onChange.mock.calls.map(([value]) => value)).toEqual([1, 2, 3, 4, 5, null]);
});
