import { useLayoutEffect, useRef, useState, type MouseEvent } from "react";
import { NavLink, Outlet, Link, useLocation } from "react-router-dom";

import {
  BookmarkIcon,
  BriefcaseIcon,
  BuildingIcon,
  DocumentIcon,
  MoonIcon,
  SettingsIcon,
  SunIcon,
} from "@/components/icons";
import { ChecksTab } from "@/components/runs/ChecksTab";
import { RunLiveRegions } from "@/components/runs/RunLiveRegions";
import { RunPanel } from "@/components/runs/RunPanel";
import { useRunMonitor } from "@/lib/RunMonitorContext";
import { useTheme } from "@/lib/ThemeContext";

const nav = [
  { href: "/", label: "Jobs", Icon: BriefcaseIcon },
  { href: "/documents", label: "Documents", Icon: DocumentIcon },
  { href: "/new-roles", label: "New Roles", Icon: BookmarkIcon },
  { href: "/companies", label: "Companies", Icon: BuildingIcon },
  { href: "/settings", label: "Settings", Icon: SettingsIcon },
];

export function Layout() {
  const { theme, toggleTheme } = useTheme();
  const { state } = useRunMonitor();
  const { pathname } = useLocation();
  const previousPathname = useRef(pathname);
  const [runDetailsOpen, setRunDetailsOpen] = useState(false);
  const runId = state.displayed?.runId ?? null;
  // Hiding details is local navigation state: monitoring and the saved run
  // remain intact. Restored and background runs never replace the main page.
  const panelVisible = runId !== null && runDetailsOpen;
  const showRun = () => setRunDetailsOpen(true);

  useLayoutEffect(() => {
    if (previousPathname.current === pathname) return;
    previousPathname.current = pathname;
    setRunDetailsOpen(false);
  }, [pathname]);

  const navigateToPage = (event: MouseEvent<HTMLAnchorElement>) => {
    // Modified clicks open another tab/window and leave this view untouched.
    if (
      event.defaultPrevented || event.button !== 0 ||
      event.metaKey || event.ctrlKey || event.shiftKey || event.altKey
    ) return;
    // Also handle the active page link, where pathname does not change.
    setRunDetailsOpen(false);
  };

  return (
    <div className="mx-auto flex min-h-screen max-w-6xl flex-col px-4 py-8 md:px-8">
      <header className="app-header">
        <Link to="/" onClick={navigateToPage} className="app-brand">
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl bg-[var(--accent)] font-display text-lg font-semibold text-white shadow-[var(--shadow-sm)]">
            J
          </span>
          <span>
            <span className="block font-display text-xl font-semibold tracking-tight">
              Job Tracker
            </span>
            <span className="block text-xs text-[var(--faint)]">
              Your search, quietly organized
            </span>
          </span>
        </Link>
        <div className="app-navigation">
          <nav aria-label="Main navigation" className="app-nav-links">
            {nav.map(({ href, label, Icon }) => (
              <NavLink
                key={href}
                to={href}
                onClick={navigateToPage}
                end={href === "/"}
                className={({ isActive }) =>
                  `app-nav-link ${
                    isActive
                      ? "bg-[var(--accent-soft)] text-[var(--accent-ink)]"
                      : "text-[var(--muted)] hover:bg-[var(--surface-muted)] hover:text-[var(--foreground)]"
                  }`
                }
              >
                <Icon size={16} className="text-[var(--faint)]" />
                {label}
              </NavLink>
            ))}
          </nav>
          <button
            onClick={toggleTheme}
            className="app-theme-toggle text-[var(--muted)] hover:bg-[var(--surface-muted)] hover:text-[var(--foreground)]"
            aria-label="Toggle theme"
          >
            {theme === "dark" ? <SunIcon size={18} /> : <MoonIcon size={18} />}
          </button>
        </div>
        <ChecksTab panelVisible={panelVisible} onShowRun={showRun} />
      </header>
      <main className="flex-1">
        <RunLiveRegions />
        {panelVisible ? <RunPanel /> : null}
        <Outlet />
      </main>
      <footer className="mt-12 text-center text-xs text-[var(--faint)]">
        Your tracker data is stored locally on your Mac
      </footer>
    </div>
  );
}
