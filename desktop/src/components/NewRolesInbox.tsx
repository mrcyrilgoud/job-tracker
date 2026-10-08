import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";

import { NewRolesList, type TriageAction } from "@/components/companies/NewRolesList";
import {
  filterNewRoles,
  groupNewRolesByCompany,
  newRolesCompanyOptions,
  newRolesFiltersFromParams,
  sortNewRolesNewestFirst,
} from "@/lib/new-roles-filters";
import type { JobListItem } from "@/lib/schema";

function salaryDraftBound(value: string): number | undefined | null {
  if (!value.trim()) return undefined;
  if (!/^\d+$/.test(value.trim())) return null;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

export function NewRolesInbox({
  roles,
  onTriage,
  isPending,
}: {
  roles: JobListItem[];
  onTriage: (jobId: string, action: TriageAction) => void;
  isPending: (jobId: string) => boolean;
}) {
  const [searchParams, setSearchParams] = useSearchParams();
  const filters = newRolesFiltersFromParams(searchParams);
  const urlSearch = searchParams.get("search") ?? "";
  const [searchDraft, setSearchDraft] = useState(urlSearch);
  const searchDraftRef = useRef(searchDraft);
  searchDraftRef.current = searchDraft;
  const [postingStateDraft, setPostingStateDraft] = useState(filters.postingState);
  const [salaryMinDraft, setSalaryMinDraft] = useState(filters.salaryMin?.toString() ?? "");
  const [salaryMaxDraft, setSalaryMaxDraft] = useState(filters.salaryMax?.toString() ?? "");
  const [filterError, setFilterError] = useState<string | null>(null);
  const [filterPopoverOpen, setFilterPopoverOpen] = useState(false);
  const filterButtonRef = useRef<HTMLButtonElement>(null);
  const filterPopoverRef = useRef<HTMLDivElement>(null);
  const salaryMinInputRef = useRef<HTMLInputElement>(null);

  const updateParams = useCallback((values: Record<string, string | null>, replace = false) => {
    setSearchParams((current) => {
      const next = new URLSearchParams(current);
      for (const [key, value] of Object.entries(values)) {
        if (value === null || value === "") next.delete(key);
        else next.set(key, value);
      }
      return next;
    }, { replace });
  }, [setSearchParams]);

  useEffect(() => {
    if (urlSearch !== searchDraftRef.current.trim()) {
      searchDraftRef.current = urlSearch;
      setSearchDraft(urlSearch);
    }
  }, [urlSearch]);

  useEffect(() => {
    const normalized = searchDraft.trim();
    if (normalized === urlSearch) return;
    const timer = window.setTimeout(() => {
      updateParams({ search: normalized || null }, true);
    }, 250);
    return () => window.clearTimeout(timer);
  }, [searchDraft, updateParams, urlSearch]);

  useEffect(() => {
    setPostingStateDraft(filters.postingState);
    setSalaryMinDraft(filters.salaryMin?.toString() ?? "");
    setSalaryMaxDraft(filters.salaryMax?.toString() ?? "");
    setFilterError(null);
  }, [filters.postingState, filters.salaryMin, filters.salaryMax]);

  useEffect(() => {
    if (!filterPopoverOpen) return;

    salaryMinInputRef.current?.focus();

    function handlePointerDown(event: PointerEvent) {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (filterPopoverRef.current?.contains(target) || filterButtonRef.current?.contains(target)) return;
      setFilterPopoverOpen(false);
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setFilterPopoverOpen(false);
      filterButtonRef.current?.focus();
    }

    document.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [filterPopoverOpen]);

  const companyOptions = useMemo(() => newRolesCompanyOptions(roles), [roles]);

  useEffect(() => {
    if (filters.companyId && !companyOptions.some((company) => company.id === filters.companyId)) {
      updateParams({ companyId: null }, true);
    }
  }, [companyOptions, filters.companyId, updateParams]);

  const filteredRoles = useMemo(
    () => filterNewRoles(roles, { ...filters, search: searchDraft.trim() }),
    [
      roles,
      filters.companyId,
      filters.postingState,
      filters.salaryMin,
      filters.salaryMax,
      searchDraft,
    ],
  );
  const flatRoles = useMemo(() => sortNewRolesNewestFirst(filteredRoles), [filteredRoles]);
  const companyGroups = useMemo(() => groupNewRolesByCompany(filteredRoles), [filteredRoles]);
  const hasSalaryFilter = filters.salaryMin !== undefined || filters.salaryMax !== undefined;
  const activeFilterCount = Number(Boolean(filters.postingState)) + Number(hasSalaryFilter);
  const hasActiveFilters = Boolean(
    searchDraft.trim() || filters.companyId || filters.postingState || hasSalaryFilter,
  );

  function handleApplyFilters(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const salaryMin = salaryDraftBound(salaryMinDraft);
    const salaryMax = salaryDraftBound(salaryMaxDraft);
    if (salaryMin === null || salaryMax === null) {
      setFilterError("Enter a whole dollar amount for each salary bound.");
      return;
    }
    if (salaryMin !== undefined && salaryMax !== undefined && salaryMin > salaryMax) {
      setFilterError("Minimum salary must not exceed maximum salary.");
      return;
    }

    setFilterError(null);
    updateParams({
      postingState: postingStateDraft || null,
      salaryMin: salaryMin?.toString() ?? null,
      salaryMax: salaryMax?.toString() ?? null,
    });
    setFilterPopoverOpen(false);
    filterButtonRef.current?.focus();
  }

  function clearFilters() {
    searchDraftRef.current = "";
    setSearchDraft("");
    setPostingStateDraft("");
    setSalaryMinDraft("");
    setSalaryMaxDraft("");
    setFilterError(null);
    setFilterPopoverOpen(false);
    updateParams({
      search: null,
      companyId: null,
      postingState: null,
      salaryMin: null,
      salaryMax: null,
    });
  }

  function toggleCompanyGrouping() {
    updateParams({ groupBy: filters.groupByCompany ? null : "company" });
  }

  return (
    <section className="card p-5 sm:p-6">
      {roles.length === 0 ? (
        <p className="text-sm text-[var(--muted)]">
          Your watches are up to date. No new matches found.
        </p>
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <input
              type="search"
              value={searchDraft}
              onChange={(event) => setSearchDraft(event.target.value)}
              onFocus={() => setFilterPopoverOpen(false)}
              placeholder="Search title, company, or location…"
              aria-label="Search new roles"
              className="field min-w-[220px] flex-1 border-transparent bg-[var(--surface-muted)]"
            />
            <select
              value={filters.companyId}
              onChange={(event) => updateParams({ companyId: event.target.value || null })}
              aria-label="Filter by company"
              className="field w-full sm:w-auto sm:min-w-44"
            >
              <option value="">All companies</option>
              {companyOptions.map((company) => (
                <option key={company.id} value={company.id}>{company.name}</option>
              ))}
            </select>
            <div className="relative shrink-0">
              <button
                ref={filterButtonRef}
                type="button"
                className={`btn btn-secondary btn-sm ${filterPopoverOpen || activeFilterCount > 0 ? "border-[var(--accent)] text-[var(--accent-ink)]" : ""}`}
                aria-label={activeFilterCount > 0 ? `Filters, ${activeFilterCount} active` : "Filters"}
                aria-haspopup="dialog"
                aria-expanded={filterPopoverOpen}
                aria-controls="new-roles-filter-popover"
                onClick={() => setFilterPopoverOpen((open) => !open)}
              >
                Filters
                {activeFilterCount > 0 ? (
                  <span aria-hidden="true" className="ml-0.5 inline-flex min-w-5 items-center justify-center rounded-full bg-[var(--accent-soft)] px-1.5 py-0.5 text-[0.6875rem] font-semibold leading-none text-[var(--accent-ink)]">
                    {activeFilterCount}
                  </span>
                ) : null}
              </button>
              {filterPopoverOpen ? (
                <div
                  id="new-roles-filter-popover"
                  ref={filterPopoverRef}
                  role="dialog"
                  aria-modal="false"
                  aria-labelledby="new-roles-filter-heading"
                  className="jobs-filter-popover absolute right-0 top-full z-30 mt-2 rounded-xl border border-[var(--border)] bg-[var(--surface)] p-4 shadow-[var(--shadow-md)]"
                >
                  <form onSubmit={handleApplyFilters}>
                    <h2 id="new-roles-filter-heading" className="mb-4 text-sm font-semibold text-[var(--foreground)]">
                      Filters
                    </h2>
                    <label htmlFor="new-roles-posting-state" className="field-label">Posting status</label>
                    <select
                      id="new-roles-posting-state"
                      value={postingStateDraft}
                      onChange={(event) => setPostingStateDraft(event.target.value as typeof postingStateDraft)}
                      className="field text-sm"
                    >
                      <option value="">All postings</option>
                      <option value="active">Active</option>
                      <option value="inactive">Inactive</option>
                    </select>

                    <fieldset className="mt-4">
                      <legend className="field-label">Salary per year (USD)</legend>
                      <div className="grid grid-cols-2 gap-3">
                        <label htmlFor="new-roles-salary-min" className="text-xs font-medium text-[var(--muted)]">
                          Minimum
                          <input
                            ref={salaryMinInputRef}
                            id="new-roles-salary-min"
                            type="number"
                            min="0"
                            step="1000"
                            inputMode="numeric"
                            value={salaryMinDraft}
                            onChange={(event) => setSalaryMinDraft(event.target.value)}
                            placeholder="No minimum"
                            className="field mt-1.5 text-sm"
                          />
                        </label>
                        <label htmlFor="new-roles-salary-max" className="text-xs font-medium text-[var(--muted)]">
                          Maximum
                          <input
                            id="new-roles-salary-max"
                            type="number"
                            min="0"
                            step="1000"
                            inputMode="numeric"
                            value={salaryMaxDraft}
                            onChange={(event) => setSalaryMaxDraft(event.target.value)}
                            placeholder="No maximum"
                            className="field mt-1.5 text-sm"
                          />
                        </label>
                      </div>
                    </fieldset>
                    {filterError ? <p role="alert" className="mt-3 text-xs text-[var(--danger)]">{filterError}</p> : null}
                    <div className="mt-4 flex justify-end gap-2 border-t border-[var(--border)] pt-3">
                      <button type="button" className="btn btn-ghost btn-sm" onClick={clearFilters}>
                        Clear filters
                      </button>
                      <button type="submit" className="btn btn-secondary btn-sm">
                        Apply
                      </button>
                    </div>
                  </form>
                </div>
              ) : null}
            </div>
            <button
              type="button"
              aria-pressed={filters.groupByCompany}
              onClick={toggleCompanyGrouping}
              className={`btn btn-secondary btn-sm ${filters.groupByCompany ? "border-[var(--accent)] text-[var(--accent-ink)]" : ""}`}
            >
              Group by company
            </button>
          </div>

          <div className="mt-3 flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--muted)]" aria-live="polite">
            <span>
              Showing {filteredRoles.length} of {roles.length} {roles.length === 1 ? "role" : "roles"}
            </span>
            {hasActiveFilters ? (
              <button type="button" className="btn btn-ghost btn-sm px-1.5" onClick={clearFilters}>
                Clear filters
              </button>
            ) : null}
          </div>

          <div className="mt-4">
            {filteredRoles.length === 0 ? (
              <div className="py-8 text-center">
                <p className="font-display text-lg text-[var(--foreground)]">No new roles match these filters</p>
                <p className="mt-1 text-sm text-[var(--muted)]">Try a different search or clear your filters.</p>
              </div>
            ) : filters.groupByCompany ? (
              <div className="space-y-6">
                {companyGroups.map((group) => (
                  <section key={group.id} aria-labelledby={`new-roles-company-${group.id}`}>
                    <div className="mb-2 flex items-baseline justify-between gap-3">
                      <h2 id={`new-roles-company-${group.id}`} className="text-sm font-semibold text-[var(--foreground)]">
                        {group.name}
                      </h2>
                      <span className="text-xs text-[var(--muted)]">
                        {group.roles.length} {group.roles.length === 1 ? "role" : "roles"}
                      </span>
                    </div>
                    <NewRolesList
                      roles={group.roles}
                      showCompany={false}
                      onTriage={onTriage}
                      isPending={isPending}
                    />
                  </section>
                ))}
              </div>
            ) : (
              <NewRolesList
                roles={flatRoles}
                showCompany
                onTriage={onTriage}
                isPending={isPending}
              />
            )}
          </div>
        </>
      )}
    </section>
  );
}
