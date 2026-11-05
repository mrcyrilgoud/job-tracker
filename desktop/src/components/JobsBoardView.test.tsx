// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { JobListSummary, JobStatus } from "@/lib/schema";
import { COMPANY_STABILITY_SCALE_LABEL } from "./CompanyStabilitySelect";
import { EMPLOYMENT_STABILITY_SCALE_LABEL } from "./EmploymentStabilitySelect";
import { JobsBoardView } from "./JobsBoardView";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

function makeJob(
  id: string,
  status: JobStatus,
  companyStability: number | null,
  employmentStability: number | null,
): JobListSummary {
  const now = new Date().toISOString();
  return {
    job: {
      id,
      title: `Role ${id}`,
      status,
      postingState: "active",
      isNewFromWatch: false,
      isFavorite: false,
      source: "manual",
      location: null,
      appeal: null,
      applicationEase: null,
      companyStability,
      employmentStability,
      appliedAt: null,
      lastCheckedAt: null,
      salaryMin: null,
      salaryMax: null,
      createdAt: now,
      updatedAt: now,
    },
    companyName: "Acme",
  } as JobListSummary;
}

it("shows Company and Employment chips in order and omits unscored values", () => {
  act(() =>
    root.render(
      <MemoryRouter>
        <JobsBoardView
          jobs={[
            makeJob("a", "wishlist", 4, 2),
            makeJob("b", "wishlist", null, null),
            makeJob("c", "archived", 1, 3),
            makeJob("d", "archived", null, 4),
          ]}
          onToggleFavorite={vi.fn()}
        />
      </MemoryRouter>,
    ),
  );

  const companyLabels = [
    ...container.querySelectorAll<HTMLElement>(`[title="${COMPANY_STABILITY_SCALE_LABEL}"]`),
  ];
  const employmentLabels = [
    ...container.querySelectorAll<HTMLElement>(`[title="${EMPLOYMENT_STABILITY_SCALE_LABEL}"]`),
  ];

  expect(companyLabels.map((el) => el.textContent?.trim())).toEqual([
    "Company 4",
    "· Company 1",
  ]);
  expect(employmentLabels.map((el) => el.textContent?.trim())).toEqual([
    "Employment 2",
    "· Employment 3",
    "· Employment 4",
  ]);
  expect(container.textContent).not.toContain("Stability");

  const activeCard = container.querySelector(".card.group");
  const activeText = activeCard?.textContent ?? "";
  expect(activeText.indexOf("Ease")).toBeLessThan(activeText.indexOf("Company 4"));
  expect(activeText.indexOf("Company 4")).toBeLessThan(activeText.indexOf("Employment 2"));
});
