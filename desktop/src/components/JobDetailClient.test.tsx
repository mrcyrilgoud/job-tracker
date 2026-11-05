// @vitest-environment jsdom
import { act } from "react";
import { MemoryRouter } from "react-router-dom";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { JobDetail } from "@/lib/schema";

const apiMocks = vi.hoisted(() => ({
  updateJob: vi.fn(),
}));

vi.mock("@/lib/api", () => ({ api: apiMocks }));

import { COMPANY_STABILITY_SCALE_LABEL } from "./CompanyStabilitySelect";
import { EMPLOYMENT_STABILITY_SCALE_LABEL } from "./EmploymentStabilitySelect";
import { JobDetailClient } from "./JobDetailClient";

let container: HTMLDivElement;
let root: Root;

const initial = {
  title: "Engineer",
  companyName: "Acme",
  status: "wishlist" as const,
  appliedAt: null,
  notes: null,
  description: null,
  postingState: "active" as const,
  lastCheckedAt: null,
  lastCheckResult: null,
  url: "https://example.com/jobs/1",
  companyId: "company-1",
  isNewFromWatch: false,
  isFavorite: false,
  appeal: null,
  applicationEase: null,
  companyStability: 2,
  companyStabilityRationale: null,
  companyStabilityCitations: null,
  employmentStability: null,
  employmentStabilityRationale: null,
  employmentStabilityCitations: null,
  salaryMin: null,
  salaryMax: null,
};

type InitialOverrides = Partial<
  Omit<
    typeof initial,
    | "appeal"
    | "applicationEase"
    | "companyStability"
    | "companyStabilityRationale"
    | "companyStabilityCitations"
    | "employmentStability"
    | "employmentStabilityRationale"
    | "employmentStabilityCitations"
  >
> & {
  appeal?: number | null;
  applicationEase?: number | null;
  companyStability?: number | null;
  companyStabilityRationale?: string | null;
  companyStabilityCitations?: string[] | null;
  employmentStability?: number | null;
  employmentStabilityRationale?: string | null;
  employmentStabilityCitations?: string[] | null;
};

function detailWith(companyStability: number | null): JobDetail {
  return {
    job: { ...initial, id: "job-1", companyStability },
    company: { name: "Acme" },
    events: [],
    attached: [],
  } as unknown as JobDetail;
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  apiMocks.updateJob.mockReset();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

function render(onUpdated = vi.fn(), overrides: InitialOverrides = {}) {
  act(() =>
    root.render(
      <MemoryRouter>
        <JobDetailClient
          jobId="job-1"
          initial={{ ...initial, ...overrides }}
          companyNames={[]}
          onUpdated={onUpdated}
        />
      </MemoryRouter>,
    ),
  );
  return container.querySelector<HTMLSelectElement>("#job-company-stability")!;
}

function saveButton() {
  return [...container.querySelectorAll("button")].find((b) => b.textContent === "Save")!;
}

function choose(select: HTMLSelectElement, value: string) {
  act(() => {
    select.value = value;
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

it("labels and preselects both stored company scores", () => {
  const companySelect = render(undefined, { employmentStability: 4 });
  const employmentSelect = container.querySelector<HTMLSelectElement>("#job-employment-stability")!;
  expect(container.querySelector('label[for="job-company-stability"]')?.textContent).toBe(
    COMPANY_STABILITY_SCALE_LABEL,
  );
  expect(container.querySelector('label[for="job-employment-stability"]')?.textContent).toBe(
    EMPLOYMENT_STABILITY_SCALE_LABEL,
  );
  expect(companySelect.value).toBe("2");
  expect(employmentSelect.value).toBe("4");
  expect(container.textContent).toContain("apply to every job at Acme");
  expect(container.querySelector('a[href="/companies/company-1"]')?.textContent).toBe(
    "View company details",
  );
});

it("sends only the changed company score fields", async () => {
  apiMocks.updateJob.mockResolvedValue({ detail: detailWith(2) });
  render();
  const select = container.querySelector<HTMLSelectElement>("#job-employment-stability")!;
  choose(select, "4");
  await act(async () => saveButton().click());
  const payload = apiMocks.updateJob.mock.calls[0][1];
  expect(payload).toMatchObject({ employmentStability: 4 });
  expect(payload).not.toHaveProperty("companyStability");
});

it("shows read-only rationale and citation links only for scored companies", () => {
  render(undefined, {
    companyStability: 4,
    companyStabilityRationale: "Strong parent and durable revenue.",
    companyStabilityCitations: ["https://example.com/report"],
  });
  expect(container.textContent).toContain("Strong parent and durable revenue.");
  const citation = container.querySelector<HTMLAnchorElement>('a[href="https://example.com/report"]')!;
  expect(citation.target).toBe("_blank");
  expect(citation.rel).toContain("noreferrer");
  expect(container.textContent).not.toContain("Employment stability evidence");
});

it("preserves score edits and dirty state when saving fails", async () => {
  apiMocks.updateJob.mockRejectedValue(new Error("Network unavailable"));
  const select = render();
  choose(select, "5");
  await act(async () => saveButton().click());
  expect(select.value).toBe("5");
  expect(saveButton().disabled).toBe(false);
  expect(container.textContent).toContain("Unsaved changes");
  expect(container.textContent).toContain("Network unavailable");
});

it("marks the form dirty on a stability change and clean when reverted", () => {
  const select = render();
  choose(select, "5");
  expect(saveButton().disabled).toBe(false);
  expect(container.textContent).toContain("Unsaved changes");
  choose(select, "2");
  expect(saveButton().disabled).toBe(true);
});

it("sends the chosen stability (or null when unscored) in the save payload", async () => {
  apiMocks.updateJob.mockResolvedValue({ detail: detailWith(null) });
  const onUpdated = vi.fn();
  const select = render(onUpdated);
  choose(select, "");
  await act(async () => saveButton().click());
  expect(apiMocks.updateJob).toHaveBeenCalledTimes(1);
  expect(apiMocks.updateJob.mock.calls[0][0]).toBe("job-1");
  expect(apiMocks.updateJob.mock.calls[0][1]).toMatchObject({ companyStability: null });
  expect(onUpdated).toHaveBeenCalledWith(expect.objectContaining({ mode: "save" }));
  expect(select.value).toBe("");
  expect(saveButton().disabled).toBe(true);
});

it("preserves posting scores and favorites while changing only employment stability", async () => {
  const onUpdated = vi.fn();
  apiMocks.updateJob.mockResolvedValue({
    detail: {
      ...detailWith(2),
      job: {
        ...initial,
        id: "job-1",
        appeal: 4,
        applicationEase: 2,
        isFavorite: true,
        companyStability: 2,
        employmentStability: 4,
      },
      company: { name: "Acme" },
    },
  });
  render(onUpdated, { appeal: 4, applicationEase: 2, isFavorite: true });
  choose(container.querySelector<HTMLSelectElement>("#job-employment-stability")!, "4");
  await act(async () => saveButton().click());

  expect(apiMocks.updateJob.mock.calls[0][1]).toMatchObject({
    appeal: 4,
    applicationEase: 2,
    isFavorite: true,
    employmentStability: 4,
  });
  expect(apiMocks.updateJob.mock.calls[0][1]).not.toHaveProperty("companyStability");
  expect(onUpdated.mock.calls[0][0].detail.job).toMatchObject({
    appeal: 4,
    applicationEase: 2,
    isFavorite: true,
    companyStability: 2,
    employmentStability: 4,
  });
});

it("uses the existing leave warning only while a score edit is dirty", () => {
  render();
  const clean = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(clean);
  expect(clean.defaultPrevented).toBe(false);

  choose(container.querySelector<HTMLSelectElement>("#job-employment-stability")!, "4");
  const dirty = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(dirty);
  expect(dirty.defaultPrevented).toBe(true);
});
