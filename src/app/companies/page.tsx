import { CompaniesClient } from "@/components/companies-client";
import { listCompaniesWithWatches } from "@/lib/companies/service";

export const dynamic = "force-dynamic";

export default async function CompaniesPage() {
  const companies = listCompaniesWithWatches();

  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <div>
        <h1 className="font-display text-3xl font-semibold tracking-tight">Companies</h1>
        <p className="mt-1 text-sm text-[var(--muted)]">
          Follow a company&apos;s job board and we&apos;ll surface new roles automatically.
          Careers pages only ping you when something actually changes.
        </p>
      </div>
      <CompaniesClient initial={companies} />
    </div>
  );
}
