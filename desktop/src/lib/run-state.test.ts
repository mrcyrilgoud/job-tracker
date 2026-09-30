import { describe, expect, it } from "vitest";

import type {
  PostingProgress,
  RunProgressEvent,
  RunSnapshot,
  RunStatus,
  RunSummary,
} from "@/lib/run-contract";
import { RUN_STATUSES, POSTING_STATUSES } from "@/lib/run-contract";
import {
  IN_PROGRESS_MESSAGE,
  NOTICE_TTL_MS,
  POSTING_STATUS_LABELS,
  RUN_STATUS_LABELS,
  buildAnnouncements,
  canCancel,
  canRetry,
  formatElapsed,
  initialAnnouncementThrottle,
  initialRunViewState,
  isRetryEligible,
  needsAttention,
  postingRowStatusText,
  reduceRun,
  retrySelectionIds,
  runViewModel,
  visiblePostings,
  type RunAction,
  type RunViewState,
} from "@/lib/run-state";

// ---- Fixtures ----

function posting(jobId: string, overrides: Partial<PostingProgress> = {}): PostingProgress {
  return {
    jobId,
    title: `Engineer ${jobId}`,
    companyName: "Acme",
    postingUrl: `https://example.com/jobs/${jobId}`,
    status: "queued",
    ...overrides,
  };
}

const IDS = ["a", "b", "c"];

function snapshot(overrides: Partial<RunSnapshot> = {}): RunSnapshot {
  return {
    version: 1,
    runId: "run-1",
    runType: "postingCheck",
    runStatus: "queued",
    seq: 1,
    stage: "postings",
    message: "Queued",
    current: 0,
    total: 3,
    done: false,
    startedAt: "2026-01-01T00:00:00Z",
    elapsedMs: 0,
    postingCounts: { queued: 3, active: 0, completed: 0, error: 0, canceled: 0 },
    postingTotal: 3,
    postings: IDS.map((id) => posting(id)),
    live: true,
    dismissed: false,
    ...overrides,
  };
}

function event(overrides: Partial<RunProgressEvent> = {}): RunProgressEvent {
  return {
    version: 1,
    runId: "run-1",
    runType: "postingCheck",
    runStatus: "active",
    seq: 2,
    emittedAt: "2026-01-01T00:00:01Z",
    stage: "postings",
    message: "Checking",
    current: 0,
    total: 3,
    done: false,
    startedAt: "2026-01-01T00:00:00Z",
    elapsedMs: 1000,
    postingCounts: { queued: 2, active: 1, completed: 0, error: 0, canceled: 0 },
    postingTotal: 3,
    ...overrides,
  };
}

function summary(overrides: Partial<RunSummary> = {}): RunSummary {
  return {
    runId: "run-1",
    runType: "postingCheck",
    status: "completed",
    startedAt: "2026-01-01T00:00:00Z",
    finishedAt: "2026-01-01T00:00:09Z",
    durationMs: 9000,
    postingOutcomes: { active: 1, closed: 1, unknown: 1, error: 0, canceled: 0 },
    stateChanges: 1,
    ...overrides,
  };
}

function reduceAll(actions: RunAction[], start: RunViewState = initialRunViewState): RunViewState {
  return actions.reduce(reduceRun, start);
}

function accepted(snap: RunSnapshot = snapshot()): RunAction {
  return { type: "accepted", accepted: { runId: snap.runId, snapshot: snap } };
}

/** A terminal run-1 with one row per retry-relevant outcome. */
function terminalState(): RunViewState {
  return reduceAll([
    accepted(
      snapshot({
        runStatus: "completed_with_errors",
        seq: 9,
        done: true,
        current: 4,
        total: 4,
        postingTotal: 4,
        postingCounts: { queued: 0, active: 0, completed: 2, error: 1, canceled: 1 },
        postings: [
          posting("open", { status: "completed", postingState: "active", reason: "Open: listed" }),
          posting("unk", { status: "completed", postingState: "unknown", reason: "Unknown: timed out after 30s" }),
          posting("err", { status: "error", reason: "persistence failure", failureCategory: "persistence" }),
          posting("can", { status: "canceled" }),
        ],
        summary: summary({ status: "completed_with_errors" }),
      }),
    ),
  ]);
}

// ---- Reducer ----

describe("reduceRun: accepted and events", () => {
  it("displays an accepted run and applies following events as deltas", () => {
    const s = reduceAll([
      accepted(),
      {
        type: "event",
        event: event({ seq: 2, previousRunStatus: "queued", postings: [posting("a", { status: "active" })] }),
      },
      {
        type: "event",
        event: event({
          seq: 3,
          current: 1,
          postingCounts: { queued: 2, active: 0, completed: 1, error: 0, canceled: 0 },
          postings: [
            posting("a", { status: "completed", postingState: "inactive", reason: "Closed: posting returned HTTP 404" }),
          ],
        }),
      },
    ]);
    expect(s.displayed?.runStatus).toBe("active");
    expect(s.lastSeq).toBe(3);
    expect(s.needsReconcile).toBe(false);
    expect(s.displayed?.postings.map((p) => [p.jobId, p.status])).toEqual([
      ["a", "completed"],
      ["b", "queued"],
      ["c", "queued"],
    ]);
    expect(s.displayed?.postingCounts.completed).toBe(1);
  });

  it("builds the displayed run from a seq=1 Queued event that beats the accepted response", () => {
    const s = reduceAll([
      { type: "event", event: event({ seq: 1, runStatus: "queued", postings: IDS.map((id) => posting(id)) }) },
      { type: "event", event: event({ seq: 2, postings: [posting("b", { status: "active" })] }) },
      accepted(snapshot({ sourceRunId: "run-0" })),
    ]);
    expect(s.lastSeq).toBe(2);
    expect(s.displayed?.runStatus).toBe("active");
    expect(s.displayed?.sourceRunId).toBe("run-0");
  });

  it("ignores duplicate and out-of-order seq values", () => {
    const afterThree = reduceAll([
      accepted(),
      { type: "event", event: event({ seq: 2 }) },
      { type: "event", event: event({ seq: 3, current: 1 }) },
    ]);
    const dup = reduceRun(afterThree, { type: "event", event: event({ seq: 3, current: 2 }) });
    const old = reduceRun(afterThree, { type: "event", event: event({ seq: 2, current: 0 }) });
    expect(dup).toBe(afterThree);
    expect(old).toBe(afterThree);
  });

  it("marks needsReconcile on a sequence gap and still applies the newer event", () => {
    const s = reduceAll([accepted(), { type: "event", event: event({ seq: 4, current: 2 }) }]);
    expect(s.needsReconcile).toBe(true);
    expect(s.lastSeq).toBe(4);
    expect(s.displayed?.current).toBe(2);
  });

  it("ignores events for a different run id or run type", () => {
    const s = reduceAll([accepted(), { type: "event", event: event({ seq: 2 }) }]);
    expect(reduceRun(s, { type: "event", event: event({ runId: "run-2", seq: 5 }) })).toBe(s);
    expect(reduceRun(s, { type: "event", event: event({ runType: "jobsCycle", seq: 3 }) })).toBe(s);
    // A seq=1 event for another run that is not Queued is not a takeover either.
    expect(reduceRun(s, { type: "event", event: event({ runId: "run-2", seq: 1, runStatus: "active" }) })).toBe(s);
  });

  it("lets a seq=1 Queued event for a newer run take over, resetting per-run controls", () => {
    const base = reduceRun(reduceRun(terminalState(), { type: "setFilter", filter: "attention" }), {
      type: "toggleRetry",
      jobId: "unk",
    });
    const s = reduceRun(base, {
      type: "event",
      event: event({
        runId: "run-2",
        runType: "jobsCycle",
        runStatus: "queued",
        seq: 1,
        postings: [posting("x")],
      }),
    });
    expect(s.displayed?.runId).toBe("run-2");
    expect(s.displayed?.runType).toBe("jobsCycle");
    expect(s.lastSeq).toBe(1);
    expect(s.filter).toBe("all");
    expect(s.retrySelection.size).toBe(0);
  });
});

describe("reduceRun: snapshots, invalid input, notices", () => {
  it("snapshotLoaded replaces the displayed state and clears needsReconcile", () => {
    const gapped = reduceAll([accepted(), { type: "event", event: event({ seq: 5 }) }]);
    expect(gapped.needsReconcile).toBe(true);
    const fresh = snapshot({ runStatus: "active", seq: 7, current: 2 });
    const s = reduceRun(gapped, { type: "snapshotLoaded", snapshot: fresh });
    expect(s.displayed).toEqual(fresh);
    expect(s.lastSeq).toBe(7);
    expect(s.needsReconcile).toBe(false);
  });

  it("snapshotLoaded with a dismissed or missing run clears the display", () => {
    const s = reduceRun(terminalState(), { type: "snapshotLoaded", snapshot: null });
    expect(s.displayed).toBeNull();
    const d = reduceRun(terminalState(), { type: "snapshotLoaded", snapshot: snapshot({ dismissed: true }) });
    expect(d.displayed).toBeNull();
  });

  it("keeps the last valid state and sets a transient notice on invalid input", () => {
    const valid = reduceAll([accepted(), { type: "event", event: event({ seq: 2 }) }]);
    const cases: Array<[RunAction, string]> = [
      [{ type: "invalidEvent", now: 1000 }, "validation"],
      [{ type: "unsupportedVersion", now: 1000 }, "compatibility"],
      [{ type: "snapshotFailed", now: 1000 }, "retrieval"],
    ];
    for (const [action, kind] of cases) {
      const s = reduceRun(valid, action);
      expect(s.displayed).toBe(valid.displayed);
      expect(s.lastSeq).toBe(valid.lastSeq);
      expect(s.notice?.kind).toBe(kind);
      expect(s.notice?.message).not.toBe("");
      expect(s.notice?.expiresAt).toBe(1000 + NOTICE_TTL_MS);
    }
    expect(reduceRun(valid, { type: "invalidEvent", now: 0 }).needsReconcile).toBe(true);
    expect(reduceRun(valid, { type: "unsupportedVersion", now: 0 }).needsReconcile).toBe(true);
  });

  it("a successful snapshot after a retrieval failure resolves the retrieval notice", () => {
    const failed = reduceRun(reduceRun(initialRunViewState, accepted()), { type: "snapshotFailed", now: 0 });
    const s = reduceRun(failed, { type: "snapshotLoaded", snapshot: snapshot({ seq: 3 }) });
    expect(s.notice).toBeUndefined();
  });

  it("clearNotice only removes an expired notice", () => {
    const s = reduceRun(initialRunViewState, { type: "invalidEvent", now: 1000 });
    expect(reduceRun(s, { type: "clearNotice", now: 1000 + NOTICE_TTL_MS - 1 })).toBe(s);
    expect(reduceRun(s, { type: "clearNotice", now: 1000 + NOTICE_TTL_MS }).notice).toBeUndefined();
    expect(reduceRun(s, { type: "clearNotice" }).notice).toBeUndefined();
  });

  it("rejectedInProgress shows a notice without creating a displayed run", () => {
    const s = reduceRun(initialRunViewState, { type: "rejectedInProgress", now: 0 });
    expect(s.displayed).toBeNull();
    expect(s.notice).toEqual({ kind: "inProgress", message: IN_PROGRESS_MESSAGE, expiresAt: NOTICE_TTL_MS });
    expect(s.needsReconcile).toBe(true);
  });

  it("rejectedInProgress keeps an existing displayed run unchanged", () => {
    const shown = reduceRun(initialRunViewState, accepted());
    const s = reduceRun(shown, { type: "rejectedInProgress", now: 0 });
    expect(s.displayed).toBe(shown.displayed);
  });
});

describe("reduceRun: terminal summary and dismiss", () => {
  it("keeps the terminal summary through unrelated actions until dismissed", () => {
    const done = reduceAll([
      accepted(),
      {
        type: "event",
        event: event({
          seq: 2,
          runStatus: "completed",
          previousRunStatus: "queued",
          done: true,
          current: 3,
          summary: summary(),
        }),
      },
    ]);
    const after = reduceAll(
      [
        { type: "event", event: event({ runId: "run-2", seq: 4 }) },
        { type: "invalidEvent", now: 0 },
        { type: "rejectedInProgress", now: 0 },
        { type: "dismissed", runId: "run-other" },
        { type: "clearNotice" },
      ],
      done,
    );
    expect(after.displayed?.runId).toBe("run-1");
    expect(after.displayed?.summary).toEqual(summary());

    const dismissed = reduceRun(after, { type: "dismissed", runId: "run-1" });
    expect(dismissed.displayed).toBeNull();
    expect(dismissed.lastSeq).toBe(0);
  });

  it("does not dismiss a non-terminal run", () => {
    const s = reduceRun(initialRunViewState, accepted());
    expect(reduceRun(s, { type: "dismissed", runId: "run-1" })).toBe(s);
  });

  it("a newer accepted run supersedes the terminal summary", () => {
    const s = reduceRun(terminalState(), accepted(snapshot({ runId: "run-2", sourceRunId: "run-1" })));
    expect(s.displayed?.runId).toBe("run-2");
    expect(s.displayed?.summary).toBeUndefined();
  });
});

describe("reduceRun: filter and retry selection", () => {
  it("attention filter shows exactly unknown and error rows", () => {
    const s = reduceRun(terminalState(), { type: "setFilter", filter: "attention" });
    expect(visiblePostings(s).map((p) => p.jobId)).toEqual(["unk", "err"]);
  });

  it("limits retry selection to eligible rows", () => {
    let s = terminalState();
    s = reduceRun(s, { type: "toggleRetry", jobId: "open" });
    expect(s.retrySelection.size).toBe(0);
    s = reduceAll(
      [
        { type: "toggleRetry", jobId: "can" },
        { type: "selectAllNeedingAttention" },
      ],
      s,
    );
    expect(retrySelectionIds(s)).toEqual(["unk", "err", "can"]);
    expect(canRetry(s)).toBe(true);
    s = reduceRun(s, { type: "toggleRetry", jobId: "can" });
    expect(retrySelectionIds(s)).toEqual(["unk", "err"]);
    expect(canRetry(reduceRun(s, { type: "clearRetrySelection" }))).toBe(false);
  });
});

// ---- Selectors ----

describe("predicates", () => {
  it("canCancel is true only for queued and active", () => {
    const expected: Record<RunStatus, boolean> = {
      queued: true,
      active: true,
      canceling: false,
      canceled: false,
      completed: false,
      completed_with_errors: false,
      error: false,
    };
    for (const status of RUN_STATUSES) {
      expect(canCancel(status)).toBe(expected[status]);
    }
    expect(canCancel(null)).toBe(false);
  });

  it("retry eligibility and attention match the Rust ledger predicates", () => {
    for (const status of POSTING_STATUSES) {
      for (const postingState of ["active", "inactive", "unknown", undefined] as const) {
        const row = { status, postingState };
        expect(isRetryEligible(row)).toBe(
          postingState === "unknown" || status === "error" || status === "canceled",
        );
        expect(needsAttention(row)).toBe(postingState === "unknown" || status === "error");
      }
    }
  });
});

describe("labels and view model", () => {
  it("every run and posting status has a non-empty label", () => {
    for (const status of RUN_STATUSES) expect(RUN_STATUS_LABELS[status]).not.toBe("");
    for (const status of POSTING_STATUSES) expect(POSTING_STATUS_LABELS[status]).not.toBe("");
  });

  it("completed rows read with their posting state", () => {
    expect(postingRowStatusText({ status: "completed", postingState: "active" })).toBe("Done: Open");
    expect(postingRowStatusText({ status: "completed", postingState: "inactive" })).toBe("Done: Closed");
    expect(postingRowStatusText({ status: "completed", postingState: "unknown" })).toBe("Done: Couldn't confirm");
    expect(postingRowStatusText({ status: "active" })).toBe("Checking");
  });

  it("runViewModel carries id, type, status, stage, counts, and elapsed time", () => {
    const vm = runViewModel(snapshot({ runStatus: "active", current: 1, elapsedMs: 65_000 }));
    expect(vm).toMatchObject({
      runId: "run-1",
      runTypeLabel: "Check postings",
      statusLabel: "Running",
      stageLabel: "Posting checks",
      completed: 1,
      total: 3,
      elapsedLabel: "1m 05s",
      canCancel: true,
      terminal: false,
    });
    const live = runViewModel(snapshot({ runStatus: "active" }), Date.parse("2026-01-01T00:00:42Z"));
    expect(live.elapsedLabel).toBe("42s");
  });

  it("formats elapsed durations compactly", () => {
    expect(formatElapsed(0)).toBe("0s");
    expect(formatElapsed(59_999)).toBe("59s");
    expect(formatElapsed(3_720_000)).toBe("1h 02m");
  });
});

// ---- Announcements ----

describe("buildAnnouncements", () => {
  function progressAt(state: RunViewState, seq: number, current: number): RunViewState {
    return reduceRun(state, { type: "event", event: event({ seq, current, total: 100, postingTotal: 100 }) });
  }

  it("announces status changes politely and throttles progress to 10% steps or 5 s", () => {
    const start = reduceAll([accepted(snapshot({ total: 100, postingTotal: 100 }))]);
    const first = buildAnnouncements(initialRunViewState, start, initialAnnouncementThrottle, 0);
    expect(first.polite).toContain("Check postings queued");

    const active = progressAt(start, 2, 1);
    const toActive = buildAnnouncements(start, active, first.throttle, 100);
    expect(toActive.polite).toContain("running");

    // Same 10% bucket, under 5 s: silent.
    const small = progressAt(active, 3, 5);
    const quiet = buildAnnouncements(active, small, toActive.throttle, 1_000);
    expect(quiet.polite).toBeUndefined();

    // Crossing into the next 10% bucket: announced.
    const step = progressAt(small, 4, 12);
    const stepped = buildAnnouncements(small, step, quiet.throttle, 1_500);
    expect(stepped.polite).toBe("Check postings: 12 of 100 postings checked");

    // Same bucket again but 5 s later: announced.
    const later = progressAt(step, 5, 14);
    expect(buildAnnouncements(step, later, stepped.throttle, 3_000).polite).toBeUndefined();
    expect(buildAnnouncements(step, later, stepped.throttle, 6_500).polite).toBe(
      "Check postings: 14 of 100 postings checked",
    );
  });

  it("announces each posting and run error assertively exactly once", () => {
    const start = reduceRun(initialRunViewState, accepted());
    const t0 = buildAnnouncements(initialRunViewState, start, initialAnnouncementThrottle, 0);

    const errored = reduceRun(start, {
      type: "event",
      event: event({ seq: 2, postings: [posting("a", { status: "error", reason: "persistence failure" })] }),
    });
    const a1 = buildAnnouncements(start, errored, t0.throttle, 10);
    expect(a1.assertive).toEqual(["Error checking Engineer a at Acme: persistence failure"]);
    expect(a1.announcedKeys).toEqual(["run-1:a"]);
    const recorded = reduceRun(errored, { type: "announced", keys: a1.announcedKeys });

    // The same error delivered again (duplicate event or snapshot) is not re-announced.
    const again = reduceRun(recorded, { type: "snapshotLoaded", snapshot: { ...recorded.displayed!, seq: 3 } });
    expect(buildAnnouncements(recorded, again, a1.throttle, 20).assertive).toEqual([]);

    // A run Error is announced once too.
    const failed = reduceRun(again, {
      type: "event",
      event: event({ seq: 4, runStatus: "error", previousRunStatus: "active", errorReason: "runner_interrupted" }),
    });
    const a2 = buildAnnouncements(again, failed, a1.throttle, 30);
    expect(a2.assertive).toEqual(["Check postings failed: runner_interrupted"]);
    expect(a2.polite).toBeDefined();
    const recorded2 = reduceRun(failed, { type: "announced", keys: a2.announcedKeys });
    expect(buildAnnouncements(failed, recorded2, a2.throttle, 40).assertive).toEqual([]);
  });
});
