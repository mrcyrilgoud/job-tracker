import fs from "node:fs";
import path from "node:path";

import { eq } from "drizzle-orm";

import { getDataDir, getDb } from "@/lib/db";
import {
  companies,
  jobStatuses,
  jobs,
  type JobStatus,
} from "@/lib/db/schema";
import {
  addJobEvent,
  createJobFromUrl,
  updateJob,
} from "@/lib/jobs/service";
import { normalizeCanonicalUrl, nowIso } from "@/lib/utils";

const JOBS_CSV_FILENAME = "jobs.csv";

export const CSV_HEADERS = [
  "id",
  "csv_rev",
  "url",
  "canonical_url",
  "title",
  "company",
  "status",
  "applied_at",
  "notes",
  "location",
  "latest_note",
  "source",
  "posting_state",
  "updated_at",
] as const;

type EditableFieldKey =
  | "url"
  | "title"
  | "company"
  | "status"
  | "appliedAt"
  | "notes"
  | "location"
  | "latestNote";

export type EditableFields = {
  url: string;
  title: string;
  company: string;
  status: string;
  appliedAt: string | null;
  notes: string | null;
  location: string | null;
  latestNote: string | null;
};

type SyncRowState = {
  csvRev: number;
  fields: EditableFields;
};

type SyncState = {
  path: string;
  lastExportAt: string | null;
  lastImportAt: string | null;
  rows: Record<string, SyncRowState>;
};

export type ExportResult = {
  path: string;
  exportedAt: string;
  rowCount: number;
};

export type ImportMode = "merge" | "overwrite_editable";

export type ImportResult = {
  dryRun: boolean;
  mode: ImportMode;
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
  conflicts: Array<{
    key: string;
    fields: EditableFieldKey[];
    db: Partial<EditableFields>;
    csv: Partial<EditableFields>;
  }>;
  errors: Array<{ row: number; message: string }>;
  changes: Array<{
    action: "create" | "update" | "note";
    jobId?: string;
    fields?: EditableFieldKey[];
  }>;
};

export type CsvStatus = {
  path: string;
  exists: boolean;
  fileMtime: string | null;
  lastExportAt: string | null;
  lastImportAt: string | null;
  rowCountDb: number;
  rowCountCsv: number | null;
  drift: boolean;
};

const EDITABLE_KEYS: EditableFieldKey[] = [
  "url",
  "title",
  "company",
  "status",
  "appliedAt",
  "notes",
  "location",
  "latestNote",
];

let exportTimer: ReturnType<typeof setTimeout> | null = null;

export function getDefaultJobsCsvPath() {
  return path.join(getDataDir(), JOBS_CSV_FILENAME);
}

function getDefaultJobsCsvSyncPath(csvPath = getDefaultJobsCsvPath()) {
  return `${csvPath}.sync.json`;
}

function emptySyncState(csvPath: string): SyncState {
  return {
    path: csvPath,
    lastExportAt: null,
    lastImportAt: null,
    rows: {},
  };
}

function readSyncState(csvPath: string): SyncState {
  const syncPath = getDefaultJobsCsvSyncPath(csvPath);
  if (!fs.existsSync(syncPath)) {
    return emptySyncState(csvPath);
  }

  try {
    const parsed = JSON.parse(fs.readFileSync(syncPath, "utf8")) as SyncState;
    return {
      path: csvPath,
      lastExportAt: parsed.lastExportAt ?? null,
      lastImportAt: parsed.lastImportAt ?? null,
      rows: parsed.rows ?? {},
    };
  } catch {
    return emptySyncState(csvPath);
  }
}

function writeSyncState(state: SyncState) {
  const syncPath = getDefaultJobsCsvSyncPath(state.path);
  const tempPath = `${syncPath}.${process.pid}.tmp`;
  fs.writeFileSync(tempPath, `${JSON.stringify(state, null, 2)}\n`, "utf8");
  fs.renameSync(tempPath, syncPath);
}

function normalizeNullable(value: string | null | undefined): string | null {
  if (value === undefined || value === null) {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length === 0 ? null : trimmed;
}

function valuesEqual(
  a: string | null | undefined,
  b: string | null | undefined,
): boolean {
  return normalizeNullable(a ?? null) === normalizeNullable(b ?? null);
}

function escapeCsvField(value: string): string {
  if (/[",\r\n]/.test(value)) {
    return `"${value.replaceAll('"', '""')}"`;
  }
  return value;
}

function serializeCsv(rows: string[][]): string {
  return `${rows.map((row) => row.map(escapeCsvField).join(",")).join("\n")}\n`;
}

/** Minimal RFC4180 CSV parser (UTF-8 text). */
export function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let inQuotes = false;

  for (let i = 0; i < text.length; i += 1) {
    const char = text[i];
    const next = text[i + 1];

    if (inQuotes) {
      if (char === '"' && next === '"') {
        field += '"';
        i += 1;
      } else if (char === '"') {
        inQuotes = false;
      } else {
        field += char;
      }
      continue;
    }

    if (char === '"') {
      inQuotes = true;
      continue;
    }

    if (char === ",") {
      row.push(field);
      field = "";
      continue;
    }

    if (char === "\n") {
      row.push(field);
      rows.push(row);
      row = [];
      field = "";
      continue;
    }

    if (char === "\r") {
      continue;
    }

    field += char;
  }

  if (field.length > 0 || row.length > 0) {
    row.push(field);
    rows.push(row);
  }

  return rows.filter((r) => r.some((cell) => cell.trim().length > 0));
}

function isJobStatus(value: string): value is JobStatus {
  return (jobStatuses as readonly string[]).includes(value);
}

function loadJobRows() {
  const db = getDb();
  return db
    .select({
      job: jobs,
      companyName: companies.name,
    })
    .from(jobs)
    .innerJoin(companies, eq(jobs.companyId, companies.id))
    .all();
}

function editableFromDb(
  job: typeof jobs.$inferSelect,
  companyName: string,
  latestNote: string | null,
): EditableFields {
  return {
    url: job.url,
    title: job.title,
    company: companyName,
    status: job.status,
    appliedAt: job.appliedAt,
    notes: job.notes,
    location: job.location,
    latestNote,
  };
}

function editableFromCsvRow(
  record: Record<string, string>,
): EditableFields {
  return {
    url: record.url?.trim() ?? "",
    title: record.title?.trim() ?? "",
    company: record.company?.trim() ?? "",
    status: record.status?.trim() ?? "",
    appliedAt: normalizeNullable(record.applied_at),
    notes: normalizeNullable(record.notes),
    location: normalizeNullable(record.location),
    latestNote: normalizeNullable(record.latest_note),
  };
}

export function exportJobsCsv(options?: {
  path?: string;
  latestNoteOverrides?: Record<string, string | null>;
}): ExportResult {
  const csvPath = options?.path ?? getDefaultJobsCsvPath();
  const sync = readSyncState(csvPath);
  const rows = loadJobRows();
  const latestNoteOverrides = options?.latestNoteOverrides ?? {};
  const nextRows: Record<string, SyncRowState> = {};

  const csvRows: string[][] = [Array.from(CSV_HEADERS)];

  for (const { job, companyName } of rows) {
    const previous = sync.rows[job.id];
    const latestNote =
      latestNoteOverrides[job.id] !== undefined
        ? latestNoteOverrides[job.id]
        : (previous?.fields.latestNote ?? null);
    const fields = editableFromDb(job, companyName, latestNote);
    const csvRev = (previous?.csvRev ?? 0) + 1;
    nextRows[job.id] = { csvRev, fields };

    csvRows.push([
      job.id,
      String(csvRev),
      fields.url,
      job.canonicalUrl,
      fields.title,
      fields.company,
      fields.status,
      fields.appliedAt ?? "",
      fields.notes ?? "",
      fields.location ?? "",
      fields.latestNote ?? "",
      job.source,
      job.postingState,
      job.updatedAt,
    ]);
  }

  const tempPath = `${csvPath}.${process.pid}.tmp`;
  fs.mkdirSync(path.dirname(csvPath), { recursive: true });
  fs.writeFileSync(tempPath, serializeCsv(csvRows), "utf8");
  fs.renameSync(tempPath, csvPath);

  // Use file mtime so drift detection does not fire on our own writes.
  const exportedAt = fs.statSync(csvPath).mtime.toISOString();

  writeSyncState({
    path: csvPath,
    lastExportAt: exportedAt,
    lastImportAt: sync.lastImportAt,
    rows: nextRows,
  });

  return { path: csvPath, exportedAt, rowCount: rows.length };
}

export function scheduleExportJobsCsv(options?: { path?: string; delayMs?: number }) {
  if (exportTimer) {
    clearTimeout(exportTimer);
  }
  exportTimer = setTimeout(() => {
    exportTimer = null;
    try {
      exportJobsCsv(options);
    } catch (error) {
      console.error(
        "[csv-sync] export failed:",
        error instanceof Error ? error.message : error,
      );
    }
  }, options?.delayMs ?? 250);
}

export function getJobsCsvStatus(csvPath = getDefaultJobsCsvPath()): CsvStatus {
  const sync = readSyncState(csvPath);
  const exists = fs.existsSync(csvPath);
  let fileMtime: string | null = null;
  let rowCountCsv: number | null = null;

  if (exists) {
    const stat = fs.statSync(csvPath);
    fileMtime = stat.mtime.toISOString();
    try {
      const parsed = parseCsv(fs.readFileSync(csvPath, "utf8"));
      rowCountCsv = Math.max(0, parsed.length - 1);
    } catch {
      rowCountCsv = null;
    }
  }

  const rowCountDb = loadJobRows().length;
  const drift =
    exists &&
    (sync.lastExportAt === null ||
      (fileMtime !== null && fileMtime > sync.lastExportAt) ||
      (rowCountCsv !== null && rowCountCsv !== rowCountDb));

  return {
    path: csvPath,
    exists,
    fileMtime,
    lastExportAt: sync.lastExportAt,
    lastImportAt: sync.lastImportAt,
    rowCountDb,
    rowCountCsv,
    drift,
  };
}

function headerIndexMap(headerRow: string[]): Map<string, number> {
  const map = new Map<string, number>();
  headerRow.forEach((name, index) => {
    map.set(name.trim().toLowerCase(), index);
  });
  return map;
}

function rowToRecord(
  headerMap: Map<string, number>,
  row: string[],
): Record<string, string> {
  const record: Record<string, string> = {};
  for (const header of CSV_HEADERS) {
    const index = headerMap.get(header);
    record[header] = index === undefined ? "" : (row[index] ?? "");
  }
  return record;
}

function findJobByIdOrCanonical(id: string | null, url: string) {
  const db = getDb();
  if (id) {
    const byId = db.select().from(jobs).where(eq(jobs.id, id)).get();
    if (byId) {
      return byId;
    }
  }

  if (!url.trim()) {
    return null;
  }

  const canonicalUrl = normalizeCanonicalUrl(url);
  return db.select().from(jobs).where(eq(jobs.canonicalUrl, canonicalUrl)).get();
}

function companyNameForJob(companyId: string): string {
  const db = getDb();
  const company = db
    .select()
    .from(companies)
    .where(eq(companies.id, companyId))
    .get();
  return company?.name ?? "Unknown company";
}

function pickConflictFields(
  keys: EditableFieldKey[],
  dbFields: EditableFields,
  csvFields: EditableFields,
): {
  fields: EditableFieldKey[];
  db: Partial<EditableFields>;
  csv: Partial<EditableFields>;
} {
  const db = {} as Partial<EditableFields>;
  const csv = {} as Partial<EditableFields>;
  for (const key of keys) {
    Object.assign(db, { [key]: dbFields[key] });
    Object.assign(csv, { [key]: csvFields[key] });
  }
  return { fields: keys, db, csv };
}

export async function importJobsCsv(options?: {
  path?: string;
  content?: string;
  dryRun?: boolean;
  mode?: ImportMode;
}): Promise<ImportResult> {
  const csvPath = options?.path ?? getDefaultJobsCsvPath();
  const dryRun = options?.dryRun ?? false;
  const mode = options?.mode ?? "merge";
  const sync = readSyncState(csvPath);

  const result: ImportResult = {
    dryRun,
    mode,
    path: csvPath,
    summary: {
      created: 0,
      updated: 0,
      unchanged: 0,
      conflicts: 0,
      skipped: 0,
      notesAdded: 0,
      missingFromCsv: 0,
    },
    conflicts: [],
    errors: [],
    changes: [],
  };

  let text = options?.content;
  if (text === undefined) {
    if (!fs.existsSync(csvPath)) {
      throw new Error(`CSV file not found: ${csvPath}`);
    }
    text = fs.readFileSync(csvPath, "utf8");
  }

  const parsed = parseCsv(text);
  if (parsed.length === 0) {
    throw new Error("CSV is empty");
  }

  const headerMap = headerIndexMap(parsed[0] ?? []);
  if (!headerMap.has("url") || !headerMap.has("title") || !headerMap.has("company")) {
    throw new Error("CSV must include url, title, and company columns");
  }

  const seenJobIds = new Set<string>();
  const importedNotes = new Map<string, string | null>();
  const dataRows = parsed.slice(1);

  for (let i = 0; i < dataRows.length; i += 1) {
    const rowNumber = i + 2;
    const record = rowToRecord(headerMap, dataRows[i] ?? []);
    const csvFields = editableFromCsvRow(record);
    const id = normalizeNullable(record.id);
    const key = id ?? (csvFields.url || `row-${rowNumber}`);

    try {
      const existing = findJobByIdOrCanonical(id, csvFields.url);

      if (!existing) {
        if (!csvFields.url || !csvFields.title || !csvFields.company) {
          result.summary.skipped += 1;
          result.errors.push({
            row: rowNumber,
            message: "New jobs require url, title, and company",
          });
          continue;
        }

        if (csvFields.status && !isJobStatus(csvFields.status)) {
          result.summary.skipped += 1;
          result.errors.push({
            row: rowNumber,
            message: `Invalid status: ${csvFields.status}`,
          });
          continue;
        }

        if (!dryRun) {
          const created = await createJobFromUrl({
            url: csvFields.url,
            title: csvFields.title,
            companyName: csvFields.company,
            status: csvFields.status ? (csvFields.status as JobStatus) : "wishlist",
            appliedAt: csvFields.appliedAt,
            notes: csvFields.notes,
            location: csvFields.location,
          });
          if (csvFields.latestNote) {
            addJobEvent(created.job.id, "csv_note", csvFields.latestNote);
            importedNotes.set(created.job.id, csvFields.latestNote);
            result.summary.notesAdded += 1;
            result.changes.push({
              action: "note",
              jobId: created.job.id,
              fields: ["latestNote"],
            });
          }
          seenJobIds.add(created.job.id);
          result.changes.push({
            action: "create",
            jobId: created.job.id,
            fields: EDITABLE_KEYS.filter((field) => field !== "latestNote"),
          });
        } else {
          result.changes.push({
            action: "create",
            fields: EDITABLE_KEYS.filter((field) => field !== "latestNote"),
          });
        }

        result.summary.created += 1;
        continue;
      }

      seenJobIds.add(existing.id);
      const companyName = companyNameForJob(existing.companyId);
      const baseline =
        sync.rows[existing.id]?.fields ??
        editableFromDb(existing, companyName, null);
      const dbFields = editableFromDb(
        existing,
        companyName,
        baseline.latestNote,
      );

      if (csvFields.status && !isJobStatus(csvFields.status)) {
        result.summary.skipped += 1;
        result.errors.push({
          row: rowNumber,
          message: `Invalid status: ${csvFields.status}`,
        });
        continue;
      }

      const applyFields: EditableFieldKey[] = [];
      const conflictFields: EditableFieldKey[] = [];

      for (const field of EDITABLE_KEYS) {
        if (field === "latestNote") {
          continue;
        }

        const csvChanged = !valuesEqual(csvFields[field], baseline[field]);
        const dbChanged = !valuesEqual(dbFields[field], baseline[field]);

        if (mode === "overwrite_editable") {
          if (!valuesEqual(csvFields[field], dbFields[field])) {
            applyFields.push(field);
          }
          continue;
        }

        if (csvChanged && dbChanged) {
          conflictFields.push(field);
        } else if (csvChanged && !dbChanged) {
          applyFields.push(field);
        }
      }

      if (conflictFields.length > 0) {
        result.summary.conflicts += 1;
        const picked = pickConflictFields(conflictFields, dbFields, csvFields);
        result.conflicts.push({
          key: String(key),
          ...picked,
        });
      }

      const noteChanged =
        mode === "overwrite_editable"
          ? !valuesEqual(csvFields.latestNote, dbFields.latestNote) &&
            csvFields.latestNote !== null
          : !valuesEqual(csvFields.latestNote, baseline.latestNote) &&
            csvFields.latestNote !== null &&
            valuesEqual(dbFields.latestNote, baseline.latestNote);

      const noteConflict =
        mode === "merge" &&
        !valuesEqual(csvFields.latestNote, baseline.latestNote) &&
        !valuesEqual(dbFields.latestNote, baseline.latestNote) &&
        !valuesEqual(csvFields.latestNote, dbFields.latestNote);

      if (noteConflict) {
        result.summary.conflicts += 1;
        result.conflicts.push({
          key: String(key),
          ...pickConflictFields(["latestNote"], dbFields, csvFields),
        });
      }

      if (applyFields.length === 0 && !noteChanged) {
        result.summary.unchanged += 1;
        continue;
      }

      if (applyFields.length > 0) {
        const updates: Parameters<typeof updateJob>[1] = {};
        if (applyFields.includes("url")) updates.url = csvFields.url;
        if (applyFields.includes("title")) updates.title = csvFields.title;
        if (applyFields.includes("company")) {
          updates.companyName = csvFields.company;
        }
        if (applyFields.includes("status")) {
          updates.status = csvFields.status as JobStatus;
        }
        if (applyFields.includes("appliedAt")) {
          updates.appliedAt = csvFields.appliedAt;
        }
        if (applyFields.includes("notes")) updates.notes = csvFields.notes;
        if (applyFields.includes("location")) {
          updates.location = csvFields.location;
        }

        if (!dryRun) {
          updateJob(existing.id, updates);
        }
        result.summary.updated += 1;
        result.changes.push({
          action: "update",
          jobId: existing.id,
          fields: applyFields,
        });
      }

      if (noteChanged && csvFields.latestNote) {
        if (!dryRun) {
          addJobEvent(existing.id, "csv_note", csvFields.latestNote);
          importedNotes.set(existing.id, csvFields.latestNote);
        }
        result.summary.notesAdded += 1;
        result.changes.push({
          action: "note",
          jobId: existing.id,
          fields: ["latestNote"],
        });
      }
    } catch (error) {
      result.summary.skipped += 1;
      result.errors.push({
        row: rowNumber,
        message: error instanceof Error ? error.message : "Import failed",
      });
    }
  }

  for (const jobId of Object.keys(sync.rows)) {
    if (!seenJobIds.has(jobId)) {
      const stillExists = getDb()
        .select()
        .from(jobs)
        .where(eq(jobs.id, jobId))
        .get();
      if (stillExists) {
        result.summary.missingFromCsv += 1;
      }
    }
  }

  if (!dryRun) {
    const noteOverrides = Object.fromEntries(importedNotes.entries());
    const exported = exportJobsCsv({
      path: csvPath,
      latestNoteOverrides: noteOverrides,
    });
    const nextSync = readSyncState(csvPath);
    nextSync.lastImportAt = nowIso();
    nextSync.lastExportAt = exported.exportedAt;
    writeSyncState(nextSync);
  }

  return result;
}

/**
 * Import when the CSV file is newer than the last export, then always export.
 * Used by the background jobs runner.
 */
export async function syncJobsCsvWithDisk(options?: {
  path?: string;
}): Promise<{ imported: ImportResult | null; exported: ExportResult }> {
  const csvPath = options?.path ?? getDefaultJobsCsvPath();
  const status = getJobsCsvStatus(csvPath);
  let imported: ImportResult | null = null;

  if (
    status.exists &&
    (status.lastExportAt === null ||
      (status.fileMtime !== null && status.fileMtime > status.lastExportAt))
  ) {
    imported = await importJobsCsv({ path: csvPath, mode: "merge" });
    return {
      imported,
      exported: {
        path: csvPath,
        exportedAt: readSyncState(csvPath).lastExportAt ?? nowIso(),
        rowCount: loadJobRows().length,
      },
    };
  }

  const exported = exportJobsCsv({ path: csvPath });
  return { imported: null, exported };
}
