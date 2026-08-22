import { NavLink, Outlet, Link } from "react-router-dom";

import {
  BriefcaseIcon,
  BuildingIcon,
  DocumentIcon,
  MailIcon,
  MoonIcon,
  SettingsIcon,
  SunIcon,
} from "@/components/icons";
import { RunJobsButton } from "@/components/RunJobsButton";
import { useTheme } from "@/lib/ThemeContext";

const nav = [
  { href: "/", label: "Jobs", Icon: BriefcaseIcon },
  { href: "/documents", label: "Documents", Icon: DocumentIcon },
  { href: "/companies", label: "Companies", Icon: BuildingIcon },
  { href: "/gmail", label: "Gmail", Icon: MailIcon },
  { href: "/settings", label: "Settings", Icon: SettingsIcon },
];

export function Layout() {
  const { theme, toggleTheme } = useTheme();

  return (
    <div className="mx-auto flex min-h-screen max-w-6xl flex-col px-4 py-8 md:px-8">
      <header className="mb-8 flex flex-col gap-5 md:flex-row md:items-center md:justify-between">
        <Link to="/" className="flex items-center gap-3">
          <span className="flex h-10 w-10 items-center justify-center rounded-2xl bg-[var(--accent)] font-display text-lg font-semibold text-white shadow-[var(--shadow-sm)]">
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
        <div className="flex flex-wrap items-center gap-2">
          <nav className="flex flex-wrap gap-1">
            {nav.map(({ href, label, Icon }) => (
              <NavLink
                key={href}
                to={href}
                end={href === "/"}
                className={({ isActive }) =>
                  `flex items-center gap-1.5 rounded-full px-3.5 py-2 text-sm font-medium transition-colors ${
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
            className="flex h-9 w-9 items-center justify-center rounded-full text-[var(--muted)] transition-colors hover:bg-[var(--surface-muted)] hover:text-[var(--foreground)]"
            aria-label="Toggle theme"
          >
            {theme === "dark" ? <SunIcon size={18} /> : <MoonIcon size={18} />}
          </button>
          <RunJobsButton />
        </div>
      </header>
      <main className="flex-1">
        <Outlet />
      </main>
      <footer className="mt-12 text-center text-xs text-[var(--faint)]">
        Stored locally on your Mac · nothing leaves your machine
      </footer>
    </div>
  );
}
