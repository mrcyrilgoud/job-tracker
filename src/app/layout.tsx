import type { Metadata } from "next";
import { Fraunces, Plus_Jakarta_Sans } from "next/font/google";
import Link from "next/link";

import {
  BriefcaseIcon,
  BuildingIcon,
  DocumentIcon,
  MailIcon,
} from "@/components/icons";
import "./globals.css";

const jakarta = Plus_Jakarta_Sans({
  variable: "--font-jakarta",
  subsets: ["latin"],
  weight: ["400", "500", "600", "700"],
});

const fraunces = Fraunces({
  variable: "--font-fraunces",
  subsets: ["latin"],
  weight: ["400", "500", "600"],
});

export const metadata: Metadata = {
  title: "Job Tracker",
  description: "Personal local job application tracker",
};

const nav = [
  { href: "/", label: "Jobs", Icon: BriefcaseIcon },
  { href: "/documents", label: "Documents", Icon: DocumentIcon },
  { href: "/companies", label: "Companies", Icon: BuildingIcon },
  { href: "/gmail", label: "Gmail", Icon: MailIcon },
];

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html
      lang="en"
      className={`${jakarta.variable} ${fraunces.variable} h-full antialiased`}
    >
      <body className="min-h-full">
        <div className="mx-auto flex min-h-screen max-w-6xl flex-col px-4 py-8 md:px-8">
          <header className="mb-8 flex flex-col gap-5 md:flex-row md:items-center md:justify-between">
            <Link href="/" className="flex items-center gap-3">
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
            <nav className="flex flex-wrap gap-1">
              {nav.map(({ href, label, Icon }) => (
                <Link
                  key={href}
                  href={href}
                  className="flex items-center gap-1.5 rounded-full px-3.5 py-2 text-sm font-medium text-[var(--muted)] transition-colors hover:bg-[var(--surface-muted)] hover:text-[var(--foreground)]"
                >
                  <Icon size={16} className="text-[var(--faint)]" />
                  {label}
                </Link>
              ))}
            </nav>
          </header>
          <main className="flex-1">{children}</main>
          <footer className="mt-12 text-center text-xs text-[var(--faint)]">
            Stored locally on your Mac · nothing leaves your machine
          </footer>
        </div>
      </body>
    </html>
  );
}
