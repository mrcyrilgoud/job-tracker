import { useEffect, useMemo, useRef, useState } from "react";

import {
  api,
  type FilterPreviewOutcome,
  type FilterPreviewSample,
} from "@/lib/api";
import {
  addChip,
  commitCommaSeparated,
  previewLabel,
  removeChipAt,
} from "@/lib/filter-criteria-editor";
import type {
  FilterCriteria,
  LocationCriteria,
  MatchMode,
  RemoteMode,
  TitleCriteria,
} from "@/lib/schema";
import { isDesktopShell } from "@/lib/tauri";

export type FilterCriteriaEditorProps = {
  /** The criteria being edited. This is a controlled component. */
  value: FilterCriteria;
  /** Called with a new, immutably-updated criteria whenever the user edits. */
  onChange: (next: FilterCriteria) => void;
  /**
   * Sample roles used to drive the live preview. When omitted, a small
   * built-in default sample set is used so the preview is never empty.
   */
  previewSamples?: FilterPreviewSample[];
};

/** Common country options; whatever the user types is also preserved. */
const COUNTRY_OPTIONS = ["", "United States", "United Kingdom", "Canada"] as const;

const REMOTE_OPTIONS: ReadonlyArray<{ value: RemoteMode; label: string }> = [
  { value: "any", label: "Any" },
  { value: "remoteOnly", label: "Remote only" },
  { value: "onsiteOnly", label: "Onsite only" },
];

const DEFAULT_PREVIEW_SAMPLES: FilterPreviewSample[] = [
  { title: "Senior Software Engineer", location: "San Francisco, CA" },
  { title: "Contract QA", location: "Remote - US" },
  { title: "Product Manager", location: "London, UK" },
];

/** Delay before firing a preview request as the user types, in milliseconds. */
const PREVIEW_DEBOUNCE_MS = 250;

/**
 * A controlled, presentational editor for a single {@link FilterCriteria}
 * value. It renders title include/exclude chip inputs, a country selector,
 * location include/exclude chip inputs, and a remote-mode selector, plus a live
 * preview (Requirement 13.5) that shows which sample roles pass under the
 * current criteria.
 *
 * Persistence (global vs per-watch) is intentionally left to the callers
 * (SettingsPage / WatchRow); this component only edits a value in memory.
 */
export function FilterCriteriaEditor({
  value,
  onChange,
  previewSamples,
}: FilterCriteriaEditorProps) {
  const samples = useMemo(
    () => (previewSamples && previewSamples.length > 0 ? previewSamples : DEFAULT_PREVIEW_SAMPLES),
    [previewSamples],
  );

  const setTitle = (next: Partial<TitleCriteria>) =>
    onChange({ ...value, title: { ...value.title, ...next } });

  const setLocation = (next: Partial<LocationCriteria>) =>
    onChange({ ...value, location: { ...value.location, ...next } });

  const setRemote = (remote: RemoteMode) => onChange({ ...value, remote });

  return (
    <div className="space-y-4" data-testid="filter-criteria-editor">
      <fieldset className="space-y-3">
        <legend className="field-label">Title</legend>
        <ChipInput
          label="Include titles"
          testId="title-include"
          placeholder="e.g. engineer, product manager"
          tokens={value.title.include}
          onChange={(include) => setTitle({ include })}
        />
        <ChipInput
          label="Exclude titles"
          testId="title-exclude"
          placeholder="e.g. senior, contract"
          tokens={value.title.exclude}
          onChange={(exclude) => setTitle({ exclude })}
        />
        <MatchModeToggle
          label="Title matching"
          testId="title-match-mode"
          value={value.title.matchMode}
          onChange={(matchMode) => setTitle({ matchMode })}
        />
      </fieldset>

      <fieldset className="space-y-3">
        <legend className="field-label">Location</legend>
        <label className="block space-y-1.5 text-sm">
          <span className="font-medium">Country</span>
          <select
            className="field"
            data-testid="location-country"
            aria-label="Country"
            value={value.location.country ?? ""}
            onChange={(event) => {
              const raw = event.target.value.trim();
              setLocation({ country: raw.length > 0 ? raw : null });
            }}
          >
            {COUNTRY_OPTIONS.map((option) => (
              <option key={option || "any"} value={option}>
                {option === "" ? "Any country" : option}
              </option>
            ))}
            {/* Preserve a typed-in country that is not one of the presets. */}
            {value.location.country &&
            !COUNTRY_OPTIONS.includes(value.location.country as (typeof COUNTRY_OPTIONS)[number]) ? (
              <option value={value.location.country}>{value.location.country}</option>
            ) : null}
          </select>
        </label>
        <ChipInput
          label="Include locations"
          testId="location-include"
          placeholder="e.g. bay area, remote, new york"
          tokens={value.location.include}
          onChange={(include) => setLocation({ include })}
        />
        <ChipInput
          label="Exclude locations"
          testId="location-exclude"
          placeholder="e.g. onsite, india"
          tokens={value.location.exclude}
          onChange={(exclude) => setLocation({ exclude })}
        />
        <MatchModeToggle
          label="Location matching"
          testId="location-match-mode"
          value={value.location.matchMode}
          onChange={(matchMode) => setLocation({ matchMode })}
        />
      </fieldset>

      <fieldset className="space-y-1.5">
        <legend className="field-label">Remote</legend>
        <div className="flex flex-wrap gap-2" data-testid="remote-mode" role="radiogroup" aria-label="Remote mode">
          {REMOTE_OPTIONS.map((option) => {
            const active = value.remote === option.value;
            return (
              <button
                key={option.value}
                type="button"
                role="radio"
                aria-checked={active}
                className={`btn btn-sm ${active ? "btn-primary" : "btn-secondary"}`}
                onClick={() => setRemote(option.value)}
              >
                {option.label}
              </button>
            );
          })}
        </div>
      </fieldset>

      <LivePreview criteria={value} samples={samples} />
    </div>
  );
}

type ChipInputProps = {
  label: string;
  testId: string;
  placeholder?: string;
  tokens: string[];
  onChange: (tokens: string[]) => void;
};

/**
 * A text field that turns each entered token into a removable chip. Pressing
 * Enter or comma commits the current draft; whitespace-only tokens are dropped
 * (mirrors backend Req 17.3).
 */
function ChipInput({ label, testId, placeholder, tokens, onChange }: ChipInputProps) {
  const [draft, setDraft] = useState("");

  function commit(raw: string) {
    const next = addChip(tokens, raw);
    // addChip returns the same list unchanged for empty/whitespace-only input.
    if (next !== tokens) onChange(next);
    setDraft("");
  }

  function removeAt(index: number) {
    onChange(removeChipAt(tokens, index));
  }

  return (
    <label className="block space-y-1.5 text-sm">
      <span className="font-medium">{label}</span>
      <div
        className="flex flex-wrap items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--surface)] p-1.5 focus-within:border-[var(--accent)]"
        data-testid={`${testId}-chips`}
      >
        {tokens.map((token, index) => (
          <span
            key={`${token}-${index}`}
            className="inline-flex items-center gap-1 rounded-lg bg-[var(--surface-muted)] px-2 py-0.5 text-xs"
            data-testid={`${testId}-chip`}
          >
            {token}
            <button
              type="button"
              className="text-[var(--faint)] hover:text-[var(--danger)]"
              aria-label={`Remove ${token}`}
              onClick={() => removeAt(index)}
            >
              ×
            </button>
          </span>
        ))}
        <input
          className="min-w-[8rem] flex-1 border-none bg-transparent px-1 py-0.5 text-sm outline-none placeholder:text-[var(--faint)]"
          data-testid={`${testId}-input`}
          aria-label={label}
          placeholder={tokens.length === 0 ? placeholder : undefined}
          value={draft}
          onChange={(event) => {
            const raw = event.target.value;
            // Comma acts as a delimiter that commits the tokens before it.
            if (raw.includes(",")) {
              const result = commitCommaSeparated(tokens, raw);
              if (result.tokens !== tokens) onChange(result.tokens);
              setDraft(result.draft);
              return;
            }
            setDraft(raw);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              commit(draft);
            } else if (event.key === "Backspace" && draft.length === 0 && tokens.length > 0) {
              removeAt(tokens.length - 1);
            }
          }}
          onBlur={() => commit(draft)}
        />
      </div>
    </label>
  );
}

type MatchModeToggleProps = {
  label: string;
  testId: string;
  value: MatchMode;
  onChange: (mode: MatchMode) => void;
};

/** A compact Word / Substring toggle that keeps the existing matchMode intact. */
function MatchModeToggle({ label, testId, value, onChange }: MatchModeToggleProps) {
  const options: ReadonlyArray<{ value: MatchMode; label: string }> = [
    { value: "word", label: "Whole word" },
    { value: "substring", label: "Substring" },
  ];
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-xs text-[var(--muted)]">{label}</span>
      <div className="flex gap-1.5" data-testid={testId} role="radiogroup" aria-label={label}>
        {options.map((option) => {
          const active = value === option.value;
          return (
            <button
              key={option.value}
              type="button"
              role="radio"
              aria-checked={active}
              className={`btn btn-sm ${active ? "btn-primary" : "btn-secondary"}`}
              onClick={() => onChange(option.value)}
            >
              {option.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

type LivePreviewProps = {
  criteria: FilterCriteria;
  samples: FilterPreviewSample[];
};

/**
 * Runs {@link api.previewFilterMatch} whenever the criteria or samples change
 * (debounced) and renders each sample with its inclusion result and reason.
 * Preview calls are guarded behind {@link isDesktopShell} because `api.call`
 * throws outside the native shell; errors are swallowed so the editor stays
 * usable (e.g. in a plain browser preview).
 */
function LivePreview({ criteria, samples }: LivePreviewProps) {
  const [outcomes, setOutcomes] = useState<FilterPreviewOutcome[] | null>(null);
  const [failed, setFailed] = useState(false);
  const requestId = useRef(0);

  useEffect(() => {
    if (!isDesktopShell()) {
      setOutcomes(null);
      setFailed(false);
      return;
    }

    const id = ++requestId.current;
    let cancelled = false;
    const timer = setTimeout(() => {
      void api
        .previewFilterMatch(criteria, samples)
        .then((result) => {
          if (cancelled || id !== requestId.current) return;
          setOutcomes(result);
          setFailed(false);
        })
        .catch(() => {
          if (cancelled || id !== requestId.current) return;
          setOutcomes(null);
          setFailed(true);
        });
    }, PREVIEW_DEBOUNCE_MS);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [criteria, samples]);

  return (
    <div className="space-y-1.5" data-testid="filter-preview">
      <span className="field-label">Preview</span>
      {failed ? (
        <p className="text-xs text-[var(--faint)]">Preview unavailable.</p>
      ) : outcomes === null ? (
        <p className="text-xs text-[var(--faint)]">Live preview runs in the desktop app.</p>
      ) : (
        <ul className="space-y-1">
          {samples.map((sample, index) => {
            const outcome = outcomes[index];
            const included = outcome?.included ?? false;
            const label = previewLabel(outcome);
            return (
              <li
                key={`${sample.title}-${index}`}
                className="flex items-start justify-between gap-3 rounded-lg bg-[var(--surface-muted)] px-3 py-2 text-xs"
                data-testid="filter-preview-row"
              >
                <span className="min-w-0">
                  <span className="font-medium">{sample.title}</span>
                  {sample.location ? (
                    <span className="text-[var(--muted)]"> · {sample.location}</span>
                  ) : null}
                  {outcome?.reason ? (
                    <span className="block text-[var(--faint)]">{outcome.reason}</span>
                  ) : null}
                </span>
                <span
                  className={included ? "text-[var(--green-ink)]" : "text-[var(--danger)]"}
                  data-testid={included ? "preview-included" : "preview-excluded"}
                >
                  {label}
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
