import { NextResponse } from "next/server";

import { confirmEmailMatch } from "@/lib/gmail/client";

export const runtime = "nodejs";

export async function POST(request: Request) {
  try {
    const body = (await request.json()) as {
      matchId?: string;
      jobId?: string | null;
    };
    if (!body.matchId) {
      return NextResponse.json({ error: "matchId is required" }, { status: 400 });
    }
    const result = confirmEmailMatch(body.matchId, body.jobId ?? null);
    return NextResponse.json(result);
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Triage failed" },
      { status: 400 },
    );
  }
}
