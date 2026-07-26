import { useState } from "react";

import { api } from "@/lib/api";

type PendingMatch = {
  id: string;
  subject: string | null;
  snippet: string | null;
  fromAddress: string | null;
  confidence: string;
  jobId: string | null;
};

export function GmailClient({
  connected,
  configured,
  redirectUri,
  pending,
  jobs,
  initialError,
  justConnected,
  onChanged,
}: {
  connected: boolean;
  configured: boolean;
  redirectUri: string;
  pending: PendingMatch[];
  jobs: Array<{ id: string; label: string }>;
  initialError?: string;
  justConnected?: boolean;
  onChanged: () => void;
}) {
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [message, setMessage] = useState(
    justConnected ? "Gmail connected — you're all set." : null,
  );
  const [error, setError] = useState(initialError ?? null);
  const [busy, setBusy] = useState(false);

  async function configure() {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await api.gmailConfigure(clientId, clientSecret, redirectUri);
      setMessage("Credentials saved on your Mac");
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Configure failed");
    } finally {
      setBusy(false);
    }
  }

  async function connect() {
    setBusy(true);
    setError(null);
    try {
      const result = await api.gmailConnect();
      window.location.href = result.url;
    } catch (err) {
      setError(err instanceof Error ? err.message : "Connect failed");
      setBusy(false);
    }
  }

  async function disconnect() {
    setBusy(true);
    setError(null);
    try {
      await api.gmailDisconnect();
      setMessage("Gmail disconnected and credentials removed.");
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Disconnect failed");
    } finally {
      setBusy(false);
    }
  }

  async function poll() {
    setBusy(true);
    setError(null);
    try {
      const data = await api.gmailPoll();
      setMessage(
        `Checked your inbox — ${data.linked} matched, ${data.triaged} need a quick look`,
      );
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Poll failed");
    } finally {
      setBusy(false);
    }
  }

  async function triage(matchId: string, jobId: string | null) {
    setBusy(true);
    setError(null);
    try {
      await api.gmailTriage(matchId, jobId);
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Triage failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-4">
      <div className="card flex flex-wrap items-center justify-between gap-3 p-5">
        <div className="flex items-center gap-2.5">
          <span
            className={`inline-block h-2.5 w-2.5 rounded-full ${
              connected ? "bg-[var(--green-ink)]" : "bg-[var(--faint)]"
            }`}
          />
          <span className="font-medium">
            {connected ? "Connected to Gmail" : "Not connected"}
          </span>
        </div>
        {connected ? (
          <div className="flex gap-2">
            <button type="button" onClick={() => void poll()} disabled={busy} className="btn btn-primary">
              Check inbox now
            </button>
            <button
              type="button"
              onClick={() => void disconnect()}
              disabled={busy}
              className="btn btn-secondary"
            >
              Disconnect
            </button>
          </div>
        ) : configured ? (
          <button type="button" onClick={() => void connect()} disabled={busy} className="btn btn-primary">
            Connect Gmail
          </button>
        ) : null}
      </div>

      {!configured ? (
        <div className="card space-y-3 p-6">
          <h3 className="font-display text-lg font-medium">Set up access</h3>
          <p className="text-sm text-[var(--muted)]">
            Paste the OAuth client ID and secret from your Google Cloud project. They&apos;re
            stored locally and never leave your Mac.
          </p>
          <input
            value={clientId}
            onChange={(e) => setClientId(e.target.value)}
            placeholder="Google OAuth client ID"
            className="field"
          />
          <input
            value={clientSecret}
            onChange={(e) => setClientSecret(e.target.value)}
            placeholder="Google OAuth client secret"
            className="field"
          />
          <button
            type="button"
            onClick={() => void configure()}
            disabled={busy || !clientId || !clientSecret}
            className="btn btn-primary"
          >
            Save credentials
          </button>
          <details className="text-sm text-[var(--muted)]">
            <summary className="cursor-pointer font-medium text-[var(--foreground)]">
              Technical details
            </summary>
            <div className="mt-2 space-y-1">
              <p>Read-only access via OAuth with PKCE. Refresh token stored in macOS Keychain.</p>
              <p>
                Redirect URI for Google Cloud:{" "}
                <code className="rounded bg-[var(--surface-muted)] px-1.5 py-0.5 text-xs">
                  {redirectUri}
                </code>
              </p>
            </div>
          </details>
        </div>
      ) : null}

      {message ? (
        <p className="rounded-xl bg-[var(--green-soft)] px-3.5 py-2.5 text-sm text-[var(--green-ink)]">
          {message}
        </p>
      ) : null}
      {error ? (
        <p className="rounded-xl bg-[var(--danger-soft)] px-3.5 py-2.5 text-sm text-[var(--danger)]">
          {error}
        </p>
      ) : null}

      <section className="card p-6">
        <h3 className="mb-1 font-display text-lg font-medium">Needs a quick look</h3>
        <p className="mb-4 text-sm text-[var(--muted)]">
          We weren&apos;t sure which job these emails belong to.
        </p>
        {pending.length === 0 ? (
          <p className="text-sm text-[var(--faint)]">Nothing waiting — you&apos;re all caught up.</p>
        ) : (
          <ul className="space-y-3">
            {pending.map((match) => (
              <li key={match.id} className="rounded-xl bg-[var(--surface-muted)] p-4">
                <div className="flex items-start justify-between gap-2">
                  <p className="font-medium">{match.subject ?? "(no subject)"}</p>
                  <span className="pill bg-[var(--amber-soft)] text-[var(--amber-ink)]">
                    {match.confidence} confidence
                  </span>
                </div>
                <p className="text-sm text-[var(--faint)]">{match.fromAddress}</p>
                {match.snippet ? (
                  <p className="mt-1 text-sm text-[var(--muted)]">{match.snippet}</p>
                ) : null}
                <div className="mt-3 flex flex-wrap gap-2">
                  <select
                    defaultValue={match.jobId ?? ""}
                    id={`job-${match.id}`}
                    className="field w-auto min-w-[180px]"
                  >
                    <option value="">Which job?</option>
                    {jobs.map((job) => (
                      <option key={job.id} value={job.id}>
                        {job.label}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => {
                      const select = document.getElementById(
                        `job-${match.id}`,
                      ) as HTMLSelectElement | null;
                      const value = select?.value || null;
                      void triage(match.id, value);
                    }}
                    className="btn btn-primary"
                  >
                    Link it
                  </button>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => void triage(match.id, null)}
                    className="btn btn-secondary"
                  >
                    Not relevant
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
