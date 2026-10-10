export type JobFilterDraft = {
  search: string;
  postingState: string;
  salaryMin: string;
  salaryMax: string;
};

export function parseSalaryBound(value: string | null): number | undefined {
  if (value === null || !/^\d+$/.test(value)) return undefined;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : undefined;
}

export function filterDraftFromUrl(
  search: string | null,
  postingState: string | null,
  salaryMin: string | null = null,
  salaryMax: string | null = null,
): JobFilterDraft {
  return {
    search: search ?? "",
    postingState: postingState ?? "",
    salaryMin: salaryMin ?? "",
    salaryMax: salaryMax ?? "",
  };
}

export function filterCompaniesBySearch<T extends { name: string }>(companies: T[], search: string): T[] {
  const query = search.trim().toLocaleLowerCase();
  if (!query) return companies;

  return companies.filter((company) => company.name.toLocaleLowerCase().includes(query));
}
