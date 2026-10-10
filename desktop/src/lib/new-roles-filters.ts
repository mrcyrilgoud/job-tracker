import type { JobListItem } from "@/lib/schema";

import { matchesRoleSearch } from "@/lib/companies-ui";
import { parseSalaryBound } from "@/lib/job-filters";

export { parseSalaryBound };

export type NewRolesFilters = {
  search: string;
  companyId: string;
  postingState: "" | "active" | "inactive";
  salaryMin?: number;
  salaryMax?: number;
  groupByCompany: boolean;
};

export type NewRolesCompany = {
  id: string;
  name: string;
};

export type NewRolesCompanyGroup = NewRolesCompany & {
  roles: JobListItem[];
};

export function newRolesFiltersFromParams(params: URLSearchParams): NewRolesFilters {
  const postingState = params.get("postingState");
  return {
    search: params.get("search") ?? "",
    companyId: params.get("companyId") ?? "",
    postingState: postingState === "active" || postingState === "inactive" ? postingState : "",
    salaryMin: parseSalaryBound(params.get("salaryMin")),
    salaryMax: parseSalaryBound(params.get("salaryMax")),
    groupByCompany: params.get("groupBy") === "company",
  };
}

export function newRolesCompanyOptions(roles: JobListItem[]): NewRolesCompany[] {
  const companies = new Map<string, string>();
  for (const { job, companyName } of roles) {
    if (!companies.has(job.companyId)) companies.set(job.companyId, companyName);
  }
  return [...companies]
    .map(([id, name]) => ({ id, name }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

function isSalaryMatch(
  role: JobListItem,
  salaryMin: number | undefined,
  salaryMax: number | undefined,
): boolean {
  if (salaryMin === undefined && salaryMax === undefined) return true;

  const { salaryMin: roleMin, salaryMax: roleMax } = role.job;
  if (roleMin === null && roleMax === null) return false;
  if (salaryMin !== undefined && (roleMax ?? Number.MAX_SAFE_INTEGER) < salaryMin) return false;
  if (salaryMax !== undefined && (roleMin ?? 0) > salaryMax) return false;
  return true;
}

export function filterNewRoles(roles: JobListItem[], filters: NewRolesFilters): JobListItem[] {
  return roles.filter((role) => {
    if (filters.companyId && role.job.companyId !== filters.companyId) return false;
    if (filters.postingState && role.job.postingState !== filters.postingState) return false;
    if (!matchesRoleSearch(role, filters.search)) return false;
    return isSalaryMatch(role, filters.salaryMin, filters.salaryMax);
  });
}

function compareNewestFirst(a: JobListItem, b: JobListItem): number {
  const aUpdated = Date.parse(a.job.updatedAt);
  const bUpdated = Date.parse(b.job.updatedAt);
  if (Number.isFinite(aUpdated) && Number.isFinite(bUpdated) && aUpdated !== bUpdated) {
    return bUpdated - aUpdated;
  }
  return b.job.updatedAt.localeCompare(a.job.updatedAt) || a.job.title.localeCompare(b.job.title);
}

export function sortNewRolesNewestFirst(roles: JobListItem[]): JobListItem[] {
  return [...roles].sort(compareNewestFirst);
}

export function groupNewRolesByCompany(roles: JobListItem[]): NewRolesCompanyGroup[] {
  const grouped = new Map<string, NewRolesCompanyGroup>();
  for (const role of roles) {
    const { companyId } = role.job;
    const group = grouped.get(companyId);
    if (group) {
      group.roles.push(role);
    } else {
      grouped.set(companyId, { id: companyId, name: role.companyName, roles: [role] });
    }
  }

  return [...grouped.values()]
    .map((group) => ({ ...group, roles: sortNewRolesNewestFirst(group.roles) }))
    .sort((a, b) => a.name.localeCompare(b.name));
}
