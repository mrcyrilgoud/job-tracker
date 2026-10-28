// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CompaniesSidebar, type CompaniesSidebarProps } from "./CompaniesSidebar";
import { COMPANY_SORT_OPTIONS, COMPANY_VIEW_STORAGE_KEY } from "@/lib/company-sidebar";
import type { CompanyRow } from "@/lib/schema";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function row(id: string, name: string, overrides: Partial<Omit<CompanyRow, "company">> = {}): CompanyRow {
  const createdAt = "2026-01-01T00:00:00.000Z";
  return {
    company: { id, name, careersUrl: null, createdAt, updatedAt: createdAt },
    watches: [],
    reviews: [],
    openPositionCount: 0,
    trackedJobCount: 0,
    ...overrides,
  };
}

const ROWS: CompanyRow[] = [
  row("c1", "AKASA", { openPositionCount: 3, trackedJobCount: 1 }),
  row("c2", "Abby Care", { openPositionCount: 0, trackedJobCount: 2 }),
  row("c3", "beta", { openPositionCount: 5, trackedJobCount: 0 }),
];

describe("CompaniesSidebar menus, focus, and ARIA", () => {
  let root: Root;
  let host: HTMLDivElement;

  beforeEach(() => {
    // Fresh in-memory storage per test (Node's own localStorage global can shadow jsdom's).
    const storage = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => void storage.set(key, value),
      removeItem: (key: string) => void storage.delete(key),
      clear: () => storage.clear(),
    });
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.unstubAllGlobals();
  });

  function render(props: Partial<CompaniesSidebarProps> = {}) {
    const full: CompaniesSidebarProps = {
      rows: ROWS,
      load: "ready",
      activeCompanyId: undefined,
      hrefForCompany: (id) => `/jobs?companyId=${id}`,
      ...props,
    };
    act(() => {
      root.render(createElement(MemoryRouter, null, createElement(CompaniesSidebar, full)));
    });
  }

  const search = () => host.querySelector<HTMLInputElement>('input[type="search"]')!;
  const sortButton = () => host.querySelector<HTMLButtonElement>('button[aria-label="Sort companies"]')!;
  const filterButton = () => host.querySelector<HTMLButtonElement>('button[aria-label="Filter companies"]')!;
  const sortMenu = () => host.querySelector<HTMLElement>('[role="menu"][aria-label="Sort companies"]');
  const filterMenu = () => host.querySelector<HTMLElement>('[role="menu"][aria-label="Filter companies"]');
  const sortItems = () => Array.from(host.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]'));
  const filterItems = () => Array.from(host.querySelectorAll<HTMLButtonElement>('[role="menuitemcheckbox"]'));
  const clearButton = () =>
    Array.from(host.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')).find((el) => el.textContent === "Clear filters")!;
  const linkNames = () => Array.from(host.querySelectorAll("a")).map((a) => a.querySelector("span span")?.textContent);
  const sortItem = (label: string) => sortItems().find((el) => el.textContent?.includes(label))!;
  const filterItem = (label: string) => filterItems().find((el) => el.textContent?.includes(label))!;

  function click(el: HTMLElement) {
    act(() => el.click());
  }

  function pressEscape() {
    act(() => {
      (document.activeElement ?? document.body).dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
      );
    });
  }

  function pointerDownOn(target: EventTarget) {
    act(() => {
      target.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    });
  }

  it("renders search, Sort_Button, Filter_Button in that order on one non-wrapping stretched row (Req 1.1, 1.2)", () => {
    render();
    const rowEl = search().parentElement!;
    expect(sortButton().parentElement).toBe(rowEl);
    expect(filterButton().parentElement).toBe(rowEl);
    const classes = rowEl.className.split(/\s+/);
    expect(classes).toContain("flex");
    expect(classes).toContain("items-stretch");
    expect(classes).not.toContain("flex-wrap");
    const children = Array.from(rowEl.children);
    const order = [search(), sortButton(), filterButton()].map((el) => children.indexOf(el));
    expect(order[0]).toBeLessThan(order[1]);
    expect(order[1]).toBeLessThan(order[2]);
    // No fixed heights on the buttons: items-stretch sizes them to the input (jsdom has no layout).
    for (const button of [sortButton(), filterButton()]) {
      expect(button.className).not.toMatch(/(^|\s)(h|min-h)-/);
      expect(button.className).toContain("shrink-0");
    }
  });

  it("exposes accessible names, aria-haspopup, and aria-expanded on both buttons (Req 1.7)", () => {
    render();
    for (const button of [sortButton(), filterButton()]) {
      expect(button.getAttribute("aria-haspopup")).toBe("menu");
      expect(button.getAttribute("aria-expanded")).toBe("false");
    }
    click(sortButton());
    expect(sortButton().getAttribute("aria-expanded")).toBe("true");
    expect(filterButton().getAttribute("aria-expanded")).toBe("false");
    expect(sortButton().getAttribute("aria-controls")).toBe(sortMenu()!.id);
    click(filterButton());
    expect(sortButton().getAttribute("aria-expanded")).toBe("false");
    expect(filterButton().getAttribute("aria-expanded")).toBe("true");
    expect(filterButton().getAttribute("aria-controls")).toBe(filterMenu()!.id);
  });

  it("opens the Sort_Menu and focuses the active option (Req 1.3)", () => {
    localStorage.setItem(COMPANY_VIEW_STORAGE_KEY, JSON.stringify({ version: 1, sort: "trackedJobsDesc", filters: [] }));
    render();
    click(sortButton());
    expect(sortMenu()).not.toBeNull();
    expect(document.activeElement).toBe(sortItem("Most tracked jobs"));
  });

  it("opens the Filter_Menu and focuses the first filter toggle (Req 1.4)", () => {
    render();
    click(filterButton());
    expect(filterMenu()).not.toBeNull();
    expect(document.activeElement).toBe(filterItems()[0]);
  });

  it("Escape closes each menu and refocuses its button (Req 1.5)", () => {
    render();
    click(sortButton());
    pressEscape();
    expect(sortMenu()).toBeNull();
    expect(document.activeElement).toBe(sortButton());

    click(filterButton());
    pressEscape();
    expect(filterMenu()).toBeNull();
    expect(document.activeElement).toBe(filterButton());
  });

  it("outside click closes the menu without changing view state; clicks inside or on the button do not (Req 1.6)", () => {
    render();
    click(filterButton());
    click(filterItem("Has open positions"));
    const stored = localStorage.getItem(COMPANY_VIEW_STORAGE_KEY);

    pointerDownOn(filterMenu()!);
    expect(filterMenu()).not.toBeNull();
    pointerDownOn(filterButton());
    expect(filterMenu()).not.toBeNull();

    pointerDownOn(document.body);
    expect(filterMenu()).toBeNull();
    expect(localStorage.getItem(COMPANY_VIEW_STORAGE_KEY)).toBe(stored);
    expect(linkNames()).toEqual(["AKASA", "beta"]);

    click(sortButton());
    pointerDownOn(document.body);
    expect(sortMenu()).toBeNull();
    expect(sortButton().className).not.toContain("is-active");
  });

  it("re-activating the open menu's button closes it and keeps focus on the button (Req 1.10)", () => {
    render();
    for (const [button, menu] of [
      [sortButton, sortMenu],
      [filterButton, filterMenu],
    ] as const) {
      click(button());
      expect(menu()).not.toBeNull();
      button().focus();
      click(button());
      expect(menu()).toBeNull();
      expect(document.activeElement).toBe(button());
      expect(button().getAttribute("aria-expanded")).toBe("false");
    }
  });

  it("switching buttons leaves exactly one menu open (Req 1.11)", () => {
    render();
    click(sortButton());
    click(filterButton());
    expect(sortMenu()).toBeNull();
    expect(filterMenu()).not.toBeNull();
    click(sortButton());
    expect(filterMenu()).toBeNull();
    expect(sortMenu()).not.toBeNull();
    expect(host.querySelectorAll('[role="menu"]')).toHaveLength(1);
  });

  it("buttons and menu items are keyboard-focusable native buttons (Req 1.8)", () => {
    render();
    function expectKeyboardOperable(controls: HTMLButtonElement[]) {
      for (const el of controls) {
        // Native <button type="button"> gets Enter/Space activation from the browser; jsdom lacks Tab traversal.
        expect(el.tagName).toBe("BUTTON");
        expect(el.getAttribute("type")).toBe("button");
        expect(el.tabIndex).toBe(0);
        expect(el.hasAttribute("disabled")).toBe(false);
        el.focus();
        expect(document.activeElement).toBe(el);
      }
    }
    expectKeyboardOperable([sortButton(), filterButton()]);
    click(sortButton());
    expectKeyboardOperable(sortItems());
    click(filterButton());
    expectKeyboardOperable([...filterItems(), clearButton()]);
  });

  it("lists the five sort options in order with exactly one checked item for each active option (Req 2.1, 2.2)", () => {
    render();
    for (const option of COMPANY_SORT_OPTIONS) {
      click(sortButton());
      expect(sortItems().map((el) => el.textContent)).toEqual(COMPANY_SORT_OPTIONS.map((o) => o.label));
      click(sortItem(option.label));
      click(sortButton());
      const checked = sortItems().filter((el) => el.getAttribute("aria-checked") === "true");
      expect(checked).toHaveLength(1);
      expect(checked[0].textContent).toBe(option.label);
      const withCheckIcon = sortItems().filter((el) => el.querySelector(".companies-sidebar-check svg"));
      expect(withCheckIcon).toEqual(checked);
      for (const el of sortItems()) {
        if (el !== checked[0]) expect(el.getAttribute("aria-checked")).toBe("false");
      }
      click(sortButton());
    }
  });

  it("selecting a sort reorders, closes the menu, and refocuses the Sort_Button (Req 2.3)", () => {
    render();
    expect(linkNames()).toEqual(["Abby Care", "AKASA", "beta"]);
    click(sortButton());
    click(sortItem("Most open positions"));
    expect(linkNames()).toEqual(["beta", "AKASA", "Abby Care"]);
    expect(sortMenu()).toBeNull();
    expect(document.activeElement).toBe(sortButton());
  });

  it("re-selecting the active sort changes nothing and closes the menu (Req 2.4)", () => {
    render();
    const before = linkNames();
    click(sortButton());
    click(sortItem("Name A–Z"));
    expect(linkNames()).toEqual(before);
    expect(sortMenu()).toBeNull();
    expect(localStorage.getItem(COMPANY_VIEW_STORAGE_KEY)).toBeNull();
  });

  it("shows the sort dot only for non-default sorts and describes the active sort (Req 2.11)", () => {
    render();
    const description = () => document.getElementById(sortButton().getAttribute("aria-describedby")!)!;
    expect(host.querySelector(".companies-sidebar-dot")).toBeNull();
    expect(sortButton().className).not.toContain("is-active");
    expect(description().textContent).toContain("Name A–Z");

    click(sortButton());
    click(sortItem("Recently added"));
    expect(host.querySelector(".companies-sidebar-dot")).not.toBeNull();
    expect(sortButton().className).toContain("is-active");
    expect(description().textContent).toContain("Recently added");

    click(sortButton());
    click(sortItem("Name A–Z"));
    expect(host.querySelector(".companies-sidebar-dot")).toBeNull();
    expect(description().textContent).toContain("Name A–Z");
  });

  it("lists four filter toggles with checked state; toggling keeps menu open and focus (Req 3.1, 3.2)", () => {
    render();
    click(filterButton());
    expect(filterItems().map((el) => el.textContent)).toEqual([
      "Watched",
      "Has open positions",
      "Has tracked jobs",
      "Needs attention",
    ]);
    for (const el of filterItems()) expect(el.getAttribute("aria-checked")).toBe("false");

    const target = filterItem("Has tracked jobs");
    target.focus();
    click(target);
    expect(filterMenu()).not.toBeNull();
    expect(document.activeElement).toBe(filterItem("Has tracked jobs"));
    expect(filterItem("Has tracked jobs").getAttribute("aria-checked")).toBe("true");
    expect(filterItem("Has tracked jobs").querySelector("svg")).not.toBeNull();
    for (const label of ["Watched", "Has open positions", "Needs attention"]) {
      expect(filterItem(label).getAttribute("aria-checked")).toBe("false");
    }
    expect(linkNames()).toEqual(["Abby Care", "AKASA"]);

    click(filterItem("Has tracked jobs"));
    expect(filterItem("Has tracked jobs").getAttribute("aria-checked")).toBe("false");
    expect(linkNames()).toEqual(["Abby Care", "AKASA", "beta"]);
  });

  it("shows a badge with the active filter count, none at zero (Req 3.5)", () => {
    render();
    const badge = () => filterButton().querySelector(".companies-sidebar-badge");
    expect(badge()).toBeNull();
    click(filterButton());
    click(filterItem("Watched"));
    expect(badge()?.textContent).toBe("1");
    click(filterItem("Has open positions"));
    click(filterItem("Has tracked jobs"));
    click(filterItem("Needs attention"));
    expect(badge()?.textContent).toBe("4");
    click(filterItem("Watched"));
    click(filterItem("Has open positions"));
    click(filterItem("Has tracked jobs"));
    click(filterItem("Needs attention"));
    expect(badge()).toBeNull();
  });

  it("places Clear filters after the toggles, disabled at zero filters (Req 3.6, 3.10)", () => {
    render();
    click(filterButton());
    const items = Array.from(filterMenu()!.querySelectorAll("button"));
    expect(items.indexOf(clearButton())).toBe(items.length - 1);
    for (const toggle of filterItems()) {
      expect(items.indexOf(toggle)).toBeLessThan(items.indexOf(clearButton()));
    }
    expect(clearButton().getAttribute("aria-disabled")).toBe("true");
    click(clearButton());
    expect(filterMenu()).not.toBeNull();
    expect(localStorage.getItem(COMPANY_VIEW_STORAGE_KEY)).toBeNull();
    expect(linkNames()).toEqual(["Abby Care", "AKASA", "beta"]);

    click(filterItem("Watched"));
    expect(clearButton().getAttribute("aria-disabled")).toBe("false");
  });

  it("Clear filters deactivates every filter and keeps sort, search, and the open menu (Req 3.9)", () => {
    render();
    act(() => {
      const input = search();
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input, " a ");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    click(sortButton());
    click(sortItem("Most open positions"));
    click(filterButton());
    click(filterItem("Has open positions"));
    click(filterItem("Has tracked jobs"));
    expect(linkNames()).toEqual(["AKASA"]);

    click(clearButton());
    expect(filterMenu()).not.toBeNull();
    for (const el of filterItems()) expect(el.getAttribute("aria-checked")).toBe("false");
    expect(filterButton().querySelector(".companies-sidebar-badge")).toBeNull();
    expect(search().value).toBe(" a ");
    expect(linkNames()).toEqual(["beta", "AKASA", "Abby Care"]);
    expect(JSON.parse(localStorage.getItem(COMPANY_VIEW_STORAGE_KEY)!)).toEqual({
      version: 1,
      sort: "openPositionsDesc",
      filters: [],
    });
  });

  // Shares the root/host/localStorage setup and query helpers above.
  describe("persistence, search, list, and empty states", () => {
    const EMPTY_MESSAGES = ["No companies yet.", "No matching companies.", "No companies match the selected filters."];

    function type(value: string) {
      act(() => {
        const input = search();
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    }

    function remount(props: Partial<CompaniesSidebarProps> = {}) {
      act(() => root.unmount());
      root = createRoot(host);
      render(props);
    }

    const stored = () => JSON.parse(localStorage.getItem(COMPANY_VIEW_STORAGE_KEY)!);
    const resetButton = () =>
      Array.from(host.querySelectorAll<HTMLButtonElement>("button")).find((el) => el.textContent === "Reset");
    const countLine = () =>
      Array.from(host.querySelectorAll("p")).find((p) => / of \d+ companies$/.test(p.textContent ?? ""))?.textContent;
    const activeLinks = () =>
      Array.from(host.querySelectorAll("a"))
        .filter((a) => a.className.includes("bg-[var(--accent-soft)]"))
        .map((a) => a.querySelector("span span")?.textContent);

    it("writes storage synchronously inside the click handler (Req 6.1)", () => {
      render();
      const setItem = vi.spyOn(localStorage, "setItem");
      click(sortButton());
      act(() => {
        sortItem("Recently added").click();
        // Checked before act flushes renders/effects: the write happened in the handler itself.
        expect(setItem).toHaveBeenCalledTimes(1);
      });
      expect(stored()).toEqual({ version: 1, sort: "recentlyAdded", filters: [] });

      click(filterButton());
      act(() => {
        filterItem("Has open positions").click();
        expect(setItem).toHaveBeenCalledTimes(2);
      });
      expect(stored()).toEqual({ version: 1, sort: "recentlyAdded", filters: ["hasOpenPositions"] });
    });

    it("applies pre-seeded storage on the first render (Req 6.2)", () => {
      localStorage.setItem(
        COMPANY_VIEW_STORAGE_KEY,
        JSON.stringify({ version: 1, sort: "nameDesc", filters: ["hasOpenPositions"] }),
      );
      render();
      expect(linkNames()).toEqual(["beta", "AKASA"]);
      expect(filterButton().querySelector(".companies-sidebar-badge")?.textContent).toBe("1");
      expect(sortButton().className).toContain("is-active");
    });

    it("never writes search text and starts with an empty search on mount (Req 6.7)", () => {
      render();
      const setItem = vi.spyOn(localStorage, "setItem");
      type("zz-secret");
      expect(setItem).not.toHaveBeenCalled();
      click(filterButton());
      click(filterItem("Has tracked jobs"));
      expect(setItem).toHaveBeenCalled();
      for (const [, value] of setItem.mock.calls) expect(value).not.toContain("zz-secret");

      remount();
      expect(search().value).toBe("");
      expect(filterButton().querySelector(".companies-sidebar-badge")?.textContent).toBe("1");
    });

    it("search edits keep the view state; view changes keep the search text, whitespace included (Req 5.5, 5.6)", () => {
      render();
      click(sortButton());
      click(sortItem("Most tracked jobs"));
      click(filterButton());
      click(filterItem("Has tracked jobs"));
      const before = localStorage.getItem(COMPANY_VIEW_STORAGE_KEY);

      type("  a  ");
      expect(localStorage.getItem(COMPANY_VIEW_STORAGE_KEY)).toBe(before);
      expect(sortButton().className).toContain("is-active");
      expect(filterButton().querySelector(".companies-sidebar-badge")?.textContent).toBe("1");
      expect(linkNames()).toEqual(["Abby Care", "AKASA"]);

      // Filter_Menu is still open (typing doesn't dismiss it).
      expect(filterMenu()).not.toBeNull();
      click(filterItem("Has open positions"));
      expect(search().value).toBe("  a  ");
      click(sortButton());
      click(sortItem("Name Z–A"));
      expect(search().value).toBe("  a  ");
      expect(linkNames()).toEqual(["AKASA"]);
    });

    it("shows the 'N of M companies' line only while refined (Req 5.9, 5.10)", () => {
      render();
      expect(countLine()).toBeUndefined();
      type("ak");
      expect(countLine()).toBe("1 of 3 companies");
      type("   ");
      expect(countLine()).toBeUndefined();
      click(filterButton());
      click(filterItem("Has open positions"));
      expect(countLine()).toBe("2 of 3 companies");
      click(filterItem("Has open positions"));
      expect(countLine()).toBeUndefined();
    });

    it("marks only the active company, and none when it is excluded (Req 5.11)", () => {
      render({ activeCompanyId: "c1" });
      expect(activeLinks()).toEqual(["AKASA"]);
      type("beta");
      expect(linkNames()).toEqual(["beta"]);
      expect(activeLinks()).toEqual([]);
      type("");
      expect(activeLinks()).toEqual(["AKASA"]);
    });

    it("zero companies hides controls and shows 'No companies yet.' without Reset (Req 1.9, 7.1)", () => {
      render({ rows: [] });
      expect(host.textContent).toContain("No companies yet.");
      expect(host.querySelector('input[type="search"]')).toBeNull();
      expect(host.querySelector('button[aria-label="Sort companies"]')).toBeNull();
      expect(host.querySelector('button[aria-label="Filter companies"]')).toBeNull();
      expect(host.querySelectorAll("a")).toHaveLength(0);
      expect(resetButton()).toBeUndefined();
    });

    it("shows 'No matching companies.' with Reset when search leaves nothing (Req 7.2, 7.5)", () => {
      render();
      type("zzz");
      expect(host.textContent).toContain("No matching companies.");
      expect(host.textContent).not.toContain("No companies match the selected filters.");
      expect(host.querySelectorAll("a")).toHaveLength(0);
      expect(search()).not.toBeNull();
      expect(sortButton()).not.toBeNull();
      expect(filterButton()).not.toBeNull();
      const reset = resetButton()!;
      expect(reset.getAttribute("type")).toBe("button");
      reset.focus();
      expect(document.activeElement).toBe(reset);
    });

    it("shows 'No companies match the selected filters.' with Reset when only filters leave nothing (Req 7.4)", () => {
      render();
      click(filterButton());
      click(filterItem("Watched"));
      expect(host.textContent).toContain("No companies match the selected filters.");
      expect(host.textContent).not.toContain("No matching companies.");
      expect(host.querySelectorAll("a")).toHaveLength(0);
      expect(resetButton()).toBeDefined();
      expect(search()).not.toBeNull();
      expect(sortButton()).not.toBeNull();
    });

    it("Reset clears search and filters, keeps sort, writes storage, and focuses search (Req 7.3)", () => {
      render();
      click(sortButton());
      click(sortItem("Most open positions"));
      click(filterButton());
      click(filterItem("Watched"));
      type("zzz");
      expect(host.textContent).toContain("No matching companies.");

      click(resetButton()!);
      expect(search().value).toBe("");
      expect(filterButton().querySelector(".companies-sidebar-badge")).toBeNull();
      expect(stored()).toEqual({ version: 1, sort: "openPositionsDesc", filters: [] });
      expect(linkNames()).toEqual(["beta", "AKASA", "Abby Care"]);
      expect(document.activeElement).toBe(search());
      expect(resetButton()).toBeUndefined();
    });

    it("shows no empty-state message while loading (Req 7.6)", () => {
      render({ load: "loading", rows: [] });
      for (const message of EMPTY_MESSAGES) expect(host.textContent).not.toContain(message);
      expect(host.querySelectorAll("a")).toHaveLength(0);
    });

    it("shows an alert with no links or controls on error and leaves storage untouched (Req 7.7)", () => {
      const seeded = JSON.stringify({ version: 1, sort: "nameDesc", filters: ["watched"] });
      localStorage.setItem(COMPANY_VIEW_STORAGE_KEY, seeded);
      const setItem = vi.spyOn(localStorage, "setItem");
      render({ load: "error", rows: [] });
      const alert = host.querySelector('[role="alert"]');
      expect(alert?.textContent).toMatch(/couldn.t load companies/i);
      expect(host.textContent).not.toContain("No companies yet.");
      expect(host.querySelectorAll("a")).toHaveLength(0);
      expect(host.querySelector("input")).toBeNull();
      expect(host.querySelectorAll("button")).toHaveLength(0);
      expect(setItem).not.toHaveBeenCalled();
      expect(localStorage.getItem(COMPANY_VIEW_STORAGE_KEY)).toBe(seeded);
    });

    it("keeps the in-memory selection when setItem throws (Req 6.6)", () => {
      vi.stubGlobal("localStorage", {
        getItem: () => null,
        setItem: () => {
          throw new Error("QuotaExceededError");
        },
        removeItem: () => {},
        clear: () => {},
      });
      render();
      click(sortButton());
      click(sortItem("Most open positions"));
      expect(linkNames()).toEqual(["beta", "AKASA", "Abby Care"]);
      expect(sortButton().className).toContain("is-active");
      click(filterButton());
      click(filterItem("Has open positions"));
      expect(filterItem("Has open positions").getAttribute("aria-checked")).toBe("true");
      expect(linkNames()).toEqual(["beta", "AKASA"]);
      expect(host.querySelector('[role="alert"]')).toBeNull();
    });
  });
});
