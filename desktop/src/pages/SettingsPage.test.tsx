// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { SettingsPage } from "./SettingsPage";

const mocks = vi.hoisted(() => ({
  csvConfig: vi.fn(), getFilterCriteria: vi.fn(), setFilterCriteria: vi.fn(),
  csvPathStatus: vi.fn(), configureCsv: vi.fn(), resetCsvConfig: vi.fn(), save: vi.fn(),
}));
vi.mock("@/lib/api", () => ({ api: mocks }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save }));
vi.mock("@/lib/tauri", () => ({ isDesktopShell: () => false }));

const criteria = {
  version: 1, title: { include: [], exclude: [], matchMode: "word" },
  location: { country: null, include: [], exclude: [], matchMode: "word" }, remote: "any",
};
let container: HTMLDivElement;
let root: Root;
let router: ReturnType<typeof createMemoryRouter>;

beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  mocks.csvConfig.mockResolvedValue({ path: "/default/jobs.csv", isCustom: false });
  mocks.getFilterCriteria.mockResolvedValue(structuredClone(criteria));
  mocks.setFilterCriteria.mockResolvedValue(undefined);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  router.dispose();
  container.remove();
  vi.unstubAllGlobals();
});
async function mount(url = "/settings") {
  router = createMemoryRouter([{ path: "/settings", element: <SettingsPage /> }], { initialEntries: [url] });
  await act(async () => root.render(<RouterProvider router={router} />));
}
function tab(value: string) {
  return container.querySelector<HTMLButtonElement>(`#settings-tab-${value}`)!;
}
function panel(value: string) {
  return container.querySelector<HTMLElement>(`#settings-panel-${value}`)!;
}
async function click(element: HTMLElement) {
  await act(async () => element.click());
}
function button(label: string) {
  return [...container.querySelectorAll("button")].find(element => element.textContent === label)!;
}
function expectSelected(value: string) {
  for (const current of ["data-storage", "search-filters"]) {
    expect(tab(current).getAttribute("aria-selected")).toBe(String(current === value));
    expect(tab(current).tabIndex).toBe(current === value ? 0 : -1);
    expect(panel(current).hidden).toBe(current !== value);
    expect(tab(current).getAttribute("aria-controls")).toBe(panel(current).id);
    expect(panel(current).getAttribute("aria-labelledby")).toBe(tab(current).id);
  }
}
it.each([
  ["/settings", "data-storage"],
  ["/settings?tab=unknown", "data-storage"],
  ["/settings?tab=data-storage", "data-storage"],
  ["/settings?tab=search-filters", "search-filters"],
])("selects the correct accessible panel for %s", async (url, selected) => {
  await mount(url);
  expectSelected(selected);
  expect(container.querySelector('[role="tablist"]')?.textContent).toContain("Data and Storage");
});

it("switches tabs through URL history while preserving other query parameters", async () => {
  await mount("/settings?source=shortcut");
  await click(tab("search-filters"));
  expect(router.state.location.search).toBe("?source=shortcut&tab=search-filters");
  await click(tab("data-storage"));
  expectSelected("data-storage");
  await act(async () => { await router.navigate(-1); });
  expectSelected("search-filters");
  await act(async () => { await router.navigate(1); });
  expectSelected("data-storage");
  expect(mocks.csvConfig).toHaveBeenCalledTimes(1);
  expect(mocks.getFilterCriteria).toHaveBeenCalledTimes(1);
});
it("supports arrows, wrapping, Home and End with focus following selection", async () => {
  await mount();
  for (const [from, key, to] of [
    ["data-storage", "ArrowRight", "search-filters"],
    ["search-filters", "ArrowRight", "data-storage"],
    ["data-storage", "ArrowLeft", "search-filters"],
    ["search-filters", "Home", "data-storage"],
    ["data-storage", "End", "search-filters"],
  ]) {
    await act(async () => tab(from).dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));
    expectSelected(to);
    expect(document.activeElement).toBe(tab(to));
  }
});
it("keeps both committed and draft filter inputs mounted across switching and saves explicitly", async () => {
  await mount("/settings?tab=search-filters");
  const input = container.querySelector<HTMLInputElement>('[data-testid="title-include-input"]')!;
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  await act(async () => {
    setValue.call(input, "engineer");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
  await act(async () => {
    setValue.call(input, "draft");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await click(tab("data-storage"));
  await click(tab("search-filters"));
  expect(container.querySelector('[data-testid="title-include-input"]')).toBe(input);
  expect(input.value).toBe("draft");
  expect(container.querySelector('[data-testid="title-include-chip"]')?.textContent).toContain("engineer");
  expect(mocks.setFilterCriteria).not.toHaveBeenCalled();
  await click(button("Save"));
  expect(mocks.setFilterCriteria).toHaveBeenCalledExactlyOnceWith({
    ...criteria, title: { ...criteria.title, include: ["engineer"] },
  });
  expect(panel("search-filters").querySelector('[role="status"]')?.textContent).toContain("Saved");
});
it.each([false, true])("preserves CSV confirmation for an existing file: %s", async (exists) => {
  await mount();
  mocks.save.mockResolvedValue("/custom/jobs.csv");
  mocks.csvPathStatus.mockResolvedValue({ path: "/custom/jobs.csv", exists });
  mocks.configureCsv.mockResolvedValue({ path: "/custom/jobs.csv", isCustom: true });
  await click(button("Choose CSV…"));
  expect(container.querySelector('[role="dialog"]')).not.toBeNull();
  expect(mocks.configureCsv).not.toHaveBeenCalled();
  await click(button(exists ? "Import and use" : "Confirm location"));
  expect(mocks.configureCsv).toHaveBeenCalledExactlyOnceWith("/custom/jobs.csv", exists ? "import" : "replace");
  expect(panel("data-storage").textContent).toContain("/custom/jobs.csv");
  expect(container.querySelector('[role="dialog"]')).toBeNull();
});
