// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

const apiMocks = vi.hoisted(() => ({
  writeCompanyScores: vi.fn(),
}));

vi.mock("@/lib/api", () => ({ api: apiMocks }));

import { CompanyScoresEditor } from "./CompanyScoresEditor";
import type { Company } from "@/lib/schema";

const baseCompany: Company = {
  id: "company-1",
  name: "Acme",
  careersUrl: null,
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z",
  companyStability: 4,
  companyStabilityRationale: "Durable revenue",
  companyStabilityCitations: ["https://example.com/report"],
  companyStabilityScoredAt: "2026-01-02T10:20:30.123Z",
  companyStabilityScoredBy: "manual",
  employmentStability: null,
  employmentStabilityRationale: null,
  employmentStabilityCitations: null,
  employmentStabilityScoredAt: null,
  employmentStabilityScoredBy: null,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  apiMocks.writeCompanyScores.mockReset();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

function render(company: Company = baseCompany, onSaved = vi.fn()) {
  act(() => root.render(<CompanyScoresEditor company={company} onSaved={onSaved} />));
}

function select(id: string) {
  return container.querySelector<HTMLSelectElement>(`#${id}`)!;
}

function choose(id: string, value: string) {
  act(() => {
    const input = select(id);
    input.value = value;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

function saveButton() {
  return container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
}

function companyRationale() {
  return container.querySelector<HTMLTextAreaElement>("#company-rationale")!;
}

function companyCitationInput() {
  return container.querySelector<HTMLInputElement>('input[aria-label="Company stability citation URL"]')!;
}

function setInputValue(input: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(input), "value")?.set;
  setter?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

function companySection() {
  return container.querySelector<HTMLElement>('section[aria-labelledby="company-score-heading"]')!;
}

it("renders both score sections and disables metadata controls when unscored", () => {
  render();
  expect(container.querySelector("#company-score")).not.toBeNull();
  expect(container.querySelector("#employment-score")).not.toBeNull();
  expect(select("company-score").value).toBe("4");
  expect(select("employment-score").value).toBe("");
  expect(container.querySelector("#employment-rationale")).toHaveProperty("disabled", true);
  expect(container.querySelector('input[aria-label="Employment stability citation URL"]')).toHaveProperty("disabled", true);
  expect(container.textContent).toContain("2026");
  expect(container.textContent).toContain("manual");
});

it("validates Unicode rationale length and citation URLs locally", () => {
  render();
  const rationale = companyRationale();
  act(() => {
    setInputValue(rationale, "🙂".repeat(2001));
  });
  expect(container.textContent).toContain("2001 / 2000 characters");
  expect(container.textContent).toContain("2000 characters or fewer");
  expect(saveButton().disabled).toBe(true);

  act(() => {
    setInputValue(rationale, "Valid rationale");
    setInputValue(companyCitationInput(), "ftp://example.com/report");
  });
  const add = [...companySection().querySelectorAll("button")].find((button) => button.textContent === "Add citation")!;
  act(() => add.click());
  expect(container.textContent).toContain("absolute http or https URL");
  expect(saveButton().disabled).toBe(true);
});

it("shows the citation entry limit before saving", () => {
  render({
    ...baseCompany,
    companyStabilityCitations: Array.from({ length: 21 }, (_, index) => `https://example.com/${index}`),
  });
  expect(container.textContent).toContain("Citations are limited to 20 entries");
  expect(saveButton().disabled).toBe(true);
});

it("sends only changed score fields and sends no metadata when clearing", async () => {
  const returned = { ...baseCompany, companyStability: null, companyStabilityRationale: null, companyStabilityCitations: null, companyStabilityScoredAt: null, companyStabilityScoredBy: null };
  apiMocks.writeCompanyScores.mockResolvedValue({ company: returned, changed: true, warnings: [] });
  render();
  choose("company-score", "");
  await act(async () => saveButton().click());
  expect(apiMocks.writeCompanyScores).toHaveBeenCalledExactlyOnceWith("company-1", { companyStability: null });
  expect(container.textContent).toContain("Company stability");
  expect(select("company-score").value).toBe("");
});

it("sends metadata-only edits and replaces returned metadata", async () => {
  const returned = {
    ...baseCompany,
    companyStabilityRationale: "Updated rationale",
    companyStabilityCitations: ["https://example.com/new"],
    companyStabilityScoredAt: "2026-01-03T11:22:33.456Z",
    companyStabilityScoredBy: "import" as const,
  };
  apiMocks.writeCompanyScores.mockResolvedValue({ company: returned, changed: true, warnings: [] });
  render();
  act(() => {
    setInputValue(companyRationale(), "Updated rationale");
    setInputValue(companyCitationInput(), "https://example.com/new");
  });
  const add = [...companySection().querySelectorAll("button")].find((button) => button.textContent === "Add citation")!;
  act(() => add.click());
  await act(async () => saveButton().click());
  expect(apiMocks.writeCompanyScores.mock.calls[0][1]).toEqual({
    companyStabilityRationale: "Updated rationale",
    companyStabilityCitations: ["https://example.com/report", "https://example.com/new"],
  });
  expect(container.textContent).toContain("Updated rationale");
  expect(container.textContent).toContain("import");
});

it("shows the clear notice and retains edits in both sections after a failed save", async () => {
  apiMocks.writeCompanyScores.mockRejectedValue(new Error("Network unavailable"));
  render();
  choose("company-score", "");
  choose("employment-score", "4");
  expect(container.textContent).toContain("Saving clears this score's Rationale and Citation_List");
  await act(async () => saveButton().click());
  expect(select("company-score").value).toBe("");
  expect(select("employment-score").value).toBe("4");
  expect(saveButton().disabled).toBe(false);
  expect(container.textContent).toContain("Unsaved changes");
  expect(container.textContent).toContain("Network unavailable");
  expect(container.textContent).toContain("manual");
});

it("installs the existing beforeunload warning only while dirty", () => {
  render();
  const cleanEvent = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(cleanEvent);
  expect(cleanEvent.defaultPrevented).toBe(false);
  choose("company-score", "5");
  const dirtyEvent = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(dirtyEvent);
  expect(dirtyEvent.defaultPrevented).toBe(true);
});

it("accepts the exact rationale and citation limits", async () => {
  const rationale = "🙂".repeat(2000);
  const citations = ["https://example.com/report", ...Array.from({ length: 19 }, (_, index) => `https://example.com/source-${index}`)];
  apiMocks.writeCompanyScores.mockResolvedValue({
    company: {
      ...baseCompany,
      companyStabilityRationale: rationale,
      companyStabilityCitations: citations,
    },
    changed: true,
    warnings: [],
  });
  render();

  act(() => setInputValue(companyRationale(), rationale));
  for (let index = 1; index < citations.length; index += 1) {
    act(() => {
      setInputValue(companyCitationInput(), citations[index]);
      const add = [...companySection().querySelectorAll("button")].find((button) => button.textContent === "Add citation")!;
      add.click();
    });
  }

  expect(container.textContent).toContain("2000 / 2000 characters");
  expect(container.textContent).not.toContain("Citations are limited to 20 entries");
  expect(saveButton().disabled).toBe(false);
  await act(async () => saveButton().click());
  expect(apiMocks.writeCompanyScores).toHaveBeenCalledExactlyOnceWith("company-1", {
    companyStabilityRationale: rationale,
    companyStabilityCitations: citations,
  });
});

it("uses omitted, null, and value fields for changed-only score payloads", async () => {
  const firstReturned = { ...baseCompany, employmentStability: 5 };
  const secondReturned = {
    ...firstReturned,
    companyStability: null,
    companyStabilityRationale: null,
    companyStabilityCitations: null,
    companyStabilityScoredAt: null,
    companyStabilityScoredBy: null,
  };
  apiMocks.writeCompanyScores
    .mockResolvedValueOnce({ company: firstReturned, changed: true, warnings: [] })
    .mockResolvedValueOnce({ company: secondReturned, changed: true, warnings: [] });
  render();

  choose("employment-score", "5");
  await act(async () => saveButton().click());
  expect(apiMocks.writeCompanyScores).toHaveBeenNthCalledWith(1, "company-1", { employmentStability: 5 });
  expect(apiMocks.writeCompanyScores.mock.calls[0][1]).not.toHaveProperty("companyStability");

  choose("company-score", "");
  await act(async () => saveButton().click());
  expect(apiMocks.writeCompanyScores).toHaveBeenNthCalledWith(2, "company-1", { companyStability: null });
  expect(apiMocks.writeCompanyScores.mock.calls[1][1]).not.toHaveProperty("employmentStability");
});

it("shows clear notices for both score sections and disables all unscored metadata controls", () => {
  render({
    ...baseCompany,
    employmentStability: 3,
    employmentStabilityRationale: "Stable team",
    employmentStabilityCitations: ["https://example.com/employment"],
    employmentStabilityScoredAt: "2026-01-02T10:20:30.123Z",
    employmentStabilityScoredBy: "manual",
  });
  choose("company-score", "");
  choose("employment-score", "");
  const notices = [...container.querySelectorAll("p")].filter((paragraph) =>
    paragraph.textContent?.includes("Saving clears this score's Rationale and Citation_List"),
  );
  expect(notices).toHaveLength(2);
  expect(container.querySelector("#employment-rationale")).toHaveProperty("disabled", true);
  expect(container.querySelector('input[aria-label="Employment stability citation URL"]')).toHaveProperty("disabled", true);
  const addButtons = [...container.querySelectorAll("button")].filter((button) => button.textContent === "Add citation");
  expect(addButtons[1]).toHaveProperty("disabled", true);
});

it("applies the same rationale and citation validation to employment stability", () => {
  render({
    ...baseCompany,
    employmentStability: 3,
    employmentStabilityRationale: "Current hiring is steady",
    employmentStabilityCitations: ["https://example.com/employment"],
    employmentStabilityScoredAt: "2026-01-02T10:20:30.123Z",
    employmentStabilityScoredBy: "manual",
  });
  const rationale = container.querySelector<HTMLTextAreaElement>("#employment-rationale")!;
  const citation = container.querySelector<HTMLInputElement>('input[aria-label="Employment stability citation URL"]')!;
  act(() => setInputValue(rationale, "🙂".repeat(2001)));
  expect(container.textContent).toContain("Rationale must be 2000 characters or fewer");
  expect(saveButton().disabled).toBe(true);

  act(() => {
    setInputValue(rationale, "Valid employment rationale");
    setInputValue(citation, "mailto:invalid@example.com");
  });
  const section = container.querySelector<HTMLElement>('section[aria-labelledby="employment-score-heading"]')!;
  const add = [...section.querySelectorAll("button")].find((button) => button.textContent === "Add citation")!;
  act(() => add.click());
  expect(container.textContent).toContain("Citation 2 must be an absolute http or https URL");
  expect(saveButton().disabled).toBe(true);
});

it("sends changed score fields for both scales without sending unchanged metadata", async () => {
  apiMocks.writeCompanyScores.mockResolvedValue({ company: baseCompany, changed: true, warnings: [] });
  render();
  choose("company-score", "5");
  choose("employment-score", "3");
  await act(async () => saveButton().click());
  expect(apiMocks.writeCompanyScores).toHaveBeenCalledExactlyOnceWith("company-1", {
    companyStability: 5,
    employmentStability: 3,
  });
});
