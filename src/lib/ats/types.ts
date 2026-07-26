export type AtsJob = {
  externalId: string;
  title: string;
  url: string;
  location?: string;
};

export type AtsAdapter = {
  provider: "greenhouse" | "lever" | "ashby";
  validateBoard: (boardSlug: string) => Promise<void>;
  listJobs: (boardSlug: string) => Promise<AtsJob[]>;
};
