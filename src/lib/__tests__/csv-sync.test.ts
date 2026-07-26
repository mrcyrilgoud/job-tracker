import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { eq } from "drizzle-orm";
import { describe, expect, it } from "vitest";

import { getDb } from "@/lib/db";
import { jobEvents, jobs } from "@/lib/db/schema";
import {
  CSV_HEADERS,
  exportJobsCsv,
  importJobsCsv,
  parseCsv,
} from "@/lib/jobs/csv-sync";
import { createJobFromUrl, updateJob } from "@/lib/jobs/service";

function tempCsvPath(label: string) {
  return path.join(
    os.tmpdir(),
    `job-tracker-csv-${label}-${Date.now()}-${Math.random().toString(16).slice(2)}.csv`,
  );
}

function cleanup(csvPath: string) {
  for (const candidate of [csvPath, `${csvPath}.sync.json`]) {
    if (fs.existsSync(candidate)) {
      fs.unlinkSync(candidate);
    }
  }
}

describe("parseCsv", () => {
  it("handles quoted commas and escaped quotes", () => {
    const rows = parseCsv(`a,b\n"hello, world","she said ""hi"""\n`);
    expect(rows).toEqual([
      ["a", "b"],
      ["hello, world", 'she said "hi"'],
    ]);
  });
});

describe("jobs csv sync", () => {
  it("exports jobs and imports status/notes changes via merge", async () => {
    const csvPath = tempCsvPath("merge");
    const unique = Date.now();

    try {
      const { job } = await createJobFromUrl({
        url: `https://example.com/jobs/csv-merge-${unique}`,
        title: `CSV Merge ${unique}`,
        companyName: `CSV Co ${unique}`,
        status: "wishlist",
      });

      exportJobsCsv({ path: csvPath });
      expect(fs.existsSync(csvPath)).toBe(true);

      const rows = parseCsv(fs.readFileSync(csvPath, "utf8"));
      expect(rows[0]).toEqual(Array.from(CSV_HEADERS));

      const header = rows[0] ?? [];
      const jobRow = rows.find((row) => row[header.indexOf("id")] === job.id);
      expect(jobRow).toBeTruthy();

      const statusIdx = header.indexOf("status");
      const notesIdx = header.indexOf("notes");
      const latestNoteIdx = header.indexOf("latest_note");
      if (!jobRow) {
        throw new Error("missing job row");
      }
      jobRow[statusIdx] = "applied";
      jobRow[notesIdx] = "Edited in spreadsheet";
      jobRow[latestNoteIdx] = "Recruiter screen booked";

      const imported = await importJobsCsv({
        path: csvPath,
        content: `${rows.map((row) =>
          row
            .map((cell) =>
              /[",\n]/.test(cell) ? `"${cell.replaceAll('"', '""')}"` : cell,
            )
            .join(","),
        ).join("\n")}\n`,
        mode: "merge",
      });

      expect(imported.summary.updated).toBe(1);
      expect(imported.summary.notesAdded).toBe(1);
      expect(imported.summary.conflicts).toBe(0);

      const detail = getDb()
        .select()
        .from(jobEvents)
        .where(eq(jobEvents.jobId, job.id))
        .all();
      expect(detail.some((event) => event.type === "status_changed")).toBe(true);
      expect(
        detail.some(
          (event) =>
            event.type === "csv_note" &&
            event.note === "Recruiter screen booked",
        ),
      ).toBe(true);
    } finally {
      cleanup(csvPath);
    }
  });

  it("keeps DB values on dual-change conflicts", async () => {
    const csvPath = tempCsvPath("conflict");
    const unique = Date.now() + 1;

    try {
      const { job } = await createJobFromUrl({
        url: `https://example.com/jobs/csv-conflict-${unique}`,
        title: `CSV Conflict ${unique}`,
        companyName: `Conflict Co ${unique}`,
        notes: "baseline",
      });

      exportJobsCsv({ path: csvPath });

      // App changes notes after export.
      updateJob(job.id, { notes: "changed in app" });

      const rows = parseCsv(fs.readFileSync(csvPath, "utf8"));
      const header = rows[0] ?? [];
      const jobRow = rows.find((row) => row[header.indexOf("id")] === job.id);
      if (!jobRow) {
        throw new Error("missing job row");
      }
      jobRow[header.indexOf("notes")] = "changed in csv";

      const csvText = `${rows
        .map((row) =>
          row
            .map((cell) =>
              /[",\n]/.test(cell) ? `"${cell.replaceAll('"', '""')}"` : cell,
            )
            .join(","),
        )
        .join("\n")}\n`;

      const imported = await importJobsCsv({
        path: csvPath,
        content: csvText,
        mode: "merge",
      });

      expect(imported.summary.conflicts).toBe(1);
      expect(imported.conflicts[0]?.fields).toContain("notes");

      const refreshed = getDb()
        .select()
        .from(jobs)
        .where(eq(jobs.id, job.id))
        .get();
      expect(refreshed?.notes).toBe("changed in app");
    } finally {
      cleanup(csvPath);
    }
  });

  it("creates a job from a CSV row without id", async () => {
    const csvPath = tempCsvPath("create");
    const unique = Date.now() + 2;

    try {
      exportJobsCsv({ path: csvPath });

      const header = Array.from(CSV_HEADERS).join(",");
      const url = `https://example.com/jobs/csv-create-${unique}`;
      const row = [
        "",
        "0",
        url,
        "",
        `Created From CSV ${unique}`,
        `Create Co ${unique}`,
        "wishlist",
        "",
        "from csv",
        "Remote",
        "",
        "",
        "",
        "",
      ]
        .map((cell) =>
          /[",\n]/.test(cell) ? `"${cell.replaceAll('"', '""')}"` : cell,
        )
        .join(",");

      const imported = await importJobsCsv({
        path: csvPath,
        content: `${header}\n${row}\n`,
        mode: "merge",
      });

      expect(imported.summary.created).toBe(1);
      expect(imported.errors).toEqual([]);
    } finally {
      cleanup(csvPath);
    }
  });
});
