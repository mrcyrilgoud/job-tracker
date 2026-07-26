import { NextResponse } from "next/server";

import { completeGmailOAuth } from "@/lib/gmail/client";

export const runtime = "nodejs";

export async function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const code = searchParams.get("code");
  const state = searchParams.get("state");
  const error = searchParams.get("error");

  if (error) {
    return NextResponse.redirect(new URL(`/gmail?error=${encodeURIComponent(error)}`, request.url));
  }

  if (!code || !state) {
    return NextResponse.redirect(new URL("/gmail?error=missing_code", request.url));
  }

  try {
    await completeGmailOAuth({ code, state });
    return NextResponse.redirect(new URL("/gmail?connected=1", request.url));
  } catch (err) {
    const message = err instanceof Error ? err.message : "oauth_failed";
    return NextResponse.redirect(
      new URL(`/gmail?error=${encodeURIComponent(message)}`, request.url),
    );
  }
}
