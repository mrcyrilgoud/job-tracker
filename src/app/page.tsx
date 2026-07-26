import { formatDistanceToNow } from "date-fns";
import Link from "next/link";

import {
  BriefcaseIcon,
  ChatIcon,
  SendIcon,
  statusIcons,
  TrophyIcon,
} from "@/components/icons";
import { listCompanies } from "@/lib/companies/service";
import { jobStatuses } from "@/lib/db/schema";
import {
  getPipelineCounts,
  getWeeklyActivity,
  listJobs,
  type WeeklyActivity,
} from "@/lib/jobs/service";
import {
  jobSourceLabel,
  jobStatusPresentation,
  postingStatePresentation,
  toneClasses,
} from "@/lib/ui";

export const dynamic = "force-dynamic";

export default async function HomePage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const params = await searchParams;
  const status = typeof params.status === "string" ? params.status : undefined;
  const companyId =
    typeof params.companyId === "string" ? params.companyId : undefined;
  const postingState =
    typeof params.postingState === "string" ? params.postingState : undefined;
  const search = typeof params.search === "string" ? params.search : undefined;

  const jobs = listJobs({ status, companyId, postingState, search });
  const counts = getPipelineCounts();
  const activity = getWeeklyActivity();
  const companies = listCompanies();
  const newFromWatch = listJobs({ newFromWatch: true });

  const activeStatus = jobStatuses.find((value) => value === status);
  const heading = activeStatus ? jobStatusPresentation(activeStatus).label : "All jobs";
  const isFiltered = Boolean(status || companyId || postingState || search);

  return (
    <div className="grid gap-8 lg:grid-cols-[240px_1fr]">
      <aside className="space-y-6">
        <section className="card p-5">
          <h2 className="mb-4 text-sm font-semibold text-[var(--muted)]">Pipeline</h2>
          <div className="space-y-1">
            <SidebarLink href="/" active={!status} label="All jobs" count={counts.all} />
            {(["wishlist", "applied", "interviewing", "offer"] as const).map((key) => (
              <SidebarLink
                key={key}
                href={`/?status=${key}`}
                active={status === key}
                label={jobStatusPresentation(key).label}
                count={counts[key] ?? 0}
              />
            ))}
          </div>
        </section>

        <section className="card p-5">
          <div className="mb-4 flex items-center justify-between">
            <h2 className="text-sm font-semibold text-[var(--muted)]">Companies</h2>
            <Link href="/companies" className="text-xs font-medium text-[var(--accent)]">
              Manage
            </Link>
          </div>
          <div className="space-y-1">
            {companies.map((company) => (
              <SidebarLink
                key={company.id}
                href={`/?companyId=${company.id}`}
                active={companyId === company.id}
                label={company.name}
              />
            ))}
            {companies.length === 0 ? (
              <p className="text-sm text-[var(--faint)]">No companies yet.</p>
            ) : null}
          </div>
        </section>
      </aside>

      <section className="space-y-5">
        <div className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
          <div>
            <h1 className="font-display text-3xl font-semibold tracking-tight">
              {heading}
            </h1>
            <p className="mt-1 text-sm text-[var(--muted)]">
              {jobs.length === 0
                ? "Nothing here yet."
                : `${jobs.length} ${jobs.length === 1 ? "role" : "roles"} on your radar.`}
            </p>
          </div>
          <Link href="/jobs/new" className="btn btn-primary self-start md:self-auto">
            Add a job
          </Link>
        </div>

        {!isFiltered ? (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <StatCard
              label="Tracking"
              value={counts.all}
              Icon={BriefcaseIcon}
              tint="bg-[var(--accent-soft)] text-[var(--accent-ink)]"
            />
            <StatCard
              label="Applied"
              value={counts.applied ?? 0}
              Icon={SendIcon}
              tint="bg-[var(--blue-soft)] text-[var(--blue-ink)]"
            />
            <StatCard
              label="Interviewing"
              value={counts.interviewing ?? 0}
              Icon={ChatIcon}
              tint="bg-[var(--amber-soft)] text-[var(--amber-ink)]"
            />
            <StatCard
              label="Offers"
              value={counts.offer ?? 0}
              Icon={TrophyIcon}
              tint="bg-[var(--green-soft)] text-[var(--green-ink)]"
            />
          </div>
        ) : null}

        {!isFiltered ? <ActivityLine activity={activity} /> : null}

        <form className="card flex flex-wrap items-center gap-2 p-3">
          <input
            name="search"
            defaultValue={search}
            placeholder="Search roles or companies…"
            className="field min-w-[200px] flex-1 border-transparent bg-[var(--surface-muted)]"
          />
          <select
            name="postingState"
            defaultValue={postingState ?? ""}
            className="field w-auto border-transparent bg-[var(--surface-muted)]"
          >
            <option value="">Any posting</option>
            <option value="active">Open</option>
            <option value="inactive">Closed</option>
            <option value="unknown">Not checked</option>
          </select>
          {status ? <input type="hidden" name="status" value={status} /> : null}
          {companyId ? (
            <input type="hidden" name="companyId" value={companyId} />
          ) : null}
          <button className="btn btn-secondary">Filter</button>
        </form>

        {newFromWatch.length > 0 ? (
          <div className="rounded-2xl bg-[var(--accent-soft)] p-5">
            <h3 className="mb-3 font-display text-lg font-medium text-[var(--accent-ink)]">
              New from your watches
            </h3>
            <div className="space-y-2.5">
              {newFromWatch.slice(0, 5).map(({ job, companyName }) => (
                <div
                  key={job.id}
                  className="flex flex-wrap items-center justify-between gap-2 text-sm"
                >
                  <div>
                    <span className="font-medium">{job.title}</span>
                    <span className="text-[var(--muted)]"> · {companyName}</span>
                    {job.location ? (
                      <span className="text-[var(--muted)]"> · {job.location}</span>
                    ) : null}
                  </div>
                  <Link
                    href={`/jobs/${job.id}`}
                    className="font-medium text-[var(--accent-ink)] hover:underline"
                  >
                    Take a look →
                  </Link>
                </div>
              ))}
            </div>
          </div>
        ) : null}

        {jobs.length === 0 ? (
          <div className="card flex flex-col items-center gap-3 px-6 py-16 text-center">
            <p className="font-display text-lg text-[var(--foreground)]">
              Your board is empty
            </p>
            <p className="max-w-sm text-sm text-[var(--muted)]">
              Paste a posting URL and Job Tracker will keep an eye on it for you.
            </p>
            <Link href="/jobs/new" className="btn btn-primary mt-1">
              Add your first job
            </Link>
          </div>
        ) : (
          <ul className="space-y-3">
            {jobs.map(({ job, companyName }) => {
              const statusInfo = jobStatusPresentation(job.status);
              const postingInfo = postingStatePresentation(job.postingState);
              const source = jobSourceLabel(job.source);
              const StageIcon = statusIcons[job.status];
              return (
                <li key={job.id}>
                  <Link
                    href={`/jobs/${job.id}`}
                    className="card block p-5 transition-shadow hover:shadow-[var(--shadow-md)]"
                  >
                    <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
                      <div className="space-y-2">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className={`pill ${toneClasses[statusInfo.tone]}`}>
                            <StageIcon size={12} />
                            {statusInfo.label}
                          </span>
                          <span className={`pill ${toneClasses[postingInfo.tone]}`}>
                            <span className="pill-dot" />
                            {postingInfo.label}
                          </span>
                          {job.isNewFromWatch ? (
                            <span className="pill bg-[var(--accent-soft)] text-[var(--accent-ink)]">
                              New
                            </span>
                          ) : null}
                        </div>
                        <p className="font-display text-lg font-medium leading-snug">
                          {job.title}
                        </p>
                        <p className="text-sm text-[var(--muted)]">
                          {companyName}
                          {job.appliedAt
                            ? ` · Applied ${formatDistanceToNow(new Date(job.appliedAt), {
                                addSuffix: true,
                              })}`
                            : ""}
                        </p>
                        {source ? (
                          <p className="text-xs text-[var(--faint)]">{source}</p>
                        ) : null}
                      </div>
                      <p className="shrink-0 text-xs text-[var(--faint)]">
                        Updated{" "}
                        {formatDistanceToNow(new Date(job.updatedAt), { addSuffix: true })}
                      </p>
                    </div>
                  </Link>
                </li>
              );
            })}
          </ul>
        )}
      </section>
    </div>
  );
}

function ActivityLine({ activity }: { activity: WeeklyActivity }) {
  const peak = Math.max(1, ...activity.days.map((day) => day.count));
  return (
    <div className="card flex items-center justify-between gap-6 px-5 py-3">
      <div className="flex items-baseline gap-2">
        <span className="text-sm text-[var(--muted)]">This week</span>
        <span className="font-display text-lg leading-none">{activity.total}</span>
        <span className="text-xs text-[var(--faint)]">
          {activity.total === 1 ? "update" : "updates"}
        </span>
      </div>
      <div className="flex h-9 items-end gap-1.5">
        {activity.days.map((day, index) => {
          const height = day.count === 0 ? 4 : 8 + Math.round((day.count / peak) * 20);
          return (
            <span
              key={day.key}
              className="flex flex-col items-center gap-1"
              title={`${day.count} ${day.count === 1 ? "update" : "updates"}`}
            >
              <span
                className={`w-1.5 rounded-full ${
                  day.count > 0 ? "bg-[var(--accent)]" : "bg-[var(--border)]"
                } ${day.isToday && day.count === 0 ? "bg-[var(--accent-soft)]" : ""}`}
                style={{ height }}
              />
              <span
                className={`text-[9px] leading-none ${
                  day.isToday
                    ? "font-semibold text-[var(--accent-ink)]"
                    : "text-[var(--faint)]"
                }`}
              >
                {day.label}
                <span className="sr-only">
                  {" "}
                  day {index + 1} of 7
                </span>
              </span>
            </span>
          );
        })}
      </div>
    </div>
  );
}

function StatCard({
  label,
  value,
  Icon,
  tint,
}: {
  label: string;
  value: number;
  Icon: (props: { size?: number; className?: string }) => React.JSX.Element;
  tint: string;
}) {
  return (
    <div className="card flex items-center gap-3 p-4">
      <span
        className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-xl ${tint}`}
      >
        <Icon size={18} />
      </span>
      <div className="min-w-0">
        <div className="font-display text-2xl leading-none">{value}</div>
        <div className="mt-1 truncate text-xs text-[var(--muted)]">{label}</div>
      </div>
    </div>
  );
}

function SidebarLink({
  href,
  label,
  count,
  active,
}: {
  href: string;
  label: string;
  count?: number;
  active?: boolean;
}) {
  return (
    <Link
      href={href}
      className={`flex items-center justify-between rounded-lg px-3 py-2 text-sm transition-colors ${
        active
          ? "bg-[var(--accent-soft)] font-medium text-[var(--accent-ink)]"
          : "text-[var(--foreground)] hover:bg-[var(--surface-muted)]"
      }`}
    >
      <span>{label}</span>
      {typeof count === "number" ? (
        <span
          className={`font-display text-sm ${
            active ? "text-[var(--accent-ink)]" : "text-[var(--faint)]"
          }`}
        >
          {count}
        </span>
      ) : null}
    </Link>
  );
}
