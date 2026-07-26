import { GmailClient } from "@/components/gmail-client";
import { getGmailConfig, isGmailConnected, listPendingEmailMatches } from "@/lib/gmail/client";
import { listJobs } from "@/lib/jobs/service";

export const dynamic = "force-dynamic";

export default async function GmailPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const params = await searchParams;
  const connected = await isGmailConnected();
  const config = getGmailConfig();
  const pending = listPendingEmailMatches();
  const jobs = listJobs().map(({ job, companyName }) => ({
    id: job.id,
    label: `${job.title} · ${companyName}`,
  }));

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">Gmail</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Connect your inbox and Job Tracker will quietly match application emails to the right
          job. It only ever reads, and everything stays on your Mac.
        </p>
      </div>
      <GmailClient
        connected={connected}
        configured={Boolean(config.clientId && config.clientSecret)}
        redirectUri={config.redirectUri}
        pending={pending}
        jobs={jobs}
        initialError={typeof params.error === "string" ? params.error : undefined}
        justConnected={params.connected === "1"}
      />
    </div>
  );
}
