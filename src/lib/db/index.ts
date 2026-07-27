import Database from "better-sqlite3";
import { drizzle } from "drizzle-orm/better-sqlite3";
import fs from "node:fs";
import path from "node:path";

import * as schema from "./schema";

/**
 * Shared-path mode during Tauri migration: honor JOB_TRACKER_DATA_DIR so Next
 * and the desktop app read/write the same SQLite tree (default: repo `data/`).
 */
function resolveDataDir() {
  const fromEnv = process.env.JOB_TRACKER_DATA_DIR?.trim();
  if (fromEnv) {
    return path.resolve(fromEnv);
  }
  return path.join(process.cwd(), "data");
}

let DATA_DIR = resolveDataDir();
let DB_PATH = path.join(DATA_DIR, "job-tracker.db");
let DOCUMENTS_DIR = path.join(DATA_DIR, "documents");

function refreshPaths() {
  DATA_DIR = resolveDataDir();
  DB_PATH = path.join(DATA_DIR, "job-tracker.db");
  DOCUMENTS_DIR = path.join(DATA_DIR, "documents");
}

function ensureDataDirs() {
  refreshPaths();
  fs.mkdirSync(DATA_DIR, { recursive: true });
  fs.mkdirSync(DOCUMENTS_DIR, { recursive: true });
}

function migrate(sqlite: Database.Database) {
  sqlite.exec(`
    CREATE TABLE IF NOT EXISTS companies (
      id TEXT PRIMARY KEY NOT NULL,
      name TEXT NOT NULL,
      careers_url TEXT,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS jobs (
      id TEXT PRIMARY KEY NOT NULL,
      company_id TEXT NOT NULL REFERENCES companies(id),
      title TEXT NOT NULL,
      url TEXT NOT NULL,
      canonical_url TEXT NOT NULL,
      source_external_id TEXT,
      status TEXT NOT NULL DEFAULT 'wishlist',
      applied_at TEXT,
      posting_state TEXT NOT NULL DEFAULT 'unknown',
      last_checked_at TEXT,
      last_check_result TEXT,
      source TEXT NOT NULL DEFAULT 'manual',
      notes TEXT,
      location TEXT,
      is_new_from_watch INTEGER NOT NULL DEFAULT 0,
      missing_from_sync_count INTEGER NOT NULL DEFAULT 0,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE UNIQUE INDEX IF NOT EXISTS jobs_canonical_url_uidx ON jobs(canonical_url);
    CREATE UNIQUE INDEX IF NOT EXISTS jobs_source_external_uidx ON jobs(source, source_external_id)
      WHERE source_external_id IS NOT NULL;

    CREATE TABLE IF NOT EXISTS company_watches (
      id TEXT PRIMARY KEY NOT NULL,
      company_id TEXT NOT NULL REFERENCES companies(id),
      provider TEXT NOT NULL,
      board_slug TEXT NOT NULL,
      last_synced_at TEXT,
      consecutive_sync_failures INTEGER NOT NULL DEFAULT 0,
      last_sync_error TEXT,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS job_events (
      id TEXT PRIMARY KEY NOT NULL,
      job_id TEXT NOT NULL REFERENCES jobs(id),
      type TEXT NOT NULL,
      note TEXT,
      occurred_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS documents (
      id TEXT PRIMARY KEY NOT NULL,
      original_filename TEXT NOT NULL,
      stored_filename TEXT NOT NULL,
      mime_type TEXT NOT NULL,
      checksum TEXT NOT NULL,
      size_bytes INTEGER NOT NULL,
      imported_at TEXT NOT NULL
    );

    CREATE UNIQUE INDEX IF NOT EXISTS documents_checksum_uidx ON documents(checksum);

    CREATE TABLE IF NOT EXISTS job_documents (
      id TEXT PRIMARY KEY NOT NULL,
      job_id TEXT NOT NULL REFERENCES jobs(id),
      document_id TEXT NOT NULL REFERENCES documents(id),
      kind TEXT NOT NULL,
      used_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS email_matches (
      id TEXT PRIMARY KEY NOT NULL,
      job_id TEXT REFERENCES jobs(id),
      gmail_message_id TEXT NOT NULL,
      thread_id TEXT,
      subject TEXT,
      snippet TEXT,
      from_address TEXT,
      received_at TEXT,
      confidence TEXT NOT NULL,
      triage_status TEXT NOT NULL DEFAULT 'pending',
      created_at TEXT NOT NULL
    );

    CREATE UNIQUE INDEX IF NOT EXISTS email_matches_message_uidx ON email_matches(gmail_message_id);

    CREATE TABLE IF NOT EXISTS careers_page_snapshots (
      id TEXT PRIMARY KEY NOT NULL,
      company_id TEXT NOT NULL REFERENCES companies(id),
      content_hash TEXT NOT NULL,
      normalized_text TEXT NOT NULL,
      captured_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS careers_page_reviews (
      id TEXT PRIMARY KEY NOT NULL,
      company_id TEXT NOT NULL REFERENCES companies(id),
      previous_hash TEXT,
      current_hash TEXT NOT NULL,
      summary TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'pending',
      created_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS app_settings (
      key TEXT PRIMARY KEY NOT NULL,
      value TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );
  `);
}

let dbInstance: ReturnType<typeof drizzle<typeof schema>> | null = null;
let sqliteInstance: Database.Database | null = null;

function getSqlite() {
  if (sqliteInstance) {
    return sqliteInstance;
  }

  ensureDataDirs();
  const sqlite = new Database(DB_PATH);
  sqlite.pragma("journal_mode = WAL");
  sqlite.pragma("busy_timeout = 5000");
  sqlite.pragma("foreign_keys = ON");
  migrate(sqlite);
  sqliteInstance = sqlite;
  return sqlite;
}

export function getDb() {
  if (dbInstance) {
    return dbInstance;
  }

  dbInstance = drizzle(getSqlite(), { schema });
  return dbInstance;
}

export function getDocumentsDir() {
  ensureDataDirs();
  return DOCUMENTS_DIR;
}

export function getDataDir() {
  ensureDataDirs();
  return DATA_DIR;
}
