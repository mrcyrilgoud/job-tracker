import { formatDistanceToNow } from "date-fns";
import { useState } from "react";

import { FilterCriteriaEditor } from "@/components/companies/FilterCriteriaEditor";
import { api } from "@/lib/api";
import { providerLabel, syncErrorNote, watchPresentation } from "@/lib/companies-ui";
import {
  isOverride,
  matchAllCriteria,
  resetOverridePayload,
} from "@/lib/filter-criteria-editor";
import type { CompanyWatch, FilterCriteria } from "@/lib/schema";
import { isDesktopShell } from "@/lib/tauri";
import type { Feedback } from "@/lib/use-pending-actions";
import { toneClasses } from "@/lib/ui";

/**
 * One watched board. The status pill leads because that is what the user came to
 * check; the provider and board name are plumbing and are sized accordingly
 * (DESIGN.md: "visible if you look, never shouting").
 */
export function WatchRow({
  watch,
  syncing,
  removing,
  feedback,
  onSync,
  onRemove,
  onFilterChanged,
}: {
  watch: CompanyWatch;
  syncing: boolean;
  removing: boolean;
  feedback?: Feedback;
  onSync: () => void;
  onRemove: () => void;
  /** Called after a per-watch filter override is saved or reset so the card can reload. */
  onFilterChanged?: () => void;
}) {
  const [confirmingRemove, setConfirmingRemove] = useState(false);
  const status = watchPresentation(watch);

  // Per-board filter state. `hasOverride` tracks whether this watch carries its
  // own criteria (vs inheriting the global filter). We initialise from the prop
  // the card already loaded, then lazily fetch on open in case it was stale.
  const [filterOpen, setFilterOpen] = useState(false);
  const [hasOverride, setHasOverride] = useState<boolean>(isOverride(watch.filterCriteria));
  const [editingCriteria, setEditingCriteria] = useState<FilterCriteria | null>(
    watch.filterCriteria ?? null,
  );
  const [filterLoading, setFilterLoading] = useState(false);
  const [filterSaving, setFilterSaving] = useState(false);
  const [filterError, setFilterError] = useState<string | null>(null);

  async function openFilter() {
    setFilterOpen(true);
    setFilterError(null);
    // Lazily refresh the override from the backend; guarded because api.call
    // only works inside the desktop shell.
    if (!isDesktopShell()) {
      if (editingCriteria == null) setEditingCriteria(matchAllCriteria());
      return;
    }
    setFilterLoading(true);
    try {
      const override = await api.getWatchFilterCriteria(watch.id);
      setHasOverride(isOverride(override));
      setEditingCriteria(override ?? matchAllCriteria());
    } catch (error) {
      setFilterError(error instanceof Error ? error.message : "Couldn’t load this board’s filter.");
      if (editingCriteria == null) setEditingCriteria(matchAllCriteria());
    } finally {
      setFilterLoading(false);
    }
  }

  async function saveFilter() {
    if (!editingCriteria) return;
    setFilterSaving(true);
    setFilterError(null);
    try {
      await api.setWatchFilterCriteria(watch.id, editingCriteria);
      setHasOverride(true);
      onFilterChanged?.();
    } catch (error) {
      setFilterError(error instanceof Error ? error.message : "Couldn’t save this board’s filter.");
    } finally {
      setFilterSaving(false);
    }
  }

  async function resetFilter() {
    setFilterSaving(true);
    setFilterError(null);
    try {
      await api.setWatchFilterCriteria(watch.id, resetOverridePayload);
      setHasOverride(false);
      setEditingCriteria(matchAllCriteria());
      onFilterChanged?.();
    } catch (error) {
      setFilterError(error instanceof Error ? error.message : "Couldn’t reset this board’s filter.");
    } finally {
      setFilterSaving(false);
    }
  }

  const checked = watch.lastSyncedAt
    ? `Checked ${formatDistanceToNow(new Date(watch.lastSyncedAt), { addSuffix: true })}`
    : "Not checked yet";

  return (
    <div className="rounded-xl bg-[var(--surface-muted)] px-4 py-3">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className={`pill ${toneClasses[status.tone]}`}>
              <span className="pill-dot" aria-hidden />
              {status.label}
            </span>
            <span className="text-xs text-[var(--muted)]">{checked}</span>
          </div>
          <p className="mt-1 text-xs text-[var(--faint)]">
            {providerLabel(watch.provider)} · {watch.boardSlug}
          </p>
        </div>

        {confirmingRemove ? (
          <div className="flex items-center gap-2">
            <span className="text-sm text-[var(--muted)]">Stop watching this board?</span>
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              disabled={removing}
              onClick={onRemove}
            >
              {removing ? <span className="spinner" aria-hidden /> : null}
              Yes, stop
            </button>
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => setConfirmingRemove(false)}
            >
              Keep
            </button>
          </div>
        ) : (
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              disabled={syncing}
              onClick={onSync}
            >
              {syncing ? <span className="spinner" aria-hidden /> : null}
              {syncing ? "Checking…" : "Check now"}
            </button>
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => setConfirmingRemove(true)}
            >
              Stop watching
            </button>
          </div>
        )}
      </div>

      {watch.lastSyncError ? (
        <div className="mt-2">
          <p className="text-sm text-[var(--danger)]">
            {syncErrorNote(watch.lastSyncError, watch.provider)}
          </p>
          <details className="mt-1">
            <summary className="cursor-pointer text-xs text-[var(--faint)] hover:text-[var(--muted)]">
              Technical details
            </summary>
            <p className="mt-1 font-mono text-xs break-all text-[var(--faint)]">
              {watch.lastSyncError}
            </p>
          </details>
        </div>
      ) : null}

      {feedback ? (
        <p
          className={`mt-2 text-sm ${
            feedback.tone === "positive" ? "text-[var(--green-ink)]" : "text-[var(--danger)]"
          }`}
          role={feedback.tone === "negative" ? "alert" : undefined}
        >
          {feedback.text}
        </p>
      ) : null}

      <div className="mt-2 border-t border-[var(--border)] pt-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <button
            type="button"
            className="btn btn-ghost btn-sm px-0"
            aria-expanded={filterOpen}
            onClick={() => (filterOpen ? setFilterOpen(false) : void openFilter())}
          >
            <span aria-hidden>{filterOpen ? "▴" : "▾"}</span>
            Filter for this board
          </button>
          <span className="text-xs text-[var(--faint)]">
            {hasOverride ? "Custom filter" : "Using global filter"}
          </span>
        </div>

        {filterOpen ? (
          <div className="mt-3 space-y-3">
            {filterLoading ? (
              <p className="flex items-center gap-2 text-sm text-[var(--muted)]">
                <span className="spinner" aria-hidden />
                Loading filter…
              </p>
            ) : editingCriteria ? (
              <>
                {!hasOverride ? (
                  <p className="text-xs text-[var(--muted)]">
                    This board uses the global filter. Adjust the criteria below and save to
                    create an override just for it.
                  </p>
                ) : null}
                <FilterCriteriaEditor value={editingCriteria} onChange={setEditingCriteria} />
                <div className="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    className="btn btn-primary btn-sm"
                    disabled={filterSaving}
                    onClick={() => void saveFilter()}
                  >
                    {filterSaving ? <span className="spinner" aria-hidden /> : null}
                    Save filter
                  </button>
                  {hasOverride ? (
                    <button
                      type="button"
                      className="btn btn-ghost btn-sm"
                      disabled={filterSaving}
                      onClick={() => void resetFilter()}
                    >
                      Reset to global
                    </button>
                  ) : null}
                </div>
              </>
            ) : null}

            {filterError ? (
              <p className="text-sm text-[var(--danger)]" role="alert">
                {filterError}
              </p>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );
}
