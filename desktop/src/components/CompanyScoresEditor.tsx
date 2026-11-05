import { useEffect, useMemo, useState } from "react";

import {
  COMPANY_STABILITY_SCALE_LABEL,
  CompanyStabilitySelect,
} from "@/components/CompanyStabilitySelect";
import {
  EMPLOYMENT_STABILITY_SCALE_LABEL,
  EmploymentStabilitySelect,
} from "@/components/EmploymentStabilitySelect";
import { AlertIcon } from "@/components/icons";
import { api, type CompanyScoreWriteInput } from "@/lib/api";
import type { Company } from "@/lib/schema";

type ScoreKey = "company" | "employment";

type ScoreDraft = {
  score: number | null;
  rationale: string;
  citations: string[];
  citationDraft: string;
  scoredAt: string | null;
  scoredBy: "manual" | "import" | null;
};

type EditorDraft = Record<ScoreKey, ScoreDraft>;

const SCORE_CONFIG: Record<ScoreKey, {
  title: string;
  scoreField: "companyStability" | "employmentStability";
  rationaleField: "companyStabilityRationale" | "employmentStabilityRationale";
  citationsField: "companyStabilityCitations" | "employmentStabilityCitations";
  scaleLabel: string;
}> = {
  company: {
    title: "Company stability",
    scoreField: "companyStability",
    rationaleField: "companyStabilityRationale",
    citationsField: "companyStabilityCitations",
    scaleLabel: COMPANY_STABILITY_SCALE_LABEL,
  },
  employment: {
    title: "Employment stability",
    scoreField: "employmentStability",
    rationaleField: "employmentStabilityRationale",
    citationsField: "employmentStabilityCitations",
    scaleLabel: EMPLOYMENT_STABILITY_SCALE_LABEL,
  },
};

function scoreDraftFromCompany(company: Company): EditorDraft {
  return {
    company: {
      score: company.companyStability,
      rationale: company.companyStabilityRationale ?? "",
      citations: [...(company.companyStabilityCitations ?? [])],
      citationDraft: "",
      scoredAt: company.companyStabilityScoredAt,
      scoredBy: company.companyStabilityScoredBy,
    },
    employment: {
      score: company.employmentStability,
      rationale: company.employmentStabilityRationale ?? "",
      citations: [...(company.employmentStabilityCitations ?? [])],
      citationDraft: "",
      scoredAt: company.employmentStabilityScoredAt,
      scoredBy: company.employmentStabilityScoredBy,
    },
  };
}

function normalizedRationale(value: string): string | null {
  const trimmed = value.trim();
  return trimmed === "" ? null : trimmed;
}

function normalizedCitations(value: string[]): string[] | null {
  const trimmed = value.map((citation) => citation.trim());
  return trimmed.length === 0 ? null : trimmed;
}

function sameCitations(left: string[] | null, right: string[] | null): boolean {
  if (left === right) return true;
  if (!left || !right || left.length !== right.length) return false;
  return left.every((value, index) => value === right[index]);
}

function codePointCount(value: string): number {
  return [...value.trim()].length;
}

function citationError(value: string): boolean {
  const trimmed = value.trim();
  if (trimmed.length === 0) return true;
  try {
    const url = new URL(trimmed);
    return (url.protocol !== "http:" && url.protocol !== "https:") || url.hostname.length === 0;
  } catch {
    return true;
  }
}

function formatScoredAt(value: string | null): string {
  if (!value) return "—";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function draftsMatch(left: EditorDraft, right: EditorDraft): boolean {
  return (Object.keys(SCORE_CONFIG) as ScoreKey[]).every((key) => {
    const a = left[key];
    const b = right[key];
    if (a.score !== b.score) return false;
    if (a.score === null) return true;
    return (
      normalizedRationale(a.rationale) === normalizedRationale(b.rationale) &&
      sameCitations(normalizedCitations(a.citations), normalizedCitations(b.citations))
    );
  });
}

function validationError(draft: ScoreDraft): string | null {
  if (draft.score === null) return null;
  const rationaleLength = codePointCount(draft.rationale);
  if (rationaleLength > 2000) return "Rationale must be 2000 characters or fewer.";
  if (draft.citations.length > 20) return "Citations are limited to 20 entries.";
  const invalidIndex = draft.citations.findIndex(citationError);
  if (invalidIndex >= 0) {
    return `Citation ${invalidIndex + 1} must be an absolute http or https URL.`;
  }
  return null;
}

function changedFields(saved: EditorDraft, draft: EditorDraft): CompanyScoreWriteInput {
  const input: CompanyScoreWriteInput = {};
  (Object.keys(SCORE_CONFIG) as ScoreKey[]).forEach((key) => {
    const config = SCORE_CONFIG[key];
    const previous = saved[key];
    const next = draft[key];
    if (previous.score !== next.score) {
      input[config.scoreField] = next.score;
    }
    // A null score clears all server-side metadata. The score field alone is
    // deliberately sufficient and avoids sending disabled metadata controls.
    if (next.score === null) return;
    if (normalizedRationale(previous.rationale) !== normalizedRationale(next.rationale)) {
      input[config.rationaleField] = normalizedRationale(next.rationale);
    }
    if (!sameCitations(normalizedCitations(previous.citations), normalizedCitations(next.citations))) {
      input[config.citationsField] = normalizedCitations(next.citations);
    }
  });
  return input;
}

function scoreSelect(key: ScoreKey, props: { id: string; value: number | null; onChange: (value: number | null) => void }) {
  return key === "company" ? (
    <CompanyStabilitySelect {...props} />
  ) : (
    <EmploymentStabilitySelect {...props} />
  );
}

export function CompanyScoresEditor({
  company,
  onSaved,
}: {
  company: Company;
  onSaved?: (company: Company) => void;
}) {
  const [saved, setSaved] = useState<EditorDraft>(() => scoreDraftFromCompany(company));
  const [draft, setDraft] = useState<EditorDraft>(() => scoreDraftFromCompany(company));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // A company id change means this editor has been mounted for a different
  // route. Ordinary parent refreshes must not overwrite an in-progress edit.
  useEffect(() => {
    setSaved(scoreDraftFromCompany(company));
    setDraft(scoreDraftFromCompany(company));
    setError(null);
  }, [company.id]);

  const isDirty = !draftsMatch(draft, saved);
  const validationErrors = useMemo(
    () => (Object.keys(SCORE_CONFIG) as ScoreKey[]).map((key) => validationError(draft[key])).filter(Boolean) as string[],
    [draft],
  );
  const canSave = isDirty && !saving && validationErrors.length === 0;

  useEffect(() => {
    if (!isDirty) return;
    function warn(event: BeforeUnloadEvent) {
      event.preventDefault();
    }
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [isDirty]);

  function updateScore(key: ScoreKey, score: number | null) {
    setDraft((current) => ({ ...current, [key]: { ...current[key], score } }));
    setError(null);
  }

  function updateRationale(key: ScoreKey, rationale: string) {
    setDraft((current) => ({ ...current, [key]: { ...current[key], rationale } }));
    setError(null);
  }

  function updateCitationDraft(key: ScoreKey, citationDraft: string) {
    setDraft((current) => ({ ...current, [key]: { ...current[key], citationDraft } }));
    setError(null);
  }

  function addCitation(key: ScoreKey) {
    setDraft((current) => ({
      ...current,
      [key]: {
        ...current[key],
        citations: [...current[key].citations, current[key].citationDraft],
        citationDraft: "",
      },
    }));
    setError(null);
  }

  function removeCitation(key: ScoreKey, index: number) {
    setDraft((current) => ({
      ...current,
      [key]: {
        ...current[key],
        citations: current[key].citations.filter((_, citationIndex) => citationIndex !== index),
      },
    }));
    setError(null);
  }

  async function save() {
    if (!canSave) return;
    setSaving(true);
    setError(null);
    try {
      const result = await api.writeCompanyScores(company.id, changedFields(saved, draft));
      const nextSaved = scoreDraftFromCompany(result.company);
      setSaved(nextSaved);
      setDraft(nextSaved);
      onSaved?.(result.company);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save company scores");
    } finally {
      setSaving(false);
    }
  }

  return (
    <form
      className="card space-y-5 p-6"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="font-display text-xl font-medium">Company scores</h2>
          <p className="mt-1 text-sm text-[var(--muted)]">
            Score the employer once. Both scores apply to every job at {company.name}.
          </p>
        </div>
        {isDirty ? <span className="text-sm text-[var(--amber-ink)]">Unsaved changes</span> : null}
      </div>

      {(Object.keys(SCORE_CONFIG) as ScoreKey[]).map((key) => {
        const config = SCORE_CONFIG[key];
        const values = draft[key];
        const storedScore = saved[key].score;
        const metadataDisabled = values.score === null;
        const rationaleLength = codePointCount(values.rationale);
        const hasClearNotice = storedScore !== null && values.score === null;
        const scoreError = validationError(values);
        const scoredAt = values.scoredAt;
        const scoredBy = values.scoredBy;

        return (
          <section key={key} aria-labelledby={`${key}-score-heading`} className="space-y-4 rounded-xl border border-[var(--border)] p-4">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div>
                <h3 id={`${key}-score-heading`} className="font-medium">{config.title}</h3>
                <p className="mt-1 text-xs text-[var(--muted)]">{config.scaleLabel}</p>
              </div>
              <div className="min-w-48">
                <label htmlFor={`${key}-score`} className="sr-only">{config.scaleLabel}</label>
                {scoreSelect(key, {
                  id: `${key}-score`,
                  value: values.score,
                  onChange: (next) => updateScore(key, next),
                })}
              </div>
            </div>

            {hasClearNotice ? (
              <p className="rounded-lg bg-[var(--amber-soft)] px-3 py-2 text-sm text-[var(--amber-ink)]">
                Saving clears this score&apos;s Rationale and Citation_List.
              </p>
            ) : null}

            <div className="grid gap-4 sm:grid-cols-2">
              <div className="sm:col-span-2">
                <label htmlFor={`${key}-rationale`} className="field-label">Rationale</label>
                <textarea
                  id={`${key}-rationale`}
                  value={values.rationale}
                  disabled={metadataDisabled}
                  onChange={(event) => updateRationale(key, event.target.value)}
                  rows={4}
                  placeholder="Why this score?"
                  className="field"
                />
                <p className={`mt-1.5 text-xs ${rationaleLength > 2000 ? "text-[var(--danger)]" : "text-[var(--faint)]"}`}>
                  {rationaleLength} / 2000 characters
                </p>
                {rationaleLength > 2000 ? (
                  <p className="mt-1 text-sm text-[var(--danger)]" role="alert">
                    Rationale must be 2000 characters or fewer.
                  </p>
                ) : null}
              </div>

              <div className="sm:col-span-2">
                <span className="field-label">Citation_List</span>
                <div className="flex gap-2">
                  <input
                    aria-label={`${config.title} citation URL`}
                    value={values.citationDraft}
                    disabled={metadataDisabled}
                    onChange={(event) => updateCitationDraft(key, event.target.value)}
                    placeholder="Add citation URL"
                    className="field"
                  />
                  <button
                    type="button"
                    className="btn btn-secondary shrink-0"
                    disabled={metadataDisabled || values.citationDraft.length === 0}
                    onClick={() => addCitation(key)}
                  >
                    Add citation
                  </button>
                </div>
                {values.citations.length > 0 ? (
                  <ul className="mt-3 space-y-2">
                    {values.citations.map((citation, index) => {
                      const invalid = citationError(citation);
                      return (
                        <li key={`${index}-${citation}`} className="flex items-start gap-2">
                          <span className="min-w-0 flex-1 break-all rounded-lg bg-[var(--surface-muted)] px-3 py-2 text-sm">
                            {citation}
                            {invalid ? <span className="ml-2 text-[var(--danger)]">Invalid URL</span> : null}
                          </span>
                          <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            disabled={metadataDisabled}
                            onClick={() => removeCitation(key, index)}
                          >
                            Remove
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                ) : null}
                {values.citations.length > 20 ? (
                  <p className="mt-1.5 text-sm text-[var(--danger)]" role="alert">Citations are limited to 20 entries.</p>
                ) : null}
                {scoreError && values.citations.some(citationError) ? (
                  <p className="mt-1.5 text-sm text-[var(--danger)]" role="alert">
                    {scoreError}
                  </p>
                ) : null}
              </div>
            </div>

            <dl className="grid gap-2 text-xs text-[var(--muted)] sm:grid-cols-2">
              <div>
                <dt className="font-medium text-[var(--foreground)]">Scored at</dt>
                <dd>{formatScoredAt(scoredAt)}</dd>
              </div>
              <div>
                <dt className="font-medium text-[var(--foreground)]">Source</dt>
                <dd>{scoredBy ?? "—"}</dd>
              </div>
            </dl>
          </section>
        );
      })}

      {validationErrors.length > 0 ? (
        <div className="space-y-1 text-sm text-[var(--danger)]" role="alert">
          {validationErrors.map((message, index) => <p key={`${message}-${index}`}>{message}</p>)}
        </div>
      ) : null}
      {error ? (
        <p className="flex items-center gap-2 rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]" role="alert">
          <AlertIcon size={15} />
          <span>{error}</span>
        </p>
      ) : null}

      <div className="flex items-center justify-end gap-3 border-t border-[var(--border)] pt-4">
        <button type="submit" className="btn btn-primary" disabled={!canSave} aria-label="Save company scores">
          {saving ? "Saving…" : "Save"}
        </button>
      </div>
    </form>
  );
}
