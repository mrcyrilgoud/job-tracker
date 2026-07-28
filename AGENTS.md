# Agent notes

This is a **Tauri 2** desktop app:

- `desktop/` — Vite + React UI
- `src-tauri/` — Rust backend (SQLite, Keychain, jobs runner, ATS, Gmail)
- `scripts/install-launchd.ts` — macOS LaunchAgent installer for the hourly jobs runner

Do not reintroduce a Next.js or Node server stack. Prefer Rust for backend logic and the desktop Vite app for UI.
