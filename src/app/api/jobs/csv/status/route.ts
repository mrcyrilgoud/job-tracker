import { NextResponse } from "next/server";

import {
  getDefaultJobsCsvPath,
  getJobsCsvStatus,
} from "@/lib/jobs/csv-sync";

export const runtime = "nodejs";

export async function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const pathParam = searchParams.get("path");
  const csvPath = pathParam ?? getDefaultJobsCsvPath();
  return NextResponse.json(getJobsCsvStatus(csvPath));
}
