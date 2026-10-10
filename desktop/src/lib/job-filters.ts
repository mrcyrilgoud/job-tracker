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

export function validateSalaryRange(salaryMin: string, salaryMax: string): string | null {
  const minimum = salaryMin.trim();
  const maximum = salaryMax.trim();
  const parsedMinimum = minimum ? parseSalaryBound(minimum) : undefined;
  const parsedMaximum = maximum ? parseSalaryBound(maximum) : undefined;

  if ((minimum && parsedMinimum === undefined) || (maximum && parsedMaximum === undefined)) {
    return "Enter whole-dollar amounts of 0 or more.";
  }
  if (parsedMinimum !== undefined && parsedMaximum !== undefined && parsedMinimum > parsedMaximum) {
    return "Minimum income must not exceed maximum income.";
  }
  return null;
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
