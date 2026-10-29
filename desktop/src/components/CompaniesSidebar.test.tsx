// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CompaniesSidebar, type CompaniesSidebarProps } from "./CompaniesSidebar";
import { COMPANY_SORT_OPTIONS, COMPANY_VIEW_STORAGE_KEY } from "@/lib/company-sidebar";
import { formatShortDate } from "@/lib/instant";
import type { CompanyRow } from "@/lib/schema";

// The sidebar must sort and filter in memory; any backend call would show up on these mocks.
const apiMocks = vi.hoisted(() => ({ listCompanies: vi.fn() }));
vi.mock("@/lib/api", () => ({ api: apiMocks }));

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function row(id: string, name: string, overrides: Partial<Omit<CompanyRow, "company">> = {}): CompanyRow {
  const createdAt = "2026-01-01T00:00:00.000Z";
  return {
    company: { id, name, careersUrl: null, createdAt, updatedAt: createdAt },
    watches: [],
    reviews: [],
    openPositionCount: 0,
    trackedJobCount: 0,
    wishlistJobCount: 0,
    appliedJobCount: 0,
    interviewingJobCount: 0,
    lastActivityAt: null,
    lastAppliedAt: null,
    wishlistEaseAverage: null,
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

  it("lists all sort options in order with exactly one checked item for each active option (Req 2.1, 2.2)", () => {
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
    click(sortItem("Recently added company"));
    expect(host.querySelector(".companies-sidebar-dot")).not.toBeNull();
    expect(sortButton().className).toContain("is-active");
    expect(description().textContent).toContain("Recently added company");

    click(sortButton());
    click(sortItem("Name A–Z"));
    expect(host.querySelector(".companies-sidebar-dot")).toBeNull();
    expect(description().textContent).toContain("Name A–Z");
  });

  it("lists six filter toggles with checked state; toggling keeps menu open and focus (Req 3.1, 3.2, 7.1)", () => {
    render();
    click(filterButton());
    expect(filterItems().map((el) => el.textContent)).toEqual([
      "Watched",
      "Has open positions",
      "Has tracked jobs",
      "Needs attention",
      "Has wishlist jobs",
      "Has applied jobs",
    ]);
    for (const el of filterItems()) expect(el.getAttribute("aria-checked")).toBe("false");

    const target = filterItem("Has tracked jobs");
    target.focus();
    click(target);
    expect(filterMenu()).not.toBeNull();
    expect(document.activeElement).toBe(filterItem("Has tracked jobs"));
    expect(filterItem("Has tracked jobs").getAttribute("aria-checked")).toBe("true");
    expect(filterItem("Has tracked jobs").querySelector("svg")).not.toBeNull();
    for (const label of ["Watched", "Has open positions", "Needs attention", "Has wishlist jobs", "Has applied jobs"]) {
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
        sortItem("Recently added company").click();
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

  describe("companies-tab-sorting: sort menu layout, annotations, and pipeline filters", () => {
    const PIPELINE_ROWS: CompanyRow[] = [
      row("p1", "Acme", { wishlistJobCount: 1, appliedJobCount: 3, lastActivityAt: "2026-03-04" }),
      row("p2", "Bolt", { wishlistJobCount: 3, appliedJobCount: 0, lastActivityAt: null }),
      row("p3", "Cora", { wishlistJobCount: 0, appliedJobCount: 1, lastActivityAt: "2025-03-04" }),
    ];
    const LABELS = [
      "Name A–Z",
      "Name Z–A",
      "Recently added company",
      "Most open positions",
      "Most tracked jobs",
      "Most wishlist jobs",
      "Fewest wishlist jobs",
      "Most applied jobs",
      "Fewest applied jobs",
      "Most interviews",
      "Recently updated job",
      "Recently applied",
      "Easiest wishlist to apply",
    ];

    /** Visible text and sr-only label of each link's trailing annotation, or null when absent. */
    const annotations = () =>
      Array.from(host.querySelectorAll("a")).map((a) => {
        const trailing = a.children[1];
        if (!trailing) return null;
        return [
          trailing.querySelector('[aria-hidden="true"]')?.textContent,
          trailing.querySelector(".sr-only")?.textContent,
        ];
      });
    const badge = () => filterButton().querySelector(".companies-sidebar-badge")?.textContent ?? null;
    const countLine = () =>
      Array.from(host.querySelectorAll("p")).find((p) => / of \d+ companies$/.test(p.textContent ?? ""))?.textContent;
    const stored = () => JSON.parse(localStorage.getItem(COMPANY_VIEW_STORAGE_KEY)!);

    function arrow(key: "ArrowDown" | "ArrowUp") {
      act(() => {
        (document.activeElement ?? document.body).dispatchEvent(
          new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }),
        );
      });
    }

    function selectSort(label: string) {
      click(sortButton());
      click(sortItem(label));
    }

    beforeEach(() => {
      apiMocks.listCompanies.mockReset();
    });

    afterEach(() => {
      vi.useRealTimers();
      delete (Element.prototype as Partial<Element>).scrollIntoView;
    });

    it("renders 13 sort options in order with two non-focusable separators at the group boundaries (Req 2.1, 2.2)", () => {
      render();
      click(sortButton());
      expect(sortItems().map((el) => el.textContent)).toEqual(LABELS);
      const children = Array.from(sortMenu()!.children);
      const separators = children.filter((el) => el.getAttribute("role") === "separator");
      expect(separators).toHaveLength(2);
      for (const sep of separators) {
        expect(sep.hasAttribute("tabindex")).toBe(false);
        const before = children[children.indexOf(sep) - 1]?.textContent;
        const after = children[children.indexOf(sep) + 1]?.textContent;
        expect([before, after]).toEqual(
          sep === separators[0] ? ["Recently added company", "Most open positions"] : ["Most interviews", "Recently updated job"],
        );
      }
    });

    it("arrow keys skip separators and wrap (Req 2.3)", () => {
      render();
      click(sortButton());
      expect(document.activeElement).toBe(sortItem("Name A–Z"));
      arrow("ArrowUp");
      expect(document.activeElement).toBe(sortItem("Easiest wishlist to apply"));
      arrow("ArrowDown");
      expect(document.activeElement).toBe(sortItem("Name A–Z"));
      // A full cycle visits every option once and never lands on a separator.
      const visited: string[] = [];
      for (let i = 0; i < LABELS.length; i += 1) {
        arrow("ArrowDown");
        expect(document.activeElement?.getAttribute("role")).toBe("menuitemradio");
        visited.push(document.activeElement!.textContent!);
      }
      expect(visited).toEqual([...LABELS.slice(1), LABELS[0]]);
    });

    it("selecting each option checks exactly that item and persists it (Req 2.4, 2.8, 9.3)", () => {
      render();
      for (const label of [...LABELS.slice(1), LABELS[0]]) {
        selectSort(label);
        const value = COMPANY_SORT_OPTIONS.find((o) => o.label === label)!.value;
        expect(stored()).toEqual({ version: 1, sort: value, filters: [] });
        click(sortButton());
        const checked = sortItems().filter((el) => el.getAttribute("aria-checked") === "true");
        expect(checked.map((el) => el.textContent)).toEqual([label]);
        click(sortButton());
      }
    });

    it("caps the Sort_Menu height to the window, tracks resize, and scrolls focused items into view (Req 2.5)", () => {
      const scrolls: Array<[Element, unknown]> = [];
      Element.prototype.scrollIntoView = function (this: Element, arg?: boolean | ScrollIntoViewOptions) {
        scrolls.push([this, arg]);
      };
      vi.stubGlobal("innerHeight", 600);
      render();
      vi.spyOn(sortButton(), "getBoundingClientRect").mockReturnValue({ bottom: 400 } as DOMRect);
      click(sortButton());
      const menu = sortMenu()!;
      expect(menu.classList.contains("companies-sidebar-menu--sort")).toBe(true);
      expect(menu.style.maxHeight).toBe("186px");
      expect(scrolls.at(-1)).toEqual([sortItem("Name A–Z"), { block: "nearest" }]);

      arrow("ArrowDown");
      arrow("ArrowDown");
      arrow("ArrowDown");
      expect(scrolls.at(-1)).toEqual([sortItem("Most open positions"), { block: "nearest" }]);

      vi.stubGlobal("innerHeight", 450);
      act(() => {
        window.dispatchEvent(new Event("resize"));
      });
      // 450 - 400 - 14 = 36, clamped to the 120px minimum.
      expect(menu.style.maxHeight).toBe("120px");
    });

    it("switching sort reorders and replaces annotations together (Req 6.10)", () => {
      vi.useFakeTimers({ toFake: ["Date"] });
      vi.setSystemTime(new Date("2026-06-01T12:00:00Z"));
      render({ rows: PIPELINE_ROWS });
      expect(annotations()).toEqual([null, null, null]);

      selectSort("Most wishlist jobs");
      expect(linkNames()).toEqual(["Bolt", "Acme", "Cora"]);
      expect(annotations()).toEqual([
        ["3", "3 wishlist jobs"],
        ["1", "1 wishlist job"],
        ["0", "0 wishlist jobs"],
      ]);

      selectSort("Fewest applied jobs");
      expect(linkNames()).toEqual(["Cora", "Acme", "Bolt"]);
      expect(annotations()).toEqual([
        ["1", "1 applied job"],
        ["3", "3 applied jobs"],
        ["0", "0 applied jobs"],
      ]);

      selectSort("Recently updated job");
      const now = new Date();
      const recent = formatShortDate({ year: 2026, month: 3, day: 4 }, now);
      const older = formatShortDate({ year: 2025, month: 3, day: 4 }, now);
      expect(older).toContain("2025");
      expect(recent).not.toContain("2026");
      expect(linkNames()).toEqual(["Acme", "Cora", "Bolt"]);
      expect(annotations()).toEqual([
        [recent, `Last activity ${recent}`],
        [older, `Last activity ${older}`],
        null,
      ]);

      selectSort("Name A–Z");
      expect(annotations()).toEqual([null, null, null]);
    });

    it("new rows reorder and reannotate without changing sort or filters (Req 1.11, 3.11, 4.9, 8.6)", () => {
      render({ rows: PIPELINE_ROWS });
      selectSort("Most wishlist jobs");
      click(filterButton());
      click(filterItem("Has wishlist jobs"));
      expect(linkNames()).toEqual(["Bolt", "Acme"]);
      const view = localStorage.getItem(COMPANY_VIEW_STORAGE_KEY);

      render({
        rows: [
          row("p1", "Acme", { wishlistJobCount: 5 }),
          row("p2", "Bolt", { wishlistJobCount: 0 }),
          row("p3", "Cora", { wishlistJobCount: 2 }),
          row("p4", "Dune", { wishlistJobCount: 2 }),
        ],
      });
      expect(linkNames()).toEqual(["Acme", "Cora", "Dune"]);
      expect(annotations()).toEqual([
        ["5", "5 wishlist jobs"],
        ["2", "2 wishlist jobs"],
        ["2", "2 wishlist jobs"],
      ]);
      expect(filterItem("Has wishlist jobs").getAttribute("aria-checked")).toBe("true");
      expect(badge()).toBe("1");
      expect(sortButton().className).toContain("is-active");
      expect(localStorage.getItem(COMPANY_VIEW_STORAGE_KEY)).toBe(view);
    });

    it("pipeline filters update list, count line, and badge in memory; Clear filters clears all six (Req 7.2, 7.7, 7.8)", () => {
      render({ rows: PIPELINE_ROWS });
      selectSort("Most applied jobs");
      act(() => {
        const input = search();
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, "o");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      expect(linkNames()).toEqual(["Cora", "Bolt"]);

      click(filterButton());
      for (const label of ["Has wishlist jobs", "Has applied jobs"]) {
        const el = filterItem(label);
        expect(el.getAttribute("role")).toBe("menuitemcheckbox");
        expect(el.getAttribute("aria-checked")).toBe("false");
        expect(el.tabIndex).toBe(0);
      }

      click(filterItem("Has wishlist jobs"));
      expect(linkNames()).toEqual(["Bolt"]);
      expect(countLine()).toBe("1 of 3 companies");
      expect(badge()).toBe("1");

      click(filterItem("Has wishlist jobs"));
      click(filterItem("Has applied jobs"));
      expect(linkNames()).toEqual(["Cora"]);
      expect(badge()).toBe("1");

      click(filterItem("Has wishlist jobs"));
      expect(linkNames()).toEqual([]);
      // Search is also active, so the prior spec's precedence shows the search empty state; never "No companies yet."
      expect(host.textContent).toContain("No matching companies.");
      expect(host.textContent).not.toContain("No companies yet.");
      expect(countLine()).toBe("0 of 3 companies");
      expect(badge()).toBe("2");

      for (const label of ["Watched", "Has open positions", "Has tracked jobs", "Needs attention"]) {
        click(filterItem(label));
      }
      expect(badge()).toBe("6");

      click(clearButton());
      for (const el of filterItems()) expect(el.getAttribute("aria-checked")).toBe("false");
      expect(badge()).toBeNull();
      expect(search().value).toBe("o");
      expect(linkNames()).toEqual(["Cora", "Bolt"]);
      expect(stored()).toEqual({ version: 1, sort: "appliedJobsDesc", filters: [] });
      expect(apiMocks.listCompanies).not.toHaveBeenCalled();
    });

    it("keeps a new sort and filter applied when setItem throws, and never stores search text (Req 9.8, 9.9)", () => {
      const writes: string[] = [];
      vi.stubGlobal("localStorage", {
        getItem: () => null,
        setItem: (_key: string, value: string) => {
          writes.push(value);
          throw new Error("QuotaExceededError");
        },
        removeItem: () => {},
        clear: () => {},
      });
      render({ rows: PIPELINE_ROWS });
      act(() => {
        const input = search();
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, "zz-secret-or-a");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      act(() => {
        const input = search();
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
        setter.call(input, "");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      selectSort("Fewest wishlist jobs");
      expect(linkNames()).toEqual(["Acme", "Bolt", "Cora"]);
      click(filterButton());
      click(filterItem("Has applied jobs"));
      expect(linkNames()).toEqual(["Acme", "Cora"]);
      expect(filterItem("Has applied jobs").getAttribute("aria-checked")).toBe("true");
      expect(sortButton().className).toContain("is-active");
      expect(host.querySelector('[role="alert"]')).toBeNull();
      expect(writes.length).toBeGreaterThan(0);
      for (const value of writes) expect(value).not.toContain("zz-secret");
    });
  });
});
