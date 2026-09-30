/**
 * Structured run errors from Tauri commands.
 *
 * Coded backend errors are strings of the form `code:category[:detail]`
 * (for example `operation_in_progress:runner`, `cancel_not_allowed:completed`,
 * `run_failed:runner_interrupted`). The string is split on its first two
 * colons, so a detail may itself contain colons. A bare `code` (for example
 * `run_not_found`) has no category. Anything else is a free-form message.
 */

export type AppErrorParts = {
  /** Machine-readable code, or `"unknown"` for a free-form message. */
  code: string;
  category?: string;
  detail?: string;
  /** Human-readable text: the original error string. */
  message: string;
};

const CODE = /^[a-z][a-z0-9_]*$/;

function rawMessage(err: unknown): string {
  if (err === null || err === undefined) return "";
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  if (typeof err === "object" && err !== null && "message" in err) {
    const m = (err as { message: unknown }).message;
    if (typeof m === "string") return m;
  }
  try {
    return JSON.stringify(err) ?? String(err);
  } catch {
    return String(err);
  }
}

/** Parse a rejected `invoke` value (string, Error, or `{ message }`) into its coded parts. */
export function parseAppError(err: unknown): AppErrorParts {
  const message = rawMessage(err);
  const trimmed = message.trim();

  const first = trimmed.indexOf(":");
  const code = first === -1 ? trimmed : trimmed.slice(0, first);
  if (!CODE.test(code)) {
    return { code: "unknown", message };
  }
  if (first === -1) {
    return { code, message };
  }

  const rest = trimmed.slice(first + 1);
  const second = rest.indexOf(":");
  const category = second === -1 ? rest : rest.slice(0, second);
  const detail = second === -1 ? "" : rest.slice(second + 1);

  const parts: AppErrorParts = { code, message };
  if (category !== "") parts.category = category;
  if (detail !== "") parts.detail = detail;
  return parts;
}

/** True for `operation_in_progress:runner`, the runner lock rejection. */
export function isRunInProgressError(err: unknown): boolean {
  const parts = parseAppError(err);
  return parts.code === "operation_in_progress" && parts.category === "runner";
}
