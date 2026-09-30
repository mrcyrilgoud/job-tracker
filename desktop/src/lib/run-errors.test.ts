import { describe, expect, it } from "vitest";

import { isRunInProgressError, parseAppError } from "@/lib/run-errors";

describe("parseAppError", () => {
  it("parses code and category", () => {
    expect(parseAppError("operation_in_progress:runner")).toEqual({
      code: "operation_in_progress",
      category: "runner",
      message: "operation_in_progress:runner",
    });
    expect(parseAppError("cancel_not_allowed:completed")).toMatchObject({
      code: "cancel_not_allowed",
      category: "completed",
    });
  });

  it("splits only on the first two colons, keeping colons in the detail", () => {
    expect(parseAppError("retry_ineligible:job:ids a:b")).toEqual({
      code: "retry_ineligible",
      category: "job",
      detail: "ids a:b",
      message: "retry_ineligible:job:ids a:b",
    });
    expect(parseAppError("run_canceled:4f1c-uuid")).toMatchObject({
      code: "run_canceled",
      category: "4f1c-uuid",
    });
  });

  it("accepts a bare code with no category", () => {
    const parts = parseAppError("run_not_found");
    expect(parts).toEqual({ code: "run_not_found", message: "run_not_found" });
    expect(parts).not.toHaveProperty("category");
  });

  it("omits empty category and detail segments", () => {
    const parts = parseAppError("run_start_failed:");
    expect(parts.code).toBe("run_start_failed");
    expect(parts).not.toHaveProperty("category");
    expect(parts).not.toHaveProperty("detail");
    expect(parseAppError("a_code:cat:")).not.toHaveProperty("detail");
  });

  it("treats free-form messages as unknown and keeps the text", () => {
    for (const raw of ["database busy; retry shortly", "Database error: disk full", "Job not found", ""]) {
      expect(parseAppError(raw)).toEqual({ code: "unknown", message: raw });
    }
  });

  it("reads Error instances and { message } objects", () => {
    expect(parseAppError(new Error("operation_in_progress:runner"))).toMatchObject({
      code: "operation_in_progress",
      category: "runner",
    });
    expect(parseAppError({ message: "run_failed:runner_interrupted" })).toMatchObject({
      code: "run_failed",
      category: "runner_interrupted",
    });
    expect(parseAppError(undefined).code).toBe("unknown");
    expect(parseAppError(42)).toEqual({ code: "unknown", message: "42" });
  });

  it("detects the runner lock rejection", () => {
    expect(isRunInProgressError("operation_in_progress:runner")).toBe(true);
    expect(isRunInProgressError(new Error("operation_in_progress:runner"))).toBe(true);
    expect(isRunInProgressError("operation_in_progress:csv")).toBe(false);
    expect(isRunInProgressError("run_not_found")).toBe(false);
  });
});
