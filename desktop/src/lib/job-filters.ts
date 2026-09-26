export type JobFilterDraft = {
  search: string;
  postingState: string;
};

export function filterDraftFromUrl(search: string | null, postingState: string | null): JobFilterDraft {
  return {
    search: search ?? "",
    postingState: postingState ?? "",
  };
}

export function filterCompaniesBySearch<T extends { name: string }>(companies: T[], search: string): T[] {
  const query = search.trim().toLocaleLowerCase();
  if (!query) return companies;

  return companies.filter((company) => company.name.toLocaleLowerCase().includes(query));
}
