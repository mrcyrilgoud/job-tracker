import { NextResponse } from "next/server";

import {
  getDefaultJobsCsvPath,
  importJobsCsv,
  type ImportMode,
} from "@/lib/jobs/csv-sync";

export const runtime = "nodejs";

export async function POST(request: Request) {
  try {
    const contentType = request.headers.get("content-type") ?? "";
    let path = getDefaultJobsCsvPath();
    let dryRun = false;
    let mode: ImportMode = "merge";
    let content: string | undefined;

    if (contentType.includes("multipart/form-data")) {
      const form = await request.formData();
      const file = form.get("file");
      const pathValue = form.get("path");
      const dryRunValue = form.get("dryRun");
      const modeValue = form.get("mode");

      if (typeof pathValue === "string" && pathValue.trim()) {
        path = pathValue;
      }
      dryRun = dryRunValue === "1" || dryRunValue === "true";
      if (modeValue === "overwrite_editable" || modeValue === "merge") {
        mode = modeValue;
      }

      if (file instanceof File) {
        content = await file.text();
      }
    } else {
      const body = (await request.json().catch(() => ({}))) as {
        path?: string;
        dryRun?: boolean;
        mode?: ImportMode;
        content?: string;
      };

      if (body.path?.trim()) {
        path = body.path;
      }
      dryRun = Boolean(body.dryRun);
      if (body.mode === "overwrite_editable" || body.mode === "merge") {
        mode = body.mode;
      }
      if (typeof body.content === "string") {
        content = body.content;
      }
    }

    const result = await importJobsCsv({
      path,
      content,
      dryRun,
      mode,
    });

    return NextResponse.json(result);
  } catch (error) {
    return NextResponse.json(
      {
        error: error instanceof Error ? error.message : "Failed to import CSV",
      },
      { status: 400 },
    );
  }
}
