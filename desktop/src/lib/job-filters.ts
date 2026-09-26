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
