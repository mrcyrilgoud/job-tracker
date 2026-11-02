// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { JobDetail } from "@/lib/schema";

const apiMocks = vi.hoisted(() => ({
  updateJob: vi.fn(),
}));

vi.mock("@/lib/api", () => ({ api: apiMocks }));

import { COMPANY_STABILITY_SCALE_LABEL } from "./CompanyStabilitySelect";
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
  isNewFromWatch: false,
  isFavorite: false,
  appeal: null,
  applicationEase: null,
  companyStability: 2,
  salaryMin: null,
  salaryMax: null,
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

function render(onUpdated = vi.fn()) {
  act(() =>
    root.render(
      <JobDetailClient jobId="job-1" initial={initial} companyNames={[]} onUpdated={onUpdated} />,
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

it("labels and preselects the stored company stability", () => {
  const select = render();
  const label = container.querySelector('label[for="job-company-stability"]')!;
  expect(label.textContent).toBe(COMPANY_STABILITY_SCALE_LABEL);
  expect(select.value).toBe("2");
  expect(saveButton().disabled).toBe(true);
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
