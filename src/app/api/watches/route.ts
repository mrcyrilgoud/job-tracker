import { NextResponse } from "next/server";

import { createWatch, deleteWatch, dismissCareersReview } from "@/lib/companies/service";
import type { WatchProvider } from "@/lib/db/schema";
import { syncCompanyWatch } from "@/lib/ats/sync";
import { checkCareersPage } from "@/lib/ats/careers-page";

export const runtime = "nodejs";

export async function POST(request: Request) {
  try {
    const body = (await request.json()) as {
      action?: "create_watch" | "delete_watch" | "sync_watch" | "check_careers" | "dismiss_review";
      companyId?: string;
      provider?: WatchProvider;
      boardSlug?: string;
      watchId?: string;
      reviewId?: string;
    };

    switch (body.action) {
      case "create_watch": {
        if (!body.companyId || !body.provider || !body.boardSlug) {
          return NextResponse.json(
            { error: "companyId, provider, and boardSlug are required" },
            { status: 400 },
          );
        }
        const watch = await createWatch({
          companyId: body.companyId,
          provider: body.provider,
          boardSlug: body.boardSlug,
        });
        return NextResponse.json({ watch }, { status: 201 });
      }
      case "delete_watch": {
        if (!body.watchId) {
          return NextResponse.json({ error: "watchId is required" }, { status: 400 });
        }
        deleteWatch(body.watchId);
        return NextResponse.json({ ok: true });
      }
      case "sync_watch": {
        if (!body.watchId) {
          return NextResponse.json({ error: "watchId is required" }, { status: 400 });
        }
        const result = await syncCompanyWatch(body.watchId);
        return NextResponse.json(result);
      }
      case "check_careers": {
        if (!body.companyId) {
          return NextResponse.json({ error: "companyId is required" }, { status: 400 });
        }
        const result = await checkCareersPage(body.companyId);
        return NextResponse.json(result);
      }
      case "dismiss_review": {
        if (!body.reviewId) {
          return NextResponse.json({ error: "reviewId is required" }, { status: 400 });
        }
        dismissCareersReview(body.reviewId);
        return NextResponse.json({ ok: true });
      }
      default: {
        const _exhaustive: never = body.action as never;
        void _exhaustive;
        return NextResponse.json({ error: "Unknown action" }, { status: 400 });
      }
    }
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Company action failed" },
      { status: 400 },
    );
  }
}
