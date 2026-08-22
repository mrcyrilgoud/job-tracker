import { BrowserRouter, Route, Routes } from "react-router-dom";

import { Layout } from "@/components/Layout";
import { isDesktopShell } from "@/lib/tauri";
import { ThemeProvider } from "@/lib/ThemeContext";
import { CompaniesPage } from "@/pages/CompaniesPage";
import { CompanyDetailPage } from "@/pages/CompanyDetailPage";
import { DocumentsPage } from "@/pages/DocumentsPage";
import { GmailPage } from "@/pages/GmailPage";
import { JobDetailPage } from "@/pages/JobDetailPage";
import { JobsPage } from "@/pages/JobsPage";
import { NewJobPage } from "@/pages/NewJobPage";
import { SettingsPage } from "@/pages/SettingsPage";

function BrowserOnlyNotice() {
  return (
    <div className="mx-auto flex min-h-screen max-w-lg flex-col justify-center px-6 py-16">
      <div className="mb-6 flex items-center gap-3">
        <span className="flex h-10 w-10 items-center justify-center rounded-2xl bg-[var(--accent)] font-display text-lg font-semibold text-white shadow-[var(--shadow-sm)]">
          J
        </span>
        <span className="font-display text-xl font-semibold tracking-tight">Job Tracker</span>
      </div>
      <h1 className="font-display text-2xl font-medium tracking-tight text-[var(--foreground)]">
        Open the desktop app
      </h1>
      <p className="mt-3 text-sm leading-relaxed text-[var(--muted)]">
        This UI talks to a local Rust backend through Tauri. A normal browser tab does not have
        that bridge, so the app only runs inside the Mac window.
      </p>
      <pre className="mt-6 overflow-x-auto rounded-2xl bg-[var(--surface-muted)] px-4 py-3 text-sm text-[var(--foreground)] shadow-[var(--shadow-sm)]">
        npm run tauri:dev
      </pre>
      <p className="mt-3 text-xs text-[var(--faint)]">
        Or open the packaged Job Tracker.app after <code>npm run tauri:build</code>.
      </p>
    </div>
  );
}

export function App() {
  if (!isDesktopShell()) {
    return <BrowserOnlyNotice />;
  }

  return (
    <ThemeProvider>
      <BrowserRouter>
        <Routes>
          <Route element={<Layout />}>
            <Route index element={<JobsPage />} />
            <Route path="jobs/new" element={<NewJobPage />} />
            <Route path="jobs/:id" element={<JobDetailPage />} />
            <Route path="documents" element={<DocumentsPage />} />
            <Route path="companies" element={<CompaniesPage />} />
            <Route path="companies/:id" element={<CompanyDetailPage />} />
            <Route path="gmail" element={<GmailPage />} />
            <Route path="settings" element={<SettingsPage />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </ThemeProvider>
  );
}
