// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

const apiMocks = vi.hoisted(() => ({
  listCompanies: vi.fn(),
  listWatchRoles: vi.fn(),
  writeCompanyScores: vi.fn(),
}));

vi.mock("@/lib/api", () => ({ api: apiMocks }));
vi.mock("@/components/companies/CompanyCard", () => ({ CompanyCard: () => <div data-testid="company-card" /> }));
vi.mock("@/components/companies/OpenPositionsPanel", () => ({ OpenPositionsPanel: () => <div data-testid="open-positions" /> }));
vi.mock("@/lib/use-company-actions", () => ({
  useCompanyActions: () => ({
    openRoles: new Set<string>(),
    toggleRoles: vi.fn(),
    syncWatch: vi.fn(),
    removeWatch: vi.fn(),
    checkCareers: vi.fn(),
    dismissReview: vi.fn(),
    triageRole: vi.fn(),
    saveOpenRole: vi.fn(),
    dismissNewRole: vi.fn(),
    resetDismissedRole: vi.fn(),
    isPending: () => false,
    feedback: {},
  }),
}));

import { CompanyDetailPage } from "./CompanyDetailPage";
import type { CompanyRow } from "@/lib/schema";

const row: CompanyRow = {
  company: {
    id: "company-1",
    name: "Acme",
    careersUrl: null,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    companyStability: 3,
    companyStabilityRationale: null,
    companyStabilityCitations: null,
    companyStabilityScoredAt: null,
    companyStabilityScoredBy: null,
    employmentStability: null,
    employmentStabilityRationale: null,
    employmentStabilityCitations: null,
    employmentStabilityScoredAt: null,
    employmentStabilityScoredBy: null,
  },
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
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  apiMocks.listCompanies.mockResolvedValue({ companies: [row] });
  apiMocks.listWatchRoles.mockResolvedValue([]);
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

async function renderPage() {
  await act(async () => {
    root.render(
      <MemoryRouter initialEntries={["/companies/company-1"]}>
        <Routes>
          <Route path="/companies/:id" element={<CompanyDetailPage />} />
        </Routes>
      </MemoryRouter>,
    );
  });
}

it("integrates the company score editor into the detail page", async () => {
  await renderPage();
  expect(container.querySelector('[data-testid="company-card"]')).not.toBeNull();
  expect(container.querySelector("#company-score")).not.toBeNull();
  expect(container.querySelector("#employment-score")).not.toBeNull();
  expect(container.textContent).toContain("Company scores");
});

it("replaces editor metadata with the returned company after saving", async () => {
  const returnedCompany = {
    ...row.company,
    companyStability: 5,
    companyStabilityScoredAt: "2026-02-03T12:34:56.789Z",
    companyStabilityScoredBy: "manual" as const,
  };
  apiMocks.writeCompanyScores.mockResolvedValue({ company: returnedCompany, changed: true, warnings: [] });
  await renderPage();
  await act(async () => {
    const select = container.querySelector<HTMLSelectElement>("#company-score")!;
    select.value = "5";
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await act(async () => {
    container.querySelector<HTMLButtonElement>('button[type="submit"]')?.click();
  });
  expect(apiMocks.writeCompanyScores).toHaveBeenCalledWith("company-1", { companyStability: 5 });
  expect(container.textContent).toContain("manual");
  expect(container.querySelector<HTMLSelectElement>("#company-score")?.value).toBe("5");
});

it("keeps both score sections dirty and blocks leave or reload while editing", async () => {
  await renderPage();
  await act(async () => {
    const companySelect = container.querySelector<HTMLSelectElement>("#company-score")!;
    companySelect.value = "5";
    companySelect.dispatchEvent(new Event("change", { bubbles: true }));
    const employmentSelect = container.querySelector<HTMLSelectElement>("#employment-score")!;
    employmentSelect.value = "4";
    employmentSelect.dispatchEvent(new Event("change", { bubbles: true }));
  });

  expect(container.textContent).toContain("Unsaved changes");
  const leaveEvent = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(leaveEvent);
  expect(leaveEvent.defaultPrevented).toBe(true);
});

it("replaces both score sections with metadata returned by the detail save", async () => {
  const returnedCompany = {
    ...row.company,
    companyStability: 5,
    companyStabilityRationale: "Durable parent",
    companyStabilityCitations: ["https://example.com/company"],
    companyStabilityScoredAt: "2026-02-03T12:34:56.789Z",
    companyStabilityScoredBy: "manual" as const,
    employmentStability: 4,
    employmentStabilityRationale: "Stable hiring trend",
    employmentStabilityCitations: ["https://example.com/employment"],
    employmentStabilityScoredAt: "2026-02-03T12:34:56.789Z",
    employmentStabilityScoredBy: "import" as const,
  };
  apiMocks.writeCompanyScores.mockResolvedValue({ company: returnedCompany, changed: true, warnings: [] });
  await renderPage();
  await act(async () => {
    const companySelect = container.querySelector<HTMLSelectElement>("#company-score")!;
    companySelect.value = "5";
    companySelect.dispatchEvent(new Event("change", { bubbles: true }));
    const employmentSelect = container.querySelector<HTMLSelectElement>("#employment-score")!;
    employmentSelect.value = "4";
    employmentSelect.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await act(async () => {
    container.querySelector<HTMLButtonElement>('button[type="submit"]')?.click();
  });

  expect(apiMocks.writeCompanyScores).toHaveBeenCalledWith("company-1", {
    companyStability: 5,
    employmentStability: 4,
  });
  expect(container.textContent).toContain("Durable parent");
  expect(container.textContent).toContain("Stable hiring trend");
  expect(container.textContent).toContain("import");
  expect(container.querySelector<HTMLSelectElement>("#company-score")?.value).toBe("5");
  expect(container.querySelector<HTMLSelectElement>("#employment-score")?.value).toBe("4");
  const cleanEvent = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(cleanEvent);
  expect(cleanEvent.defaultPrevented).toBe(false);
});
