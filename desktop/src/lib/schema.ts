export const jobStatuses = [
  "wishlist",
  "applied",
  "interviewing",
  "offer",
  "rejected",
  "withdrawn",
  "closed",
  "archived",
] as const;

export type JobStatus = (typeof jobStatuses)[number];

export type JobSource =
  | "manual"
  | "greenhouse"
  | "lever"
  | "ashby"
  | "careers_page";

export type PostingState = "active" | "inactive" | "unknown";

export type WatchDisposition = "new" | "saved" | "dismissed";

export const documentKinds = [
  "resume",
  "cover_letter",
  "portfolio",
  "work_sample",
  "other",
] as const;

export type DocumentKind = (typeof documentKinds)[number];

export const watchProviders = ["greenhouse", "lever", "ashby"] as const;

export type WatchProvider = (typeof watchProviders)[number];

export type DetectedBoard = {
  provider: WatchProvider;
  boardSlug: string;
  boardUrl: string;
  postingId: string | null;
};

export type Company = {
  id: string;
  name: string;
  careersUrl: string | null;
  createdAt: string;
  updatedAt: string;
};

export type Job = {
  id: string;
  companyId: string;
  title: string;
  url: string;
  canonicalUrl: string;
  sourceExternalId: string | null;
  status: JobStatus;
  appliedAt: string | null;
  postingState: PostingState;
  lastCheckedAt: string | null;
  lastCheckResult: string | null;
  source: JobSource;
  notes: string | null;
  description: string | null;
  location: string | null;
  isNewFromWatch: boolean;
  watchDisposition: WatchDisposition | null;
  missingFromSyncCount: number;
  isFavorite: boolean;
  createdAt: string;
  updatedAt: string;
  /** Overall appeal from 1 (least) to 5 (most). Null when unscored. */
  appeal: number | null;
  /** Optional annual USD salary bounds; a missing bound is open-ended. */
  salaryMin: number | null;
  salaryMax: number | null;
};

export type MatchMode = "word" | "substring";

export type RemoteMode = "any" | "remoteOnly" | "onsiteOnly";

export type TitleCriteria = {
  include: string[];
  exclude: string[];
  matchMode: MatchMode;
};

export type LocationCriteria = {
  country: string | null;
  include: string[];
  exclude: string[];
  matchMode: MatchMode;
};

export type FilterCriteria = {
  version: number;
  title: TitleCriteria;
  location: LocationCriteria;
  remote: RemoteMode;
};

export type CompanyWatch = {
  id: string;
  companyId: string;
  provider: WatchProvider;
  boardSlug: string;
  lastSyncedAt: string | null;
  consecutiveSyncFailures: number;
  lastSyncError: string | null;
  filterCriteria?: FilterCriteria | null;
  createdAt: string;
  updatedAt: string;
};

export type JobEvent = {
  id: string;
  jobId: string;
  type: string;
  note: string | null;
  occurredAt: string;
};

export type Document = {
  id: string;
  originalFilename: string;
  storedFilename: string;
  mimeType: string;
  checksum: string;
  sizeBytes: number;
  importedAt: string;
};

export type JobDocument = {
  id: string;
  jobId: string;
  documentId: string;
  kind: DocumentKind;
  usedAt: string;
};

export type CareersPageReview = {
  id: string;
  companyId: string;
  previousHash: string | null;
  currentHash: string;
  summary: string;
  status: string;
  createdAt: string;
};

export type WeeklyActivity = {
  total: number;
  days: Array<{ key: string; label: string; count: number; isToday: boolean }>;
};

export type JobListItem = {
  job: Job;
  companyName: string;
};

export type JobDetail = {
  job: Job;
  company: Company;
  events: JobEvent[];
  attached: Array<{
    attachment: JobDocument;
    document: Document;
  }>;
};

export type CompanyRow = {
  company: Company;
  watches: CompanyWatch[];
  reviews: CareersPageReview[];
  openPositionCount: number;
};

export type DocumentListItem = Document & {
  usedBy: string[];
  kinds: DocumentKind[];
};
