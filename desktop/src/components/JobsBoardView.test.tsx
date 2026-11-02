// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { JobListSummary, JobStatus } from "@/lib/schema";
import { COMPANY_STABILITY_SCALE_LABEL } from "./CompanyStabilitySelect";
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

function makeJob(id: string, status: JobStatus, companyStability: number | null): JobListSummary {
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

function stabilityLabels() {
  return [...container.querySelectorAll<HTMLElement>(`[title="${COMPANY_STABILITY_SCALE_LABEL}"]`)];
}

it("shows a Stability label with the scale tooltip only for scored jobs", () => {
  act(() =>
    root.render(
      <MemoryRouter>
        <JobsBoardView
          jobs={[
            makeJob("a", "wishlist", 4),
            makeJob("b", "wishlist", null),
            makeJob("c", "archived", 1),
            makeJob("d", "archived", null),
          ]}
          onToggleFavorite={vi.fn()}
        />
      </MemoryRouter>,
    ),
  );
  const labels = stabilityLabels();
  expect(labels.map((el) => el.textContent?.trim())).toEqual(["Stability 4", "· Stability 1"]);
  expect(container.textContent?.match(/Stability/g)).toHaveLength(2);
});
