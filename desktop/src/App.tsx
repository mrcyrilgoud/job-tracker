import { lazy, Suspense, useEffect, type ReactNode } from "react";
import { BrowserRouter, Route, Routes } from "react-router-dom";

import { Layout } from "@/components/Layout";
import { api } from "@/lib/api";
import { RunMonitorProvider } from "@/lib/RunMonitorContext";
import { isDesktopShell } from "@/lib/tauri";
import { ThemeProvider } from "@/lib/ThemeContext";

// Keep the shell and monitoring state in the initial chunk. Each destination
// is loaded only when its route is visited, so the common Jobs startup path
// does not pay for every page's data and component graph.
const CompaniesPage = lazy(() =>
  import("@/pages/CompaniesPage").then(({ CompaniesPage: Page }) => ({ default: Page })),
);
const CompanyDetailPage = lazy(() =>
  import("@/pages/CompanyDetailPage").then(({ CompanyDetailPage: Page }) => ({ default: Page })),
);
const DocumentsPage = lazy(() =>
  import("@/pages/DocumentsPage").then(({ DocumentsPage: Page }) => ({ default: Page })),
);
const JobDetailPage = lazy(() =>
  import("@/pages/JobDetailPage").then(({ JobDetailPage: Page }) => ({ default: Page })),
);
const JobsPage = lazy(() =>
  import("@/pages/JobsPage").then(({ JobsPage: Page }) => ({ default: Page })),
);
const NewJobPage = lazy(() =>
  import("@/pages/NewJobPage").then(({ NewJobPage: Page }) => ({ default: Page })),
);
const NewRolesPage = lazy(() =>
  import("@/pages/NewRolesPage").then(({ NewRolesPage: Page }) => ({ default: Page })),
);
const SettingsPage = lazy(() =>
  import("@/pages/SettingsPage").then(({ SettingsPage: Page }) => ({ default: Page })),
);

function RouteLoadingFallback() {
  return (
    <div className="route-loading" role="status" aria-live="polite">
      <span className="spinner" aria-hidden="true" />
      <span>Loading page…</span>
    </div>
  );
}

function LazyRoute({ children }: { children: ReactNode }) {
  return <Suspense fallback={<RouteLoadingFallback />}>{children}</Suspense>;
}

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
  useEffect(() => {
    if (isDesktopShell()) {
      api.showMainWindow().catch(console.error);
    }
  }, []);

  if (!isDesktopShell()) {
    return <BrowserOnlyNotice />;
  }

  return (
    <ThemeProvider>
      {/* Outside the router so run state survives route changes (Req 2.4, 12.1). */}
      <RunMonitorProvider>
        <BrowserRouter>
          <Routes>
            <Route element={<Layout />}>
              <Route index element={<LazyRoute><JobsPage /></LazyRoute>} />
              <Route path="jobs/new" element={<LazyRoute><NewJobPage /></LazyRoute>} />
              <Route path="jobs/:id" element={<LazyRoute><JobDetailPage /></LazyRoute>} />
              <Route path="documents" element={<LazyRoute><DocumentsPage /></LazyRoute>} />
              <Route path="new-roles" element={<LazyRoute><NewRolesPage /></LazyRoute>} />
              <Route path="companies" element={<LazyRoute><CompaniesPage /></LazyRoute>} />
              <Route path="companies/:id" element={<LazyRoute><CompanyDetailPage /></LazyRoute>} />
              <Route path="settings" element={<LazyRoute><SettingsPage /></LazyRoute>} />
            </Route>
          </Routes>
        </BrowserRouter>
      </RunMonitorProvider>
    </ThemeProvider>
  );
}
