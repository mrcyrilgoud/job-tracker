import { NextResponse } from "next/server";

import {
  beginGmailOAuth,
  disconnectGmail,
  getGmailConfig,
  isGmailConnected,
  listPendingEmailMatches,
  pollGmailMatches,
  saveGmailConfig,
} from "@/lib/gmail/client";

export const runtime = "nodejs";

export async function GET() {
  const connected = await isGmailConnected();
  const config = getGmailConfig();
  return NextResponse.json({
    connected,
    configured: Boolean(config.clientId && config.clientSecret),
    redirectUri: config.redirectUri,
    pending: listPendingEmailMatches(),
  });
}

export async function POST(request: Request) {
  try {
    const body = (await request.json()) as {
      action?: "configure" | "connect" | "disconnect" | "poll";
      clientId?: string;
      clientSecret?: string;
      redirectUri?: string;
    };

    switch (body.action) {
      case "configure": {
        if (!body.clientId || !body.clientSecret) {
          return NextResponse.json(
            { error: "clientId and clientSecret are required" },
            { status: 400 },
          );
        }
        saveGmailConfig({
          clientId: body.clientId,
          clientSecret: body.clientSecret,
          redirectUri: body.redirectUri,
        });
        return NextResponse.json({ ok: true });
      }
      case "connect": {
        const { url } = await beginGmailOAuth();
        return NextResponse.json({ url });
      }
      case "disconnect": {
        await disconnectGmail();
        return NextResponse.json({ ok: true });
      }
      case "poll": {
        const result = await pollGmailMatches();
        return NextResponse.json(result);
      }
      default: {
        const _exhaustive: never = body.action as never;
        void _exhaustive;
        return NextResponse.json({ error: "Unknown action" }, { status: 400 });
      }
    }
  } catch (error) {
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Gmail action failed" },
      { status: 400 },
    );
  }
}
