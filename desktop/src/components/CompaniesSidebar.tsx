import { Fragment, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { CheckIcon, FunnelIcon, SortIcon } from "./icons";
import { SidebarLink } from "./SidebarLink";
import {
  COMPANY_FILTERS,
  COMPANY_SORT_OPTIONS,
  DEFAULT_COMPANY_SORT,
  clearCompanyFilters,
  companyEmptyState,
  deriveCompanyList,
  loadCompanyViewState,
  saveCompanyViewState,
  sortAnnotationFor,
  toggleCompanyFilter,
  type CompanyFilter,
  type CompanySortOption,
  type CompanyViewState,
} from "../lib/company-sidebar";
import type { CompanyRow } from "../lib/schema";

export type CompaniesLoadStatus = "loading" | "ready" | "error";

export type CompaniesSidebarProps = {
  rows: CompanyRow[];
  load: CompaniesLoadStatus;
  activeCompanyId: string | undefined;
  /** Builds the link for a company; the sidebar never reads or writes URL params itself. */
  hrefForCompany: (companyId: string) => string;
};

type OpenMenu = "sort" | "filter" | null;

/** Sort_Menu offset below the Sort_Button plus bottom margin, in px. */
const SORT_MENU_VIEWPORT_GAP = 14;
/** Smallest Sort_Menu max-height, in px, so a very short window still shows a few options. */
const SORT_MENU_MIN_HEIGHT = 120;

/**
 * Focuses a menu item without letting the browser scroll ancestors, then scrolls the
 * menu's own box just enough to show the whole item (Req 2.5). jsdom lacks scrollIntoView.
 */
function focusMenuItem(item: HTMLElement | null | undefined) {
  if (!item) return;
  item.focus({ preventScroll: true });
  item.scrollIntoView?.({ block: "nearest" });
}

/** `window.localStorage`, or undefined when storage access throws (e.g. disabled storage). */
function safeLocalStorage(): Storage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}

/**
 * Companies list for the Jobs page sidebar: search, sort/filter controls, list,
 * refined-count line, and empty states. Sort and filters persist in localStorage;
 * search text stays in memory only.
 */
export function CompaniesSidebar({ rows, load, activeCompanyId, hrefForCompany }: CompaniesSidebarProps) {
  const [search, setSearch] = useState("");
  // Lazy init so the first render already uses the restored view state (no flash of default order).
  const [view, setView] = useState<CompanyViewState>(() => loadCompanyViewState(safeLocalStorage()));
  const [openMenu, setOpenMenu] = useState<OpenMenu>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const sortButtonRef = useRef<HTMLButtonElement>(null);
  const sortMenuRef = useRef<HTMLDivElement>(null);
  const activeSortItemRef = useRef<HTMLButtonElement>(null);
  const idBase = useId();
  const sortMenuId = `${idBase}-sort-menu`;
  const sortDescriptionId = `${idBase}-sort-description`;
  const sortLabel = COMPANY_SORT_OPTIONS.find((option) => option.value === view.sort)?.label ?? "";
  const filterButtonRef = useRef<HTMLButtonElement>(null);
  const filterMenuRef = useRef<HTMLDivElement>(null);
  const firstFilterItemRef = useRef<HTMLButtonElement>(null);
  const filterMenuId = `${idBase}-filter-menu`;
  const filterDescriptionId = `${idBase}-filter-description`;
  const activeFilterCount = view.filters.size;

  // Focus the active sort option when the Sort_Menu opens (Req 1.3),
  // or the first filter toggle when the Filter_Menu opens (Req 1.4).
  useEffect(() => {
    if (openMenu === "sort") focusMenuItem(activeSortItemRef.current);
    else if (openMenu === "filter") focusMenuItem(firstFilterItemRef.current);
  }, [openMenu]);

  // Cap the Sort_Menu height to the space below the Sort_Button so it scrolls internally (Req 2.5).
  // A layout effect runs before the passive focus effect above, so the menu is sized before the
  // active item is scrolled into view.
  useLayoutEffect(() => {
    if (openMenu !== "sort") return;
    function sizeSortMenu() {
      const menu = sortMenuRef.current;
      const button = sortButtonRef.current;
      if (!menu || !button) return;
      const available = window.innerHeight - button.getBoundingClientRect().bottom - SORT_MENU_VIEWPORT_GAP;
      menu.style.maxHeight = `${Math.max(SORT_MENU_MIN_HEIGHT, Math.floor(available))}px`;
    }
    sizeSortMenu();
    window.addEventListener("resize", sizeSortMenu);
    return () => window.removeEventListener("resize", sizeSortMenu);
  }, [openMenu]);

  // Shared dismissal and keyboard handling for whichever menu is open (Req 1.5, 1.6, 1.8),
  // modeled on the jobs Filter_Popover in JobsPage.
  useEffect(() => {
    if (openMenu === null) return;
    const buttonRef = openMenu === "sort" ? sortButtonRef : filterButtonRef;
    const menuRef = openMenu === "sort" ? sortMenuRef : filterMenuRef;

    // Outside click closes the menu; view state is untouched.
    function handlePointerDown(event: PointerEvent) {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (menuRef.current?.contains(target) || buttonRef.current?.contains(target)) return;
      setOpenMenu(null);
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setOpenMenu(null);
        buttonRef.current?.focus();
        return;
      }
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      const menu = menuRef.current;
      if (!menu) return;
      const items = Array.from(
        menu.querySelectorAll<HTMLElement>('[role="menuitemradio"], [role="menuitemcheckbox"], [role="menuitem"]'),
      );
      if (items.length === 0) return;
      // Only steer focus when it is inside the menu, so arrows elsewhere keep their default behavior.
      const active = document.activeElement;
      if (!(active instanceof Node) || !menu.contains(active)) return;
      event.preventDefault();
      const current = items.findIndex((item) => item === active);
      const step = event.key === "ArrowDown" ? 1 : -1;
      const next = current === -1 ? (step === 1 ? 0 : items.length - 1) : (current + step + items.length) % items.length;
      focusMenuItem(items[next]);
    }

    document.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [openMenu]);

  const derived = useMemo(() => deriveCompanyList(rows, search, view), [rows, search, view]);
  // Annotations derive from the same view as the order, so a sort change updates both in one render (Req 6.10).
  const visibleRows = useMemo(() => {
    const ctx = { now: new Date() };
    return derived.visible.map((row) => ({ row, annotation: sortAnnotationFor(row, view.sort, ctx) }));
  }, [derived.visible, view.sort]);
  const emptyState = companyEmptyState({
    load,
    total: derived.total,
    visibleCount: derived.visible.length,
    search,
    filterCount: view.filters.size,
  });

  /** Updates the view and persists it synchronously inside the handler. Write failures are ignored. */
  function commitView(next: CompanyViewState) {
    setView(next);
    saveCompanyViewState(safeLocalStorage(), next);
  }

  /** Toggles the Sort_Menu, or switches to it when the Filter_Menu is open. Focus stays on the button. */
  function toggleSortMenu() {
    setOpenMenu((current) => (current === "sort" ? null : "sort"));
  }

  /** Commits the sort only when it changed, then closes the menu and refocuses the Sort_Button. */
  function selectSort(sort: CompanySortOption) {
    if (sort !== view.sort) commitView({ ...view, sort });
    setOpenMenu(null);
    sortButtonRef.current?.focus();
  }

  /** Toggles the Filter_Menu, or switches to it when the Sort_Menu is open. Focus stays on the button. */
  function toggleFilterMenu() {
    setOpenMenu((current) => (current === "filter" ? null : "filter"));
  }

  /** Flips one filter; the menu stays open and focus stays on the toggled item. */
  function toggleFilter(filter: CompanyFilter) {
    commitView(toggleCompanyFilter(view, filter));
  }

  /** Clears all filters (sort and search kept); no-op when none are active. Menu stays open. */
  function clearFilters() {
    if (view.filters.size === 0) return;
    commitView(clearCompanyFilters(view));
  }

  /** Clears search and filters in one batched update, keeps the sort, and focuses the search input. */
  function reset() {
    setSearch("");
    setOpenMenu(null);
    commitView(clearCompanyFilters(view));
    searchInputRef.current?.focus();
  }

  if (emptyState === "loading") return null;

  if (emptyState === "error") {
    return (
      <p role="alert" className="text-sm text-[var(--faint)]">
        Couldn&apos;t load companies.
      </p>
    );
  }

  const showControls = load === "ready" && rows.length > 0;

  return (
    <>
      {showControls ? (
        <div className="relative mb-3 flex items-stretch gap-1.5">
          <input
            ref={searchInputRef}
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search companies…"
            aria-label="Search companies"
            className="field min-w-0 flex-1 py-2 text-xs"
          />
          {/* Square, shrink-0, no fixed height: the row's items-stretch makes it match the input height. */}
          <button
            ref={sortButtonRef}
            type="button"
            onClick={toggleSortMenu}
            aria-label="Sort companies"
            aria-haspopup="menu"
            aria-expanded={openMenu === "sort"}
            aria-controls={sortMenuId}
            aria-describedby={sortDescriptionId}
            className={`companies-sidebar-iconbtn shrink-0 ${view.sort !== DEFAULT_COMPANY_SORT ? "is-active" : ""}`}
          >
            <SortIcon size={16} />
            {view.sort !== DEFAULT_COMPANY_SORT ? <span className="companies-sidebar-dot" aria-hidden="true" /> : null}
          </button>
          <span id={sortDescriptionId} className="sr-only">
            Sorted by {sortLabel}
          </span>
          {openMenu === "sort" ? (
            <div
              ref={sortMenuRef}
              id={sortMenuId}
              role="menu"
              aria-label="Sort companies"
              className="companies-sidebar-menu companies-sidebar-menu--sort"
            >
              {COMPANY_SORT_OPTIONS.map((option, index) => {
                const checked = option.value === view.sort;
                const startsGroup = index > 0 && COMPANY_SORT_OPTIONS[index - 1]?.group !== option.group;
                return (
                  <Fragment key={option.value}>
                    {startsGroup ? <div role="separator" className="companies-sidebar-sep" /> : null}
                    <button
                      ref={checked ? activeSortItemRef : undefined}
                      type="button"
                      role="menuitemradio"
                      aria-checked={checked}
                      tabIndex={0}
                      onClick={() => selectSort(option.value)}
                      className="companies-sidebar-item"
                    >
                      <span className="companies-sidebar-check" aria-hidden="true">
                        {checked ? <CheckIcon size={14} /> : null}
                      </span>
                      {option.label}
                    </button>
                  </Fragment>
                );
              })}
            </div>
          ) : null}
          <button
            ref={filterButtonRef}
            type="button"
            onClick={toggleFilterMenu}
            aria-label="Filter companies"
            aria-haspopup="menu"
            aria-expanded={openMenu === "filter"}
            aria-controls={filterMenuId}
            aria-describedby={activeFilterCount > 0 ? filterDescriptionId : undefined}
            className={`companies-sidebar-iconbtn shrink-0 ${activeFilterCount > 0 ? "is-active" : ""}`}
          >
            <FunnelIcon size={16} />
            {activeFilterCount > 0 ? (
              <span className="companies-sidebar-badge" aria-hidden="true">
                {activeFilterCount}
              </span>
            ) : null}
          </button>
          {activeFilterCount > 0 ? (
            <span id={filterDescriptionId} className="sr-only">
              {activeFilterCount} {activeFilterCount === 1 ? "filter" : "filters"} active
            </span>
          ) : null}
          {openMenu === "filter" ? (
            <div
              ref={filterMenuRef}
              id={filterMenuId}
              role="menu"
              aria-label="Filter companies"
              className="companies-sidebar-menu"
            >
              {COMPANY_FILTERS.map((filter, index) => {
                const checked = view.filters.has(filter.value);
                return (
                  <button
                    key={filter.value}
                    ref={index === 0 ? firstFilterItemRef : undefined}
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={checked}
                    tabIndex={0}
                    onClick={() => toggleFilter(filter.value)}
                    className="companies-sidebar-item"
                  >
                    <span className="companies-sidebar-box" aria-hidden="true">
                      {checked ? <CheckIcon size={11} /> : null}
                    </span>
                    {filter.label}
                  </button>
                );
              })}
              <div role="separator" className="companies-sidebar-sep" />
              <button
                type="button"
                role="menuitem"
                aria-disabled={activeFilterCount === 0}
                tabIndex={0}
                onClick={clearFilters}
                className="companies-sidebar-item companies-sidebar-clear"
              >
                Clear filters
              </button>
            </div>
          ) : null}
        </div>
      ) : null}
      {derived.isRefined ? (
        <p aria-live="polite" className="mb-2 px-3 text-xs text-[var(--faint)]">
          {derived.visible.length} of {derived.total} companies
        </p>
      ) : null}
      <div className="max-h-80 space-y-1 overflow-y-auto pr-1">
        {visibleRows.map(({ row, annotation }) => (
          <SidebarLink
            key={row.company.id}
            to={hrefForCompany(row.company.id)}
            active={activeCompanyId === row.company.id}
            label={row.company.name}
            annotation={annotation ?? undefined}
          />
        ))}
        {emptyState === "noCompanies" ? (
          <p className="text-sm text-[var(--faint)]">No companies yet.</p>
        ) : emptyState === "noSearchMatch" || emptyState === "noFilterMatch" ? (
          <div className="space-y-2 px-3 py-2">
            <p className="text-sm text-[var(--faint)]">
              {emptyState === "noSearchMatch" ? "No matching companies." : "No companies match the selected filters."}
            </p>
            <button type="button" onClick={reset} className="btn btn-ghost btn-sm">
              Reset
            </button>
          </div>
        ) : null}
      </div>
    </>
  );
}
