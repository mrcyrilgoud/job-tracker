import { isTauri } from "@tauri-apps/api/core";

/** True only inside the native Job Tracker window (not a regular browser tab). */
export function isDesktopShell(): boolean {
  return isTauri();
}

export const DESKTOP_SHELL_REQUIRED =
  "Job Tracker needs the desktop app. Run `npm run tauri:dev` (or open the packaged app) instead of visiting this URL in a browser.";
