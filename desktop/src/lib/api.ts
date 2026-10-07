import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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
import {
  parseRunAccepted,
  parseRunEvent,
  parseRunSnapshot,
  RUN_PROGRESS_EVENT,
  unwrapContract,
  type ParseResult,
  type RunAccepted,
  type RunProgressEvent,
  type RunSnapshot,
  type RunType,
} from "@/lib/run-contract";
import { DESKTOP_SHELL_REQUIRED, isDesktopShell } from "@/lib/tauri";

export type { FilterCriteria, MatchMode, RemoteMode } from "@/lib/schema";

export type RetryRunInput = {
  sourceRunId: string;
  jobIds: string[];
};

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
  salaryMin?: number;
  salaryMax?: number;
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
  /** 1–5, where 5 is most appealing. Null clears the score. */
  appeal?: number | null;
  salaryMin?: number | null;
  salaryMax?: number | null;
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

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isDesktopShell()) {
    throw new Error(DESKTOP_SHELL_REQUIRED);
  }
  return invoke<T>(command, args);
}

/** Invoke a run command and validate its response against Progress_Contract v1. */
async function callValidated<T>(
  command: string,
  parse: (input: unknown) => ParseResult<T>,
  args?: Record<string, unknown>,
): Promise<T> {
  return unwrapContract(parse(await call<unknown>(command, args)));
}

/**
 * Subscribe to `jobs-runner-progress`. Every payload is validated with
 * `parseRunEvent`; the handler receives the parse result so the caller can keep
 * its last valid state on `invalid` or `unsupported_version` (Req 11.8, 11.9).
 */
async function listenRunProgress(
  handler: (result: ParseResult<RunProgressEvent>) => void,
): Promise<UnlistenFn> {
  if (!isDesktopShell()) {
    throw new Error(DESKTOP_SHELL_REQUIRED);
  }
  return listen<unknown>(RUN_PROGRESS_EVENT, (event) => handler(parseRunEvent(event.payload)));
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

  deleteJobs: (ids: string[]) =>
    call<{ success: boolean; deletedCount: number }>("delete_jobs", { ids }),

  archiveJob: (id: string) => call<{ detail: JobDetail }>("archive_job_cmd", { id }),

  unarchiveJob: (id: string, targetStatus?: JobStatus) =>
    call<{ detail: JobDetail }>("unarchive_job_cmd", { id, targetStatus: targetStatus ?? null }),

  unarchiveJobs: (ids: string[]) =>
    call<{ success: boolean; restoredCount: number }>("unarchive_jobs", { ids }),

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

  // Run model (Progress_Contract v1). Responses are validated; a contract
  // violation rejects with `RunContractError`. Backend rejections keep their
  // `code:category[:detail]` strings (see `parseAppError`).

  /** Accept a run and return immediately; progress arrives on `listenRunProgress`. */
  startRun: (runType: RunType) =>
    callValidated<RunAccepted>("start_run_cmd", parseRunAccepted, { input: { runType } }),

  retryRun: (input: RetryRunInput) =>
    callValidated<RunAccepted>("retry_run_cmd", parseRunAccepted, {
      input: { sourceRunId: input.sourceRunId, jobIds: input.jobIds },
    }),

  cancelRun: (runId: string) =>
    callValidated<RunSnapshot>("cancel_run_cmd", parseRunSnapshot, { runId }),

  getRun: (runId: string) => callValidated<RunSnapshot>("get_run_cmd", parseRunSnapshot, { runId }),

  /** The non-terminal run, else the latest undismissed terminal run, else `null`. */
  getCurrentRun: async (): Promise<RunSnapshot | null> => {
    const raw = await call<unknown>("get_current_run_cmd");
    return raw === null || raw === undefined ? null : unwrapContract(parseRunSnapshot(raw));
  },

  dismissRun: (runId: string) => call<{ ok: boolean }>("dismiss_run_cmd", { runId }),

  listenRunProgress,

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
