import { invoke } from "@tauri-apps/api/core";

import type {
  Company,
  CompanyRow,
  DetectedBoard,
  Document,
  DocumentKind,
  DocumentListItem,
  FilterCriteria,
  Job,
  JobDetail,
  JobListItem,
  JobStatus,
  WeeklyActivity,
  WatchProvider,
} from "@/lib/schema";
import { DESKTOP_SHELL_REQUIRED, isDesktopShell } from "@/lib/tauri";

export type { FilterCriteria, MatchMode, RemoteMode } from "@/lib/schema";

export type FilterPreviewSample = {
  title: string;
  location?: string | null;
};

export type FilterPreviewOutcome = {
  included: boolean;
  reason: string;
};

export type JobFilters = {
  status?: string;
  companyId?: string;
  postingState?: string;
  search?: string;
  location?: string;
  newFromWatch?: boolean;
  isFavorite?: boolean;
  isArchived?: boolean;
  limit?: number;
};

export type LocationSettings = {
  country: string;
  cities: string;
};

export type CreateJobInput = {
  url: string;
  title?: string;
  companyName?: string;
  status?: JobStatus;
  appliedAt?: string | null;
  notes?: string | null;
  description?: string | null;
  location?: string | null;
  confirmedDiscovery?: ConfirmedJobDiscovery | null;
};

export type ConfirmedJobDiscovery = {
  provider?: WatchProvider;
  boardSlug?: string;
  careersUrl?: string;
};

export type JobUrlPreview = {
  title: string | null;
  companyName: string | null;
  description: string | null;
  board: DetectedBoard | null;
  careersUrl: string | null;
};

export type UpdateJobInput = {
  title?: string;
  companyName?: string;
  status?: JobStatus;
  appliedAt?: string | null;
  notes?: string | null;
  description?: string | null;
  location?: string | null;
  url?: string;
  isNewFromWatch?: boolean;
  isFavorite?: boolean;
};

export type ImportDocumentInput = {
  originalFilename: string;
  mimeType: string;
  bytesBase64: string;
  jobId?: string;
  kind?: DocumentKind;
};

export type CsvConfig = {
  path: string;
  defaultPath: string;
  isCustom: boolean;
};

export type CsvPathStatus = {
  path: string;
  exists: boolean;
  defaultPath: string;
};

export type JobsRunnerProgress = {
  phase: string;
  message: string;
  current?: number;
  total?: number;
};

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isDesktopShell()) {
    throw new Error(DESKTOP_SHELL_REQUIRED);
  }
  return invoke<T>(command, args);
}

export const api = {
  listJobs: (filters?: JobFilters) =>
    call<{
      jobs: JobListItem[];
    }>("list_jobs_cmd", { filters: filters ?? null }),

  getJobsDashboard: () =>
    call<{
      counts: Record<string, number>;
      weeklyActivity: WeeklyActivity;
      dataDir: string;
    }>("get_jobs_dashboard"),

  createJob: (input: CreateJobInput) =>
    call<{ job: Job; company: Company }>("create_job", { input }),

  previewJobUrl: (url: string) => call<JobUrlPreview>("preview_job_url", { url }),

  getJob: (id: string) => call<{ detail: JobDetail }>("get_job", { id }),

  updateJob: (id: string, updates: UpdateJobInput) =>
    call<{ detail: JobDetail }>("update_job_cmd", { id, updates }),

  deleteJob: (id: string) => call<{ success: boolean; id: string }>("delete_job", { id }),

  archiveJob: (id: string) => call<{ detail: JobDetail }>("archive_job_cmd", { id }),

  unarchiveJob: (id: string, targetStatus?: JobStatus) =>
    call<{ detail: JobDetail }>("unarchive_job_cmd", { id, targetStatus: targetStatus ?? null }),

  toggleFavorite: (jobId: string) =>
    call<{ item: JobListItem }>("toggle_job_favorite_cmd", { jobId }),

  setFavorite: (jobId: string, isFavorite: boolean) =>
    call<{ item: JobListItem }>("set_job_favorite_cmd", { jobId, isFavorite }),

  checkJobPosting: (id: string) =>
    call<{ postingState: string; lastCheckedAt: string; lastCheckResult: string | null }>(
      "check_job_posting",
      { id },
    ),

  listCompanies: () => call<{ companies: CompanyRow[] }>("list_companies"),

  listOpenWatchPositions: (companyId: string) =>
    call<{ positions: JobListItem[] }>("list_open_watch_positions_cmd", { companyId }),

  createCompany: (name: string, careersUrl?: string | null) =>
    call<{ company: Company }>("create_company", {
      input: { name, careersUrl: careersUrl ?? null },
    }),

  createWatch: (companyId: string, provider: WatchProvider, boardSlug: string) =>
    call<{ watch: unknown }>("create_watch", {
      input: { companyId, provider, boardSlug },
    }),

  deleteWatch: (watchId: string) => call<{ ok: boolean }>("delete_watch", { watchId }),

  syncWatch: (watchId: string) =>
    call<{ ok: boolean; created: number; error?: string }>("sync_watch", { watchId }),

  checkCareers: (companyId: string) =>
    call<{ ok: boolean; changed?: boolean }>("check_careers", { companyId }),

  dismissReview: (reviewId: string) =>
    call<{ ok: boolean }>("dismiss_review", { reviewId }),

  approveWatchJob: (jobId: string) =>
    call<{ job: Job }>("approve_watch_job_cmd", { jobId }),

  dismissWatchJob: (jobId: string) =>
    call<{ job: Job }>("dismiss_watch_job_cmd", { jobId }),

  saveOpenWatchJob: (jobId: string) =>
    call<{ job: Job }>("save_open_watch_job_cmd", { jobId }),

  resetDismissedWatchJob: (jobId: string) =>
    call<{ job: Job }>("reset_dismissed_watch_job_cmd", { jobId }),

  listDocuments: () => call<{ documents: DocumentListItem[] }>("list_documents"),

  importDocument: (input: ImportDocumentInput) =>
    call<{ document: Document; attachment?: unknown }>("import_document", { input }),

  attachDocument: (jobId: string, documentId: string, kind: DocumentKind) =>
    call<{ attachment: unknown }>("attach_document", { jobId, documentId, kind }),

  openDocument: (documentId: string) => call<{ ok: boolean }>("open_document", { documentId }),

  csvConfig: () => call<CsvConfig>("csv_config"),

  csvPathStatus: (path: string) => call<CsvPathStatus>("csv_path_status", { path }),

  configureCsv: (path: string, mode: "import" | "replace") =>
    call<CsvConfig>("csv_configure", { input: { path, mode } }),

  resetCsvConfig: () => call<CsvConfig>("csv_reset_config"),

  runJobsCycle: () => call<Record<string, unknown>>("run_jobs_cycle_cmd"),

  checkAllPostings: () => call<Record<string, unknown>>("check_all_postings_cmd"),

  getWatchRoleKeywords: () => call<string>("get_watch_role_keywords"),

  setWatchRoleKeywords: (keywords: string) =>
    call<{ ok: boolean }>("set_watch_role_keywords", { keywords }),

  getLocationSettings: () => call<LocationSettings>("get_location_settings_cmd"),

  setLocationSettings: (settings: LocationSettings) =>
    call<{ ok: boolean }>("set_location_settings_cmd", { settings }),

  getFilterCriteria: () => call<FilterCriteria>("get_filter_criteria"),

  setFilterCriteria: (criteria: FilterCriteria) =>
    call<void>("set_filter_criteria", { criteria }),

  getWatchFilterCriteria: (watchId: string) =>
    call<FilterCriteria | null>("get_watch_filter_criteria", { watchId }),

  setWatchFilterCriteria: (watchId: string, criteria: FilterCriteria | null) =>
    call<void>("set_watch_filter_criteria", { watchId, criteria }),

  previewFilterMatch: (criteria: FilterCriteria, samples: FilterPreviewSample[]) =>
    call<FilterPreviewOutcome[]>("preview_filter_match", { criteria, samples }),

  showMainWindow: () => call<void>("show_main_window"),
};
