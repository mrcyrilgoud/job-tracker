/**
 * Dev-only entry point for design review.
 *
 * The app refuses to render outside the Tauri window, and every screen talks to
 * the Rust backend over IPC. That makes the UI impossible to open in a normal
 * browser — and therefore impossible to screenshot, diff, or iterate on visually
 * without a full native build.
 *
 * This entry stubs the two things Tauri provides (the `isTauri` marker, set in
 * design-preview.html, and the `__TAURI_INTERNALS__` IPC bridge) with an
 * in-memory fixture backend, so the real components render against realistic
 * data in any browser.
 *
 * Not part of the production bundle: Vite's build only uses `index.html` as an
 * entry, so `design-preview.html` and this file are dev-server only.
 */
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "@/App";
import "@/index.css";
import { createFixtureBackend } from "@/dev/fixtures";

const backend = createFixtureBackend();

declare global {
  interface Window {
    __TAURI_INTERNALS__?: Record<string, unknown>;
  }
}

window.__TAURI_INTERNALS__ = {
  invoke: (cmd: string, args?: Record<string, unknown>) => backend.invoke(cmd, args),
  transformCallback: (callback?: (payload: unknown) => void) => {
    const id = Math.floor(Math.random() * 1_000_000);
    (window as unknown as Record<string, unknown>)[`_${id}`] = callback ?? (() => {});
    return id;
  },
  unregisterCallback: () => {},
  convertFileSrc: (filePath: string) => filePath,
};

// The app routes with BrowserRouter, so the entry's own path
// (/design-preview.html) matches nothing. Rewrite it to the requested route
// before mounting: /design-preview.html?route=/companies renders /companies.
const requestedRoute = new URLSearchParams(window.location.search).get("route");
window.history.replaceState(null, "", requestedRoute ?? "/");

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
