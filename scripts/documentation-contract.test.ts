import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const agents = fs.readFileSync(path.join(repoRoot, "AGENTS.md"), "utf8");
const readme = fs.readFileSync(path.join(repoRoot, "README.md"), "utf8");
const importer = fs.readFileSync(
  path.join(repoRoot, "src-tauri/src/cli/importer.rs"),
  "utf8",
);
const csv = fs.readFileSync(path.join(repoRoot, "src-tauri/src/jobs/csv.rs"), "utf8");

function requireText(document: string, text: string, contract: string): void {
  assert.ok(document.includes(text), `${contract} is missing: ${text}`);
}

function rustStringArray(source: string, declaration: RegExp, contract: string): string[] {
  const body = source.match(declaration)?.[1];
  assert.ok(body, `Could not find ${contract} in its implementation`);
  return [...body.matchAll(/"([^"]+)"/g)].map((match) => match[1]);
}

test("AGENTS documents the company and employment score flags and scales", () => {
  for (const flag of [
    "--company-stability",
    "--clear-company-stability",
    "--min-company-stability",
    "--employment-stability",
    "--clear-employment-stability",
    "--min-employment-stability",
    "--company-stability-rationale",
    "--clear-company-stability-rationale",
    "--employment-stability-rationale",
    "--clear-employment-stability-rationale",
  ]) {
    requireText(agents, flag, "AGENTS CLI flag contract");
  }

  requireText(agents, "`1` is least stable and `5` is most stable", "company scale");
  requireText(agents, "`1` is least secure and `5` is most secure", "employment scale");
  requireText(agents, "only integers 1 through 5 are accepted", "score range contract");
  requireText(agents, "The value applies to every job at that company", "company scope");
  requireText(agents, "unscored companies are excluded", "minimum-filter behavior");
  requireText(agents, "returns only jobs meeting both minimums", "combined minimum-filter behavior");
  requireText(agents, "at most 2,000 characters", "rationale limit");
  requireText(agents, "The CLI has no citation-editing flag", "citation-editing contract");
});

test("AGENTS documents the score-file mappings and summary/exit contract", () => {
  requireText(agents, "jt scores import <path>", "score importer command");
  requireText(agents, "Without `--yes` it writes nothing and exits 0", "dry-run exit contract");
  requireText(agents, "Only JSON input is accepted", "score-file input contract");
  requireText(
    agents,
    "`score` → company stability, `rationale` → company stability rationale, `citations` → company stability citations, `employment_stability` → employment stability, `employment_rationale` → employment stability rationale, and `employment_citations` → employment stability citations",
    "score-file mapping contract",
  );
  requireText(agents, "An omitted key preserves the stored value; JSON `null` clears it", "tri-state importer contract");
  requireText(agents, "Invalid entries are skipped while remaining entries continue", "invalid-entry continuation contract");
  requireText(agents, "unmatched companies are reported and never create a company or job", "unmatched-entry contract");
  requireText(agents, "failed writes or CSV mirror refresh failures exit nonzero", "failure exit contract");
  requireText(agents, "unmatched, conflict, invalid, and warning groups exit successfully", "non-fatal importer exit contract");

  for (const key of [
    "dryRun",
    "applied",
    "unchanged",
    "unmatched",
    "conflicts",
    "invalid",
    "warnings",
    "failed",
  ]) {
    requireText(agents, `\`${key}\``, "importer JSON summary key");
  }

  assert.deepEqual(
    rustStringArray(
      importer,
      /const SUPPORTED_KEYS: \[&str; 6\] = \[(.*?)\];/s,
      "score-file supported mappings",
    ),
    [
      "score",
      "rationale",
      "citations",
      "employment_stability",
      "employment_rationale",
      "employment_citations",
    ],
    "documented mappings must match the importer mapping list",
  );
});

test("AGENTS documents the complete JSON score shape", () => {
  for (const key of [
    "companyStability",
    "employmentStability",
    "companyStabilityRationale",
    "companyStabilityCitations",
    "companyStabilityScoredAt",
    "companyStabilityScoredBy",
    "employmentStabilityRationale",
    "employmentStabilityCitations",
    "employmentStabilityScoredAt",
    "employmentStabilityScoredBy",
  ]) {
    requireText(agents, `\`${key}\``, "JSON score field contract");
  }
  requireText(agents, "citations are arrays of URLs or `null`", "JSON citation shape");
  requireText(agents, "source is `manual`, `import`, or `null`", "JSON source shape");
  requireText(agents, "does not favorite jobs or alter the per-job `Appeal` or `Application_Ease` scores", "score-scope contract");
});

test("README documents the ordered CSV score columns and import rules", () => {
  const editableLine = readme.match(/^- Editable columns: (.+)$/m)?.[1];
  assert.ok(editableLine, "README is missing its Jobs CSV editable-columns line");
  assert.deepEqual(
    [...editableLine.matchAll(/`([^`]+)`/g)].map((match) => match[1]),
    [
      "url",
      "title",
      "company",
      "status",
      "applied_at",
      "notes",
      "location",
      "latest_note",
      "appeal",
      "application_ease",
      "company_stability",
      "employment_stability",
    ],
    "README CSV editable columns/order",
  );

  for (const text of [
    "company-wide score from 1 to 5 (**5 = most stable**, 1 = least stable)",
    "employment_stability` is a company-wide score from 1 to 5 (**5 = most secure**, 1 = least secure)",
    "repeats each company's stored score on every job row for that company",
    "Leave either cell blank when that score is unscored",
    "a CSV that omits the column",
    "Company score metadata, including rationales, citations, scored-at timestamps, and scored-by sources, is not exported as CSV columns",
    "a score cell is changed only when it differs from that baseline",
    "Blank score cells clear a stored score only when the import treats that cell as changed",
    "disagreements create a company-level conflict",
    "the stored value wins",
    "Invalid score cells must be an integer from 1 to 5 or blank",
  ]) {
    requireText(readme, text, "README CSV contract");
  }

  assert.deepEqual(
    rustStringArray(
      csv,
      /pub const CSV_HEADERS: &\[&str\] = &\[(.*?)\];/s,
      "CSV header",
    ).slice(-2),
    ["company_stability", "employment_stability"],
    "implementation CSV score columns/order",
  );
});

test("AGENTS.md and README.md contain no stale per-job company-stability wording", () => {
  for (const [name, document] of [
    ["AGENTS.md", agents],
    ["README.md", readme],
  ] as const) {
    for (const sentence of document.split(/(?<=[.!?])\s+|\n+/)) {
      if (!/company[_ ]stability/i.test(sentence)) continue;
      assert.doesNotMatch(
        sentence,
        /per[- ]job|job[- ]specific|job[- ]level|for (?:each|a|the) job's? score/i,
        `${name} still describes company stability as per-job: ${sentence.trim()}`,
      );
    }
  }
});
