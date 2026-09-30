import { describe, expect, it } from "vitest";
import corpus from "@/lib/__fixtures__/run-contract-corpus.json";

import {
  CONTRACT_BOUNDS,
  MAX_COUNT,
  MAX_ID_BYTES,
  MAX_MESSAGE_BYTES,
  MAX_TITLE_BYTES,
  PROGRESS_CONTRACT_VERSION,
  RunContractError,
  parseRunAccepted,
  parseRunEvent,
  parseRunSnapshot,
  parseRunSummary,
  unwrapContract,
  utf8ByteLength,
  type ParseResult,
  type PostingStateValue,
  type PostingStatus,
} from "@/lib/run-contract";
import { isRetryEligible, needsAttention } from "@/lib/run-state";

type Json = Record<string, unknown>;

/** Mirrors `event()` in src-tauri/src/runs/progress.rs tests (serde output). */
function event(overrides: Json = {}): Json {
  return {
    version: 1,
    runId: "run-1",
    runType: "postingCheck",
    runStatus: "active",
    seq: 2,
    emittedAt: "2026-01-01T00:00:01Z",
    stage: "postings",
    message: "Checking job postings",
    current: 1,
    total: 3,
    done: false,
    startedAt: "2026-01-01T00:00:00Z",
    elapsedMs: 1000,
    postingCounts: { queued: 1, active: 1, completed: 1, error: 0, canceled: 0 },
    postingTotal: 3,
    ...overrides,
  };
}

function completedPosting(overrides: Json = {}): Json {
  return {
    jobId: "c",
    title: "Engineer c",
    companyName: "Acme",
    postingUrl: "https://example.com/jobs/c",
    status: "completed",
    postingState: "inactive",
    reasonCode: "http_404",
    reason: "Closed: posting returned HTTP 404",
    evidence: {
      evidenceVersion: 1,
      attemptedAt: "t1",
      requestedUrl: "https://example.com/jobs/c",
      finalUrl: "https://example.com/careers",
      httpStatus: 404,
      redirectStatuses: [301, 302],
      provider: { provider: "greenhouse", signal: "absent_from_listing", postingId: "127817" },
      content: ["company_match", "generic_careers"],
      failureCategory: "timeout",
    },
    ...overrides,
  };
}

function summary(overrides: Json = {}): Json {
  return {
    runId: "run-1",
    runType: "jobsCycle",
    status: "completed",
    startedAt: "t0",
    finishedAt: "t1",
    durationMs: 5,
    postingOutcomes: { active: 0, closed: 0, unknown: 0, error: 0, canceled: 0 },
    stateChanges: 0,
    ...overrides,
  };
}

function snapshot(overrides: Json = {}): Json {
  return {
    version: 1,
    runId: "run-1",
    runType: "postingCheck",
    runStatus: "queued",
    seq: 1,
    stage: "postings",
    message: "Queued",
    current: 0,
    total: 1,
    done: false,
    startedAt: "t0",
    elapsedMs: 0,
    postingCounts: { queued: 1, active: 0, completed: 0, error: 0, canceled: 0 },
    postingTotal: 1,
    postings: [
      { jobId: "a", title: "Engineer a", companyName: "Acme", postingUrl: "https://example.com/jobs/a", status: "queued" },
    ],
    live: true,
    dismissed: false,
    ...overrides,
  };
}

function expectInvalid<T>(result: ParseResult<T>, fragment?: string): void {
  expect(result.ok).toBe(false);
  if (!result.ok) {
    expect(result.kind).toBe("invalid");
    expect(result.message.length).toBeGreaterThan(0);
    if (fragment) expect(result.message).toContain(fragment);
  }
}

describe("parseRunEvent: valid samples", () => {
  it("accepts a minimal event and returns an equal value", () => {
    const input = event();
    const result = parseRunEvent(input);
    expect(result).toEqual({ ok: true, value: input });
  });

  it("accepts the final Jobs_Cycle event with stages, summary, and a status transition", () => {
    const input = event({
      runType: "jobsCycle",
      runStatus: "completed",
      previousRunStatus: "active",
      stage: "cycle",
      message: "Jobs cycle complete",
      current: 1,
      total: 1,
      done: true,
      stages: [
        { name: "postings", outcome: "succeeded", current: 1, total: 1 },
        { name: "watches", outcome: "failed", current: 2, total: 3, error: "boom" },
        { name: "careers", outcome: "not_started", current: 0, total: 0 },
        { name: "csv", outcome: "skipped", current: 0, total: 0 },
      ],
      summary: summary({ sourceRunId: "run-0" }),
    });
    const result = parseRunEvent(input);
    expect(result).toEqual({ ok: true, value: input });
  });

  it("accepts postings with every status and full evidence", () => {
    const input = event({
      postings: [
        { jobId: "q", title: "Engineer q", companyName: "Acme", postingUrl: "https://example.com/jobs/q", status: "queued" },
        completedPosting(),
        {
          jobId: "e",
          title: "Engineer e",
          companyName: "Acme",
          postingUrl: "https://example.com/jobs/e",
          status: "error",
          reason: "worker panicked",
          failureCategory: "run_aborted",
        },
      ],
    });
    expect(parseRunEvent(input)).toEqual({ ok: true, value: input });
  });

  it("accepts an error run with an errorReason", () => {
    const input = event({ runStatus: "error", errorReason: "runner_interrupted" });
    expect(parseRunEvent(input).ok).toBe(true);
  });

  it("rejects a first event without its full posting list", () => {
    expectInvalid(parseRunEvent(event({ seq: 1 })), "event.postings is required");
  });

  it("drops unknown extra keys and keeps absent optionals absent", () => {
    const result = parseRunEvent({ ...event(), futureField: 7 });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).not.toHaveProperty("futureField");
      expect(result.value).not.toHaveProperty("previousRunStatus");
      expect(result.value).not.toHaveProperty("postings");
      expect(Object.keys(result.value).sort()).toEqual(Object.keys(event()).sort());
    }
  });

  it("returns a fresh object that round-trips through JSON", () => {
    const input = event({ postings: [completedPosting()] });
    const result = parseRunEvent(JSON.parse(JSON.stringify(input)));
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).not.toBe(input);
      expect(JSON.parse(JSON.stringify(result.value))).toEqual(input);
    }
  });
});

describe("parseRunEvent: version", () => {
  it("reports unsupported_version for a well-formed version other than 1", () => {
    for (const version of [0, 2, 99]) {
      const result = parseRunEvent(event({ version }));
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.kind).toBe("unsupported_version");
        expect(result.message).toContain(String(version));
      }
    }
  });

  it("checks the version before the rest of the shape", () => {
    const result = parseRunEvent({ version: 2, somethingElse: true });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.kind).toBe("unsupported_version");
  });

  it("treats a missing or non-numeric version as invalid", () => {
    const legacy = event();
    delete legacy.version;
    expectInvalid(parseRunEvent(legacy), "event.version is required");
    expectInvalid(parseRunEvent(event({ version: "1" })), "event.version");
    expectInvalid(parseRunEvent(event({ version: 1.5 })), "event.version");
  });

  it("rejects the pre-v1 legacy payload", () => {
    expectInvalid(parseRunEvent({ stage: "postings", message: "Checked j1", current: 1, total: 2, done: false }));
  });
});

describe("parseRunEvent: invalid inputs", () => {
  it("rejects inconsistent terminal and error optionals", () => {
    expectInvalid(parseRunEvent(event({ done: true })), "event.summary must be present");
    expectInvalid(parseRunEvent(event({ runStatus: "error" })), "event.errorReason must be present");
    expectInvalid(
      parseRunEvent(event({ runStatus: "active", errorReason: "unexpected" })),
      "event.errorReason must be present exactly when runStatus is error",
    );
  });

  it("never throws on non-object input", () => {
    for (const input of [null, undefined, 42, "event", [], true]) {
      expectInvalid(parseRunEvent(input), "must be an object");
    }
  });

  it("rejects missing required fields", () => {
    for (const key of Object.keys(event())) {
      if (key === "version") continue;
      const input = event();
      delete input[key];
      expectInvalid(parseRunEvent(input), `event.${key} is required`);
    }
  });

  it("rejects wrong types", () => {
    expectInvalid(parseRunEvent(event({ runId: 7 })), "event.runId must be a string");
    expectInvalid(parseRunEvent(event({ done: "false" })), "event.done must be a boolean");
    expectInvalid(parseRunEvent(event({ seq: "2" })), "event.seq must be an integer");
    expectInvalid(parseRunEvent(event({ elapsedMs: 1.5 })), "event.elapsedMs must be an integer");
    expectInvalid(parseRunEvent(event({ postings: {} })), "event.postings must be an array");
    expectInvalid(parseRunEvent(event({ postingCounts: [] })), "event.postingCounts must be an object");
  });

  it("rejects unknown enum values", () => {
    expectInvalid(parseRunEvent(event({ runType: "jobs_cycle" })), "run type");
    expectInvalid(parseRunEvent(event({ runStatus: "running" })), "run status");
    expectInvalid(parseRunEvent(event({ previousRunStatus: "done" })), "run status");
    expectInvalid(parseRunEvent(event({ stage: "done" })), "stage");
    expectInvalid(parseRunEvent(event({ postings: [completedPosting({ status: "checking" })] })), "posting status");
    expectInvalid(parseRunEvent(event({ postings: [completedPosting({ postingState: "closed" })] })), "posting state");
    expectInvalid(
      parseRunEvent(event({ stages: [{ name: "postings", outcome: "done", current: 0, total: 0 }] })),
      "stage outcome",
    );
    expectInvalid(
      parseRunEvent(event({ stages: [{ name: "cycle", outcome: "skipped", current: 0, total: 0 }] })),
      "stage name",
    );
    const badContent = completedPosting();
    (badContent.evidence as Json).content = ["title_match", "body_text"];
    expectInvalid(parseRunEvent(event({ postings: [badContent] })), "content signal");
    const badFailure = completedPosting();
    (badFailure.evidence as Json).failureCategory = "run_aborted";
    expectInvalid(parseRunEvent(event({ postings: [badFailure] })), "failure category");
    const badProvider = completedPosting();
    (badProvider.evidence as Json).provider = { provider: "workday", signal: "listed_open", postingId: "1" };
    expectInvalid(parseRunEvent(event({ postings: [badProvider] })), "provider");
  });

  it("rejects null for optional fields (absent, never null)", () => {
    expectInvalid(parseRunEvent(event({ previousRunStatus: null })), "event.previousRunStatus must be absent rather than null");
    expectInvalid(parseRunEvent(event({ postings: null })), "must be absent rather than null");
    expectInvalid(parseRunEvent(event({ summary: null })), "must be absent rather than null");
    expectInvalid(parseRunEvent(event({ errorReason: null })), "must be absent rather than null");
    expectInvalid(
      parseRunEvent(event({ postings: [completedPosting({ reasonCode: null })] })),
      "event.postings[0].reasonCode must be absent rather than null",
    );
    const nullFinal = completedPosting();
    (nullFinal.evidence as Json).finalUrl = null;
    expectInvalid(parseRunEvent(event({ postings: [nullFinal] })), "event.postings[0].evidence.finalUrl");
  });

  it("rejects null for required fields", () => {
    expectInvalid(parseRunEvent(event({ message: null })), "event.message must be a string, got null");
  });
});

describe("contract bounds", () => {
  it("measures bounds in UTF-8 bytes, not UTF-16 code units", () => {
    expect(utf8ByteLength("abc")).toBe(3);
    expect(utf8ByteLength("é")).toBe(2);
    expect(utf8ByteLength("日")).toBe(3);
    expect(utf8ByteLength("🦀")).toBe(4);
    expect("🦀".length).toBe(2);
  });

  it("accepts strings exactly at the byte bound and rejects one byte over", () => {
    expect(parseRunEvent(event({ message: "a".repeat(MAX_MESSAGE_BYTES) })).ok).toBe(true);
    expectInvalid(parseRunEvent(event({ message: "a".repeat(MAX_MESSAGE_BYTES + 1) })), "exceeds 500");
    expect(parseRunEvent(event({ runId: "r".repeat(MAX_ID_BYTES) })).ok).toBe(true);
    expectInvalid(parseRunEvent(event({ runId: "r".repeat(MAX_ID_BYTES + 1) })), "event.runId");
  });

  it("rejects multibyte text that fits in code units but not in bytes", () => {
    // 200 "日" = 200 UTF-16 code units but 600 UTF-8 bytes.
    const title = "日".repeat(200);
    expect(title.length).toBeLessThan(MAX_TITLE_BYTES);
    expectInvalid(parseRunEvent(event({ postings: [completedPosting({ title })] })), "event.postings[0].title is 600 bytes");
    // 100 "日" = 300 bytes: exactly at the bound.
    expect(parseRunEvent(event({ postings: [completedPosting({ title: "日".repeat(100) })] })).ok).toBe(true);
    // 17 crabs = 68 bytes > 64 for jobId.
    expectInvalid(parseRunEvent(event({ postings: [completedPosting({ jobId: "🦀".repeat(17) })] })), "jobId");
    // A Rust-truncated value ("…" is 3 bytes) at the bound is accepted.
    const truncated = `${"a".repeat(MAX_MESSAGE_BYTES - 3)}…`;
    expect(utf8ByteLength(truncated)).toBe(MAX_MESSAGE_BYTES);
    expect(parseRunEvent(event({ message: truncated })).ok).toBe(true);
  });

  it("bounds reason, category, URL, and stage-error strings", () => {
    expectInvalid(parseRunEvent(event({ errorReason: "x".repeat(501) })), "errorReason");
    expectInvalid(parseRunEvent(event({ postings: [completedPosting({ reason: "x".repeat(501) })] })), "reason");
    expectInvalid(parseRunEvent(event({ postings: [completedPosting({ reasonCode: "c".repeat(65) })] })), "reasonCode");
    expectInvalid(
      parseRunEvent(event({ postings: [completedPosting({ postingUrl: `https://e.com/${"p".repeat(2048)}` })] })),
      "postingUrl",
    );
    expectInvalid(
      parseRunEvent(event({ stages: [{ name: "csv", outcome: "failed", current: 0, total: 0, error: "e".repeat(501) }] })),
      "stages[0].error",
    );
  });

  it("accepts counts up to 2^53 − 1 and rejects larger or negative counts", () => {
    expect(MAX_COUNT).toBe(Number.MAX_SAFE_INTEGER);
    expect(parseRunEvent(event({ current: MAX_COUNT, total: MAX_COUNT, postingTotal: MAX_COUNT })).ok).toBe(true);
    expectInvalid(parseRunEvent(event({ total: MAX_COUNT + 1 })), "event.total");
    expectInvalid(parseRunEvent(event({ elapsedMs: -1 })), "event.elapsedMs");
    expectInvalid(
      parseRunEvent(event({ postingCounts: { queued: -1, active: 0, completed: 0, error: 0, canceled: 0 } })),
      "postingCounts.queued",
    );
    expectInvalid(parseRunEvent(event({ total: Number.POSITIVE_INFINITY })), "event.total");
    expectInvalid(parseRunEvent(event({ total: Number.NaN })), "event.total");
  });

  it("rejects current > total on events, snapshots, and stages", () => {
    expectInvalid(parseRunEvent(event({ current: 4, total: 3 })), "event.current (4) exceeds event.total (3)");
    expectInvalid(parseRunSnapshot(snapshot({ current: 2, total: 1 })), "snapshot.current");
    expectInvalid(
      parseRunEvent(event({ stages: [{ name: "watches", outcome: "in_progress", current: 3, total: 2 }] })),
      "stages[0].current",
    );
  });

  it("rejects HTTP statuses outside u16", () => {
    const p = completedPosting();
    (p.evidence as Json).httpStatus = 70000;
    expectInvalid(parseRunEvent(event({ postings: [p] })), "httpStatus");
  });

  it("exposes the Rust bound constants", () => {
    expect(CONTRACT_BOUNDS).toEqual({
      MAX_ID_BYTES: 64,
      MAX_TITLE_BYTES: 300,
      MAX_COMPANY_BYTES: 200,
      MAX_URL_BYTES: 2048,
      MAX_MESSAGE_BYTES: 500,
      MAX_REASON_BYTES: 500,
      MAX_CATEGORY_BYTES: 64,
    });
    expect(PROGRESS_CONTRACT_VERSION).toBe(1);
  });
});

describe("parseRunSnapshot / parseRunAccepted / parseRunSummary", () => {
  it("accepts a queued snapshot and an accepted wrapper", () => {
    expect(parseRunSnapshot(snapshot())).toEqual({ ok: true, value: snapshot() });
    const accepted = { runId: "run-1", snapshot: snapshot() };
    expect(parseRunAccepted(accepted)).toEqual({ ok: true, value: accepted });
  });

  it("accepts a terminal snapshot with summary and sourceRunId", () => {
    const input = snapshot({
      runStatus: "completed_with_errors",
      done: true,
      current: 1,
      summary: summary({
        runType: "postingCheck",
        status: "completed_with_errors",
        postingOutcomes: { active: 0, closed: 0, unknown: 0, error: 1, canceled: 0 },
      }),
      sourceRunId: "run-0",
      live: false,
      dismissed: false,
    });
    expect(parseRunSnapshot(input)).toEqual({ ok: true, value: input });
  });

  it("requires the full postings list and snapshot flags", () => {
    for (const key of ["postings", "live", "dismissed"]) {
      const input = snapshot();
      delete input[key];
      expectInvalid(parseRunSnapshot(input), `snapshot.${key} is required`);
    }
  });

  it("does not read event-only fields from snapshots", () => {
    const result = parseRunSnapshot(snapshot({ previousRunStatus: "queued", emittedAt: "t" }));
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).not.toHaveProperty("previousRunStatus");
      expect(result.value).not.toHaveProperty("emittedAt");
    }
  });

  it("propagates version errors through the accepted wrapper", () => {
    const result = parseRunAccepted({ runId: "run-1", snapshot: snapshot({ version: 2 }) });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.kind).toBe("unsupported_version");
    expectInvalid(parseRunAccepted({ snapshot: snapshot() }), "accepted.runId is required");
  });

  it("requires all five posting outcome keys in a summary", () => {
    expect(parseRunSummary(summary()).ok).toBe(true);
    expectInvalid(
      parseRunSummary(summary({ postingOutcomes: { active: 0, closed: 0, unknown: 0, error: 0 } })),
      "summary.postingOutcomes.canceled is required",
    );
    expectInvalid(parseRunSummary(summary({ sourceRunId: null })), "must be absent rather than null");
  });
});

describe("unwrapContract", () => {
  it("returns the value or throws RunContractError with the result kind", () => {
    expect(unwrapContract(parseRunSummary(summary()))).toEqual(summary());
    try {
      unwrapContract(parseRunSnapshot(snapshot({ version: 3 })));
      expect.unreachable();
    } catch (err) {
      expect(err).toBeInstanceOf(RunContractError);
      expect((err as RunContractError).kind).toBe("unsupported_version");
    }
    expect(() => unwrapContract(parseRunSnapshot(null))).toThrow(RunContractError);
  });
});

describe("Rust golden contract corpus", () => {
  it("keeps mirrored bounds and parses every event and snapshot", () => {
    expect(corpus.bounds).toEqual({ ...CONTRACT_BOUNDS, MAX_COUNT });
    for (const raw of corpus.events) {
      const result = parseRunEvent(raw);
      expect(result.ok).toBe(true);
    }
    for (const raw of corpus.snapshots) {
      expect(parseRunSnapshot(raw).ok).toBe(true);
    }
  });

  it("agrees with Rust on retry and attention eligibility", async () => {
    for (const row of corpus.eligibility) {
      const input = {
        status: row.status as PostingStatus,
        ...(row.postingState ? { postingState: row.postingState as PostingStateValue } : {}),
      } as Parameters<typeof isRetryEligible>[0];
      expect(isRetryEligible(input)).toBe(row.retry);
      expect(needsAttention(input)).toBe(row.attention);
    }
  });
});
