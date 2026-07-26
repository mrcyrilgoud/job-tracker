import { integer, sqliteTable, text, uniqueIndex } from "drizzle-orm/sqlite-core";

export const jobStatuses = [
  "wishlist",
  "applied",
  "interviewing",
  "offer",
  "rejected",
  "withdrawn",
  "closed",
] as const;

export type JobStatus = (typeof jobStatuses)[number];

export const jobSources = [
  "manual",
  "greenhouse",
  "lever",
  "ashby",
  "careers_page",
] as const;

export type JobSource = (typeof jobSources)[number];

export const postingStates = ["active", "inactive", "unknown"] as const;

export type PostingState = (typeof postingStates)[number];

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

export const companies = sqliteTable("companies", {
  id: text("id").primaryKey(),
  name: text("name").notNull(),
  careersUrl: text("careers_url"),
  createdAt: text("created_at").notNull(),
  updatedAt: text("updated_at").notNull(),
});

export const jobs = sqliteTable(
  "jobs",
  {
    id: text("id").primaryKey(),
    companyId: text("company_id")
      .notNull()
      .references(() => companies.id),
    title: text("title").notNull(),
    url: text("url").notNull(),
    canonicalUrl: text("canonical_url").notNull(),
    sourceExternalId: text("source_external_id"),
    status: text("status").$type<JobStatus>().notNull().default("wishlist"),
    appliedAt: text("applied_at"),
    postingState: text("posting_state")
      .$type<PostingState>()
      .notNull()
      .default("unknown"),
    lastCheckedAt: text("last_checked_at"),
    lastCheckResult: text("last_check_result"),
    source: text("source").$type<JobSource>().notNull().default("manual"),
    notes: text("notes"),
    location: text("location"),
    isNewFromWatch: integer("is_new_from_watch", { mode: "boolean" })
      .notNull()
      .default(false),
    missingFromSyncCount: integer("missing_from_sync_count").notNull().default(0),
    createdAt: text("created_at").notNull(),
    updatedAt: text("updated_at").notNull(),
  },
  (table) => [
    uniqueIndex("jobs_source_external_uidx").on(table.source, table.sourceExternalId),
    uniqueIndex("jobs_canonical_url_uidx").on(table.canonicalUrl),
  ],
);

export const companyWatches = sqliteTable("company_watches", {
  id: text("id").primaryKey(),
  companyId: text("company_id")
    .notNull()
    .references(() => companies.id),
  provider: text("provider").$type<WatchProvider>().notNull(),
  boardSlug: text("board_slug").notNull(),
  lastSyncedAt: text("last_synced_at"),
  consecutiveSyncFailures: integer("consecutive_sync_failures").notNull().default(0),
  lastSyncError: text("last_sync_error"),
  createdAt: text("created_at").notNull(),
  updatedAt: text("updated_at").notNull(),
});

export const jobEvents = sqliteTable("job_events", {
  id: text("id").primaryKey(),
  jobId: text("job_id")
    .notNull()
    .references(() => jobs.id),
  type: text("type").notNull(),
  note: text("note"),
  occurredAt: text("occurred_at").notNull(),
});

export const documents = sqliteTable(
  "documents",
  {
    id: text("id").primaryKey(),
    originalFilename: text("original_filename").notNull(),
    storedFilename: text("stored_filename").notNull(),
    mimeType: text("mime_type").notNull(),
    checksum: text("checksum").notNull(),
    sizeBytes: integer("size_bytes").notNull(),
    importedAt: text("imported_at").notNull(),
  },
  (table) => [uniqueIndex("documents_checksum_uidx").on(table.checksum)],
);

export const jobDocuments = sqliteTable("job_documents", {
  id: text("id").primaryKey(),
  jobId: text("job_id")
    .notNull()
    .references(() => jobs.id),
  documentId: text("document_id")
    .notNull()
    .references(() => documents.id),
  kind: text("kind").$type<DocumentKind>().notNull(),
  usedAt: text("used_at").notNull(),
});

export const emailMatches = sqliteTable(
  "email_matches",
  {
    id: text("id").primaryKey(),
    jobId: text("job_id").references(() => jobs.id),
    gmailMessageId: text("gmail_message_id").notNull(),
    threadId: text("thread_id"),
    subject: text("subject"),
    snippet: text("snippet"),
    fromAddress: text("from_address"),
    receivedAt: text("received_at"),
    confidence: text("confidence").notNull(),
    triageStatus: text("triage_status").notNull().default("pending"),
    createdAt: text("created_at").notNull(),
  },
  (table) => [uniqueIndex("email_matches_message_uidx").on(table.gmailMessageId)],
);

export const careersPageSnapshots = sqliteTable("careers_page_snapshots", {
  id: text("id").primaryKey(),
  companyId: text("company_id")
    .notNull()
    .references(() => companies.id),
  contentHash: text("content_hash").notNull(),
  normalizedText: text("normalized_text").notNull(),
  capturedAt: text("captured_at").notNull(),
});

export const careersPageReviews = sqliteTable("careers_page_reviews", {
  id: text("id").primaryKey(),
  companyId: text("company_id")
    .notNull()
    .references(() => companies.id),
  previousHash: text("previous_hash"),
  currentHash: text("current_hash").notNull(),
  summary: text("summary").notNull(),
  status: text("status").notNull().default("pending"),
  createdAt: text("created_at").notNull(),
});

export const appSettings = sqliteTable("app_settings", {
  key: text("key").primaryKey(),
  value: text("value").notNull(),
  updatedAt: text("updated_at").notNull(),
});

export type Company = typeof companies.$inferSelect;
export type Job = typeof jobs.$inferSelect;
export type CompanyWatch = typeof companyWatches.$inferSelect;
export type JobEvent = typeof jobEvents.$inferSelect;
export type Document = typeof documents.$inferSelect;
export type JobDocument = typeof jobDocuments.$inferSelect;
export type EmailMatch = typeof emailMatches.$inferSelect;
