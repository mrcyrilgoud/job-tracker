import { NextResponse } from "next/server";

import { scheduleExportJobsCsv } from "@/lib/jobs/csv-sync";
import { getPipelineCounts, listJobs } from "@/lib/jobs/service";
import { createJobFromUrl } from "@/lib/jobs/service";

export const runtime = "nodejs";

export async function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const jobs = listJobs({
    status: searchParams.get("status") ?? undefined,
    companyId: searchParams.get("companyId") ?? undefined,
    postingState: searchParams.get("postingState") ?? undefined,
    search: searchParams.get("search") ?? undefined,
    newFromWatch: searchParams.get("newFromWatch") === "1",
  });

  return NextResponse.json({ jobs, counts: getPipelineCounts() });
}

export async function POST(request: Request) {
  try {
    const body = (await request.json()) as {
      url?: string;
      title?: string;
      companyName?: string;
      status?: string;
      appliedAt?: string | null;
      notes?: string | null;
    };

    if (!body.url) {
      return NextResponse.json({ error: "URL is required" }, { status: 400 });
    }

    const created = await createJobFromUrl({
      url: body.url,
      title: body.title,
      companyName: body.companyName,
      status: body.status as never,
      appliedAt: body.appliedAt,
      notes: body.notes,
    });

    scheduleExportJobsCsv();
    return NextResponse.json(created, { status: 201 });
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to create job" },
      { status: 400 },
    );
  }
}
