import { readFileSync } from "node:fs";

import { NextResponse } from "next/server";

import {
  exportJobsCsv,
  getDefaultJobsCsvPath,
  getJobsCsvStatus,
} from "@/lib/jobs/csv-sync";

export const runtime = "nodejs";

export async function GET(request: Request) {
  try {
    const { searchParams } = new URL(request.url);
    const download = searchParams.get("download") === "1";
    const pathParam = searchParams.get("path") ?? undefined;
    const csvPath = pathParam ?? getDefaultJobsCsvPath();

    const exported = exportJobsCsv({ path: csvPath });

    if (download) {
      const body = readFileSync(exported.path, "utf8");
      return new NextResponse(body, {
        status: 200,
        headers: {
          "Content-Type": "text/csv; charset=utf-8",
          "Content-Disposition": 'attachment; filename="jobs.csv"',
        },
      });
    }

    return NextResponse.json({
      ...exported,
      status: getJobsCsvStatus(csvPath),
    });
  } catch (error) {
    return NextResponse.json(
      {
        error: error instanceof Error ? error.message : "Failed to export CSV",
      },
      { status: 500 },
    );
  }
}
