import { eq } from "drizzle-orm";
import { CodeChallengeMethod } from "google-auth-library";
import { google } from "googleapis";
import keytar from "keytar";

import { getDb } from "@/lib/db";
import { appSettings, companies, emailMatches, jobEvents, jobs } from "@/lib/db/schema";
import { classifyEmail } from "@/lib/gmail/classify";
import { createId, nowIso } from "@/lib/utils";

const SERVICE = "job-tracker-local";
const ACCOUNT = "gmail-refresh-token";
const CLIENT_ID_KEY = "gmail_client_id";
const CLIENT_SECRET_KEY = "gmail_client_secret";
const REDIRECT_URI_KEY = "gmail_redirect_uri";
const CHECKPOINT_KEY = "gmail_history_checkpoint";
const OAUTH_STATE_KEY = "gmail_oauth_state";
const OAUTH_VERIFIER_KEY = "gmail_oauth_verifier";

function getSetting(key: string) {
  const db = getDb();
  return db.select().from(appSettings).where(eq(appSettings.key, key)).get()?.value ?? null;
}

function setSetting(key: string, value: string) {
  const db = getDb();
  const updatedAt = nowIso();
  const existing = db.select().from(appSettings).where(eq(appSettings.key, key)).get();
  if (existing) {
    db.update(appSettings)
      .set({ value, updatedAt })
      .where(eq(appSettings.key, key))
      .run();
  } else {
    db.insert(appSettings).values({ key, value, updatedAt }).run();
  }
}

function deleteSetting(key: string) {
  const db = getDb();
  db.delete(appSettings).where(eq(appSettings.key, key)).run();
}

export function getGmailConfig() {
  return {
    clientId: process.env.GMAIL_CLIENT_ID ?? getSetting(CLIENT_ID_KEY),
    clientSecret: process.env.GMAIL_CLIENT_SECRET ?? getSetting(CLIENT_SECRET_KEY),
    redirectUri:
      process.env.GMAIL_REDIRECT_URI ??
      getSetting(REDIRECT_URI_KEY) ??
      "http://localhost:3000/api/gmail/callback",
  };
}

export function saveGmailConfig(input: {
  clientId: string;
  clientSecret: string;
  redirectUri?: string;
}) {
  setSetting(CLIENT_ID_KEY, input.clientId);
  setSetting(CLIENT_SECRET_KEY, input.clientSecret);
  if (input.redirectUri) {
    setSetting(REDIRECT_URI_KEY, input.redirectUri);
  }
}

function createOAuthClient() {
  const config = getGmailConfig();
  if (!config.clientId || !config.clientSecret) {
    throw new Error("Gmail OAuth client is not configured");
  }

  return new google.auth.OAuth2(
    config.clientId,
    config.clientSecret,
    config.redirectUri,
  );
}

function base64Url(buffer: Buffer) {
  return buffer
    .toString("base64")
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
}

export async function beginGmailOAuth() {
  const client = createOAuthClient();
  const state = base64Url(Buffer.from(crypto.getRandomValues(new Uint8Array(24))));
  const verifier = base64Url(Buffer.from(crypto.getRandomValues(new Uint8Array(32))));
  const challenge = base64Url(
    Buffer.from(
      await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier)),
    ),
  );

  setSetting(OAUTH_STATE_KEY, state);
  setSetting(OAUTH_VERIFIER_KEY, verifier);

  const url = client.generateAuthUrl({
    access_type: "offline",
    prompt: "consent",
    scope: ["https://www.googleapis.com/auth/gmail.readonly"],
    state,
    code_challenge: challenge,
    code_challenge_method: CodeChallengeMethod.S256,
  });

  return { url, state };
}

export async function completeGmailOAuth(input: {
  code: string;
  state: string;
}) {
  const expectedState = getSetting(OAUTH_STATE_KEY);
  const verifier = getSetting(OAUTH_VERIFIER_KEY);
  if (!expectedState || expectedState !== input.state) {
    throw new Error("Invalid OAuth state");
  }
  if (!verifier) {
    throw new Error("Missing PKCE verifier");
  }

  const client = createOAuthClient();
  const { tokens } = await client.getToken({
    code: input.code,
    codeVerifier: verifier,
  });

  if (!tokens.refresh_token) {
    throw new Error("No refresh token returned. Re-authorize with prompt=consent.");
  }

  await keytar.setPassword(SERVICE, ACCOUNT, tokens.refresh_token);
  deleteSetting(OAUTH_STATE_KEY);
  deleteSetting(OAUTH_VERIFIER_KEY);
  return { connected: true };
}

export async function isGmailConnected() {
  const token = await keytar.getPassword(SERVICE, ACCOUNT);
  return Boolean(token);
}

export async function disconnectGmail() {
  await keytar.deletePassword(SERVICE, ACCOUNT);
}

async function getAuthorizedClient() {
  const refreshToken = await keytar.getPassword(SERVICE, ACCOUNT);
  if (!refreshToken) {
    throw new Error("Gmail is not connected");
  }
  const client = createOAuthClient();
  client.setCredentials({ refresh_token: refreshToken });
  return client;
}

export async function pollGmailMatches() {
  const auth = await getAuthorizedClient();
  const gmail = google.gmail({ version: "v1", auth });
  const db = getDb();
  const checkpoint = getSetting(CHECKPOINT_KEY);
  const afterQuery = checkpoint
    ? `after:${Math.floor(new Date(checkpoint).getTime() / 1000)}`
    : "newer_than:30d";

  const list = await gmail.users.messages.list({
    userId: "me",
    q: afterQuery,
    maxResults: 50,
  });

  const trackedJobs = db
    .select({
      job: jobs,
      companyName: companies.name,
    })
    .from(jobs)
    .innerJoin(companies, eq(jobs.companyId, companies.id))
    .all();

  let linked = 0;
  let triaged = 0;
  let newest = checkpoint ?? "";

  for (const item of list.data.messages ?? []) {
    if (!item.id) continue;
    const existing = db
      .select()
      .from(emailMatches)
      .where(eq(emailMatches.gmailMessageId, item.id))
      .get();
    if (existing) continue;

    const message = await gmail.users.messages.get({
      userId: "me",
      id: item.id,
      format: "metadata",
      metadataHeaders: ["From", "Subject", "Date"],
    });

    const headers = message.data.payload?.headers ?? [];
    const subject = headers.find((h) => h.name === "Subject")?.value ?? "";
    const fromAddress = headers.find((h) => h.name === "From")?.value ?? "";
    const dateHeader = headers.find((h) => h.name === "Date")?.value;
    const receivedAt = dateHeader ? new Date(dateHeader).toISOString() : nowIso();
    const snippet = message.data.snippet ?? "";

    if (!newest || receivedAt > newest) {
      newest = receivedAt;
    }

    let best:
      | {
          jobId: string;
          confidence: "high" | "medium" | "low";
        }
      | null = null;

    for (const row of trackedJobs) {
      const confidence = classifyEmail({
        subject,
        snippet,
        fromAddress,
        companyName: row.companyName,
        jobTitle: row.job.title,
      });
      if (!confidence) continue;
      if (
        !best ||
        (confidence === "high" && best.confidence !== "high") ||
        (confidence === "medium" && best.confidence === "low")
      ) {
        best = { jobId: row.job.id, confidence };
      }
    }

    if (!best) {
      continue;
    }

    const createdAt = nowIso();
    if (best.confidence === "high") {
      db.insert(emailMatches)
        .values({
          id: createId(),
          jobId: best.jobId,
          gmailMessageId: item.id,
          threadId: message.data.threadId ?? null,
          subject,
          snippet,
          fromAddress,
          receivedAt,
          confidence: best.confidence,
          triageStatus: "auto_linked",
          createdAt,
        })
        .run();

      db.insert(jobEvents)
        .values({
          id: createId(),
          jobId: best.jobId,
          type: "email_update",
          note: `Gmail: ${subject}`,
          occurredAt: receivedAt,
        })
        .run();
      linked += 1;
    } else {
      db.insert(emailMatches)
        .values({
          id: createId(),
          jobId: best.jobId,
          gmailMessageId: item.id,
          threadId: message.data.threadId ?? null,
          subject,
          snippet,
          fromAddress,
          receivedAt,
          confidence: best.confidence,
          triageStatus: "pending",
          createdAt,
        })
        .run();
      triaged += 1;
    }
  }

  if (newest) {
    setSetting(CHECKPOINT_KEY, newest);
  }

  return { linked, triaged, checkpoint: newest || null };
}

export function confirmEmailMatch(matchId: string, jobId: string | null) {
  const db = getDb();
  const match = db.select().from(emailMatches).where(eq(emailMatches.id, matchId)).get();
  if (!match) {
    throw new Error("Email match not found");
  }

  if (!jobId) {
    db.update(emailMatches)
      .set({ triageStatus: "dismissed", jobId: null })
      .where(eq(emailMatches.id, matchId))
      .run();
    return { dismissed: true };
  }

  db.update(emailMatches)
    .set({ triageStatus: "confirmed", jobId })
    .where(eq(emailMatches.id, matchId))
    .run();

  db.insert(jobEvents)
    .values({
      id: createId(),
      jobId,
      type: "email_update",
      note: `Gmail (confirmed): ${match.subject ?? "Update"}`,
      occurredAt: match.receivedAt ?? nowIso(),
    })
    .run();

  return { confirmed: true };
}

export function listPendingEmailMatches() {
  const db = getDb();
  return db
    .select()
    .from(emailMatches)
    .where(eq(emailMatches.triageStatus, "pending"))
    .all();
}
