import { describe, expect, it } from "vitest";

import type { JobListItem } from "@/lib/schema";

import {
  filterNewRoles,
  groupNewRolesByCompany,
  newRolesCompanyOptions,
  newRolesFiltersFromParams,
  parseSalaryBound,
  sortNewRolesNewestFirst,
} from "./new-roles-filters";

function role(
  id: string,
  companyId: string,
  companyName: string,
  overrides: Partial<JobListItem["job"]> = {},
): JobListItem {
  return {
    companyName,
    job: {
      id,
      companyId,
      title: id,
      updatedAt: "2026-10-01T00:00:00.000Z",
      location: null,
      postingState: "active",
      salaryMin: null,
      salaryMax: null,
      ...overrides,
    } as JobListItem["job"],
  };
}

describe("New Roles filters", () => {
  it("matches case-insensitive title, company, and location search with a company filter", () => {
    const roles = [
      role("Senior Engineer", "acme", "Acme", { location: "San Francisco" }),
      role("Product Designer", "globex", "Globex", { location: "San Francisco" }),
      role("Data Analyst", "acme", "Acme", { location: "New York" }),
    ];

    expect(filterNewRoles(roles, {
      search: "SAN FRANCISCO",
      companyId: "acme",
      postingState: "",
      groupByCompany: false,
    }).map(({ job }) => job.id)).toEqual(["Senior Engineer"]);
    expect(filterNewRoles(roles, {
      search: "GLOBEX",
      companyId: "",
      postingState: "",
      groupByCompany: false,
    }).map(({ job }) => job.id)).toEqual(["Product Designer"]);
  });

  it("filters posting status while keeping unknown statuses under All", () => {
    const roles = [
      role("active", "acme", "Acme", { postingState: "active" }),
      role("inactive", "acme", "Acme", { postingState: "inactive" }),
      role("unknown", "acme", "Acme", { postingState: "unknown" }),
    ];

    expect(filterNewRoles(roles, {
      search: "",
      companyId: "",
      postingState: "active",
      groupByCompany: false,
    }).map(({ job }) => job.id)).toEqual(["active"]);
    expect(filterNewRoles(roles, {
      search: "",
      companyId: "",
      postingState: "",
      groupByCompany: false,
    }).map(({ job }) => job.id)).toEqual(["active", "inactive", "unknown"]);
  });

  it("uses overlapping salary ranges, supports open bounds, and excludes unsalaried roles", () => {
    const roles = [
      role("overlap", "acme", "Acme", { salaryMin: 120_000, salaryMax: 180_000 }),
      role("open-high", "acme", "Acme", { salaryMin: 150_000, salaryMax: null }),
      role("open-low", "acme", "Acme", { salaryMin: null, salaryMax: 130_000 }),
      role("no-salary", "acme", "Acme"),
    ];

    expect(filterNewRoles(roles, {
      search: "",
      companyId: "",
      postingState: "",
      salaryMin: 140_000,
      salaryMax: 160_000,
      groupByCompany: false,
    }).map(({ job }) => job.id)).toEqual(["overlap", "open-high"]);
    expect(filterNewRoles(roles, {
      search: "",
      companyId: "",
      postingState: "",
      salaryMax: 140_000,
      groupByCompany: false,
    }).map(({ job }) => job.id)).toEqual(["overlap", "open-low"]);
  });

  it("parses URL filters and ignores unsupported or invalid values", () => {
    expect(newRolesFiltersFromParams(new URLSearchParams(
      "search=engineer&companyId=acme&postingState=inactive&salaryMin=120000&salaryMax=nope&groupBy=company",
    ))).toEqual({
      search: "engineer",
      companyId: "acme",
      postingState: "inactive",
      salaryMin: 120_000,
      salaryMax: undefined,
      groupByCompany: true,
    });
    expect(parseSalaryBound("-1")).toBeUndefined();
    expect(parseSalaryBound("9007199254740992")).toBeUndefined();
  });

  it("keeps company options unique and sorted, and groups names alphabetically with newest roles first", () => {
    const roles = [
      role("old-acme", "acme", "Acme", { updatedAt: "2026-10-01T00:00:00.000Z" }),
      role("new-zenith", "zenith", "Zenith", { updatedAt: "2026-10-03T00:00:00.000Z" }),
      role("new-acme", "acme", "Acme", { updatedAt: "2026-10-04T00:00:00.000Z" }),
    ];

    expect(newRolesCompanyOptions(roles)).toEqual([
      { id: "acme", name: "Acme" },
      { id: "zenith", name: "Zenith" },
    ]);
    expect(sortNewRolesNewestFirst(roles).map(({ job }) => job.id)).toEqual([
      "new-acme", "new-zenith", "old-acme",
    ]);
    expect(groupNewRolesByCompany(roles).map(({ name, roles: grouped }) => [
      name,
      grouped.map(({ job }) => job.id),
    ])).toEqual([
      ["Acme", ["new-acme", "old-acme"]],
      ["Zenith", ["new-zenith"]],
    ]);
  });
});
