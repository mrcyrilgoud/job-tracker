import { NextResponse } from "next/server";

import type { JobStatus } from "@/lib/db/schema";
import { scheduleExportJobsCsv } from "@/lib/jobs/csv-sync";
import { getJobDetail, updateJob } from "@/lib/jobs/service";

export const runtime = "nodejs";

type Params = { params: Promise<{ id: string }> };

export async function GET(_request: Request, { params }: Params) {
  const { id } = await params;
  const detail = getJobDetail(id);
  if (!detail) {
    return NextResponse.json({ error: "Job not found" }, { status: 404 });
  }
  return NextResponse.json(detail);
}

export async function PATCH(request: Request, { params }: Params) {
  try {
    const { id } = await params;
    const body = (await request.json()) as {
      title?: string;
      companyName?: string;
      status?: JobStatus;
      appliedAt?: string | null;
      notes?: string | null;
      location?: string | null;
      url?: string;
      isNewFromWatch?: boolean;
    };
    const detail = updateJob(id, body);
    scheduleExportJobsCsv();
    return NextResponse.json(detail);
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to update job" },
      { status: 400 },
    );
  }
}
