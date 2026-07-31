import { invoke } from "@tauri-apps/api/core";

import type {
  Company,
  CompanyRow,
  Document,
  DocumentKind,
  DocumentListItem,
  Job,
  JobDetail,
  JobListItem,
  JobStatus,
  WeeklyActivity,
  WatchProvider,
} from "@/lib/schema";
import { DESKTOP_SHELL_REQUIRED, isDesktopShell } from "@/lib/tauri";

export type JobFilters = {
  status?: string;
  companyId?: string;
  postingState?: string;
  search?: string;
  newFromWatch?: boolean;
};

export type CreateJobInput = {
  url: string;
  title?: string;
  companyName?: string;
  status?: JobStatus;
  appliedAt?: string | null;
  notes?: string | null;
  location?: string | null;
};

export type JobUrlPreview = {
  title: string | null;
  companyName: string | null;
};

export type UpdateJobInput = {
  title?: string;
  companyName?: string;
  status?: JobStatus;
  appliedAt?: string | null;
  notes?: string | null;
  location?: string | null;
  url?: string;
  isNewFromWatch?: boolean;
};

export type ImportDocumentInput = {
  originalFilename: string;
  mimeType: string;
  bytesBase64: string;
  jobId?: string;
  kind?: DocumentKind;
};

export type GmailStatus = {
  connected: boolean;
  configured: boolean;
  redirectUri: string;
  pending: Array<{
    id: string;
    subject: string | null;
    snippet: string | null;
    fromAddress: string | null;
    confidence: string;
    jobId: string | null;
  }>;
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
      counts: Record<string, number>;
      weeklyActivity: WeeklyActivity;
      dataDir: string;
    }>("list_jobs_cmd", { filters: filters ?? null }),

  createJob: (input: CreateJobInput) =>
    call<{ job: Job; company: Company }>("create_job", { input }),

  previewJobUrl: (url: string) => call<JobUrlPreview>("preview_job_url", { url }),

  getJob: (id: string) => call<{ detail: JobDetail }>("get_job", { id }),

  updateJob: (id: string, updates: UpdateJobInput) =>
    call<{ detail: JobDetail }>("update_job_cmd", { id, updates }),

  checkJobPosting: (id: string) =>
    call<{ postingState: string; lastCheckedAt: string; lastCheckResult: string | null }>(
      "check_job_posting",
      { id },
    ),

  listCompanies: () => call<{ companies: CompanyRow[] }>("list_companies"),

  createCompany: (name: string, careersUrl?: string | null) =>
    call<{ company: Company }>("create_company", { name, careersUrl: careersUrl ?? null }),

  createWatch: (companyId: string, provider: WatchProvider, boardSlug: string) =>
    call<{ watch: unknown }>("create_watch", { companyId, provider, boardSlug }),

  deleteWatch: (watchId: string) => call<{ ok: boolean }>("delete_watch", { watchId }),

  syncWatch: (watchId: string) =>
    call<{ ok: boolean; created: number; error?: string }>("sync_watch", { watchId }),

  checkCareers: (companyId: string) =>
    call<{ ok: boolean; changed?: boolean }>("check_careers", { companyId }),

  dismissReview: (reviewId: string) =>
    call<{ ok: boolean }>("dismiss_review", { reviewId }),

  listDocuments: () => call<{ documents: DocumentListItem[] }>("list_documents"),

  importDocument: (input: ImportDocumentInput) =>
    call<{ document: Document; attachment?: unknown }>("import_document", { input }),

  attachDocument: (jobId: string, documentId: string, kind: DocumentKind) =>
    call<{ attachment: unknown }>("attach_document", { jobId, documentId, kind }),

  openDocument: (documentId: string) => call<{ ok: boolean }>("open_document", { documentId }),

  gmailStatus: () => call<GmailStatus>("gmail_status"),

  gmailConfigure: (clientId: string, clientSecret: string, redirectUri: string) =>
    call<{ ok: boolean }>("gmail_configure", { clientId, clientSecret, redirectUri }),

  gmailConnect: () => call<{ url: string }>("gmail_connect"),

  gmailDisconnect: () => call<{ ok: boolean }>("gmail_disconnect"),

  gmailPoll: () => call<{ linked: number; triaged: number }>("gmail_poll"),

  gmailTriage: (matchId: string, jobId: string | null) =>
    call<{ ok: boolean }>("gmail_triage", { matchId, jobId }),

  runJobsCycle: () => call<Record<string, unknown>>("run_jobs_cycle_cmd"),

  getJobsCsvPath: () => call<JobsCsvPathInfo>("get_jobs_csv_path"),

  setJobsCsvPath: (input: SetJobsCsvPathInput) =>
    call<SetJobsCsvPathResult>("set_jobs_csv_path_cmd", { input }),

  resetJobsCsvPath: () => call<SetJobsCsvPathResult>("reset_jobs_csv_path_cmd"),

  pickJobsCsvPath: () => call<string | null>("pick_jobs_csv_path"),

  revealJobsCsvPath: () => call<void>("reveal_jobs_csv_path"),
};

export type JobsCsvPathInfo = {
  path: string;
  isDefault: boolean;
  envOverride: boolean;
  defaultPath: string;
  hasSidecar: boolean;
};

export type SetJobsCsvPathInput = {
  path: string;
  mode: "relocate_export" | "link_with_sidecar" | "link_without_sidecar";
  dryRun?: boolean;
  confirm?: boolean;
  withoutSidecarAction?: "export_overwrite" | "overwrite_editable";
};

export type SetJobsCsvPathResult = {
  path: string;
  action: string;
  envOverride: boolean;
  import?: {
    dryRun: boolean;
    mode: string;
    path: string;
    summary: {
      created: number;
      updated: number;
      unchanged: number;
      conflicts: number;
      skipped: number;
      notesAdded: number;
      missingFromCsv: number;
    };
  };
};
