import { NextResponse } from "next/server";

import { checkJobPosting } from "@/lib/jobs/check-active";

export const runtime = "nodejs";

type Params = { params: Promise<{ id: string }> };

export async function POST(_request: Request, { params }: Params) {
  try {
    const { id } = await params;
    const result = await checkJobPosting(id);
    return NextResponse.json(result);
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Check failed" },
      { status: 400 },
    );
  }
}
