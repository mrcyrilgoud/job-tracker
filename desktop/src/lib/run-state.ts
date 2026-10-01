/**
 * RunMonitor state: a pure reducer, selectors, and the live-region announcement builder.
 *
 * Nothing in this module touches React, Tauri, or timers. Time comes in as a
 * parameter (`now`, epoch milliseconds) so every function stays deterministic.
 *
 * Reducer rules (design.md, "Component 11: Frontend"):
 * - An event whose `runId`/`runType` differ from the displayed run is ignored,
 *   except a `seq = 1` Queued event for a different run, which takes over (Req 2.6, 2.9, 11.10).
 * - `seq <= lastSeq` is a duplicate and is ignored. `seq > lastSeq + 1` is a gap: the event
 *   is still applied and `needsReconcile` is set (Req 2.7).
 * - Invalid or unsupported-version input keeps the last valid state and sets a transient
 *   notice (Req 2.8, 11.8, 11.9). A failed snapshot retrieval does the same (Req 11.12).
 * - A loaded snapshot replaces the displayed state and clears `needsReconcile` (Req 11.11).
 * - `rejectedInProgress` never creates a displayed run (Req 4.5).
 * - A terminal run stays displayed until dismissed or superseded (Req 2.3, 2.9).
 */

import type {
  LegacyStage,
  PostingCounts,
  PostingProgress,
  PostingStateValue,
  PostingStatus,
  RunAccepted,
  RunProgressEvent,
  RunSnapshot,
  RunStatus,
  RunType,
  StageName,
  StageOutcome,
  StageProgress,
} from "@/lib/run-contract";

// ---- State ----

export type NoticeKind =
  | "validation"
  | "compatibility"
  | "retrieval"
  | "inProgress"
  | "action"
  | "refresh";

export type RunNotice = {
  kind: NoticeKind;
  message: string;
  /** Epoch ms after which `clearNotice` removes the notice. */
  expiresAt: number;
};

export type RunFilter = "all" | "attention";

export type RunViewState = {
  /** Last valid displayed run. */
  displayed: RunSnapshot | null;
  /** True while a start command is awaiting the backend response. */
  startPending: boolean;
  lastSeq: number;
  needsReconcile: boolean;
  notice?: RunNotice;
  /** Assertive announcement keys already spoken: `runId:jobId` and `runId:run`. */
  announcedErrors: ReadonlySet<string>;
  filter: RunFilter;
  /** Job ids selected for retry. Always a subset of the displayed retry-eligible rows. */
  retrySelection: ReadonlySet<string>;
};

/** How long a transient notice stays before `clearNotice` expires it. */
export const NOTICE_TTL_MS = 6_000;

export const IN_PROGRESS_MESSAGE = "Another run is in progress";

export const initialRunViewState: RunViewState = {
  displayed: null,
  startPending: false,
  lastSeq: 0,
  needsReconcile: false,
  announcedErrors: new Set(),
  filter: "all",
  retrySelection: new Set(),
};

// ---- Actions ----

export type RunAction =
  | { type: "accepted"; accepted: RunAccepted }
  | { type: "startPending" }
  | { type: "startSettled" }
  | { type: "event"; event: RunProgressEvent }
  /** `null` means the backend has no current run to display. */
  | { type: "snapshotLoaded"; snapshot: RunSnapshot | null }
  | { type: "snapshotFailed"; message?: string; now: number }
  | { type: "invalidEvent"; message?: string; now: number }
  | { type: "unsupportedVersion"; message?: string; now: number }
  | { type: "actionFailed"; message?: string; now: number }
  | { type: "dismissed"; runId: string }
  | { type: "rejectedInProgress"; now: number }
  /** Removes the notice once expired. Without `now`, removes it unconditionally. */
  | { type: "clearNotice"; now?: number }
  | { type: "refreshFailed"; message?: string; now: number }
  | { type: "setFilter"; filter: RunFilter }
  | { type: "toggleRetry"; jobId: string }
  | { type: "selectAllNeedingAttention" }
  | { type: "clearRetrySelection" }
  /** Records assertive keys returned by `buildAnnouncements` so each is spoken once. */
  | { type: "announced"; keys: readonly string[] };

// ---- Predicates ----

const TERMINAL_RUN_STATUSES: ReadonlySet<RunStatus> = new Set([
  "canceled",
  "completed",
  "completed_with_errors",
  "error",
]);

export function isTerminalRunStatus(status: RunStatus): boolean {
  return TERMINAL_RUN_STATUSES.has(status);
}

/** Cancel is enabled only for Queued and Active runs (Req 5.1, 5.2). */
export function canCancel(status: RunStatus | null | undefined): boolean {
  return status === "queued" || status === "active";
}

type RowStatus = { status: PostingStatus; postingState?: PostingStateValue };

/** Mirrors Rust `ledger::is_retry_eligible`: Unknown state, Error status, or Canceled status (Req 5.8). */
export function isRetryEligible(row: RowStatus): boolean {
  return row.postingState === "unknown" || row.status === "error" || row.status === "canceled";
}

/** Mirrors Rust `ledger::needs_attention`: Unknown state or Error status (Req 9.7). */
export function needsAttention(row: RowStatus): boolean {
  return row.postingState === "unknown" || row.status === "error";
}

/** A non-terminal displayed run blocks both start controls (Req 12.2). */
export function isRunBusy(state: RunViewState): boolean {
  return state.startPending || (state.displayed !== null && !isTerminalRunStatus(state.displayed.runStatus));
}

// ---- Reducer ----

function notice(kind: NoticeKind, message: string, now: number): RunNotice {
  return { kind, message, expiresAt: now + NOTICE_TTL_MS };
}

/** Build a displayed snapshot from a `seq = 1` event, which carries the full posting list. */
function snapshotFromEvent(e: RunProgressEvent): RunSnapshot {
  const s: RunSnapshot = {
    version: e.version,
    runId: e.runId,
    runType: e.runType,
    runStatus: e.runStatus,
    seq: e.seq,
    stage: e.stage,
    message: e.message,
    current: e.current,
    total: e.total,
    done: e.done,
    startedAt: e.startedAt,
    elapsedMs: e.elapsedMs,
    postingCounts: e.postingCounts,
    postingTotal: e.postingTotal,
    postings: e.postings ?? [],
    // Events are only emitted by the process that owns the run.
    live: true,
    dismissed: false,
  };
  if (e.stages !== undefined) s.stages = e.stages;
  if (e.errorReason !== undefined) s.errorReason = e.errorReason;
  if (e.summary !== undefined) s.summary = e.summary;
  if (e.summary?.sourceRunId !== undefined) s.sourceRunId = e.summary.sourceRunId;
  return s;
}

/** Merge posting deltas by `jobId`, preserving ordinal order. Reports unknown ids. */
function mergePostings(
  current: readonly PostingProgress[],
  delta: readonly PostingProgress[],
): { postings: PostingProgress[]; unknownIds: boolean } {
  const byId = new Map(delta.map((p) => [p.jobId, p]));
  const postings = current.map((p) => {
    const next = byId.get(p.jobId);
    if (next === undefined) return p;
    byId.delete(p.jobId);
    return next;
  });
  const unknownIds = byId.size > 0;
  for (const extra of byId.values()) postings.push(extra);
  return { postings, unknownIds };
}

/** Apply a same-run event on top of the displayed snapshot. */
function applyEvent(s: RunSnapshot, e: RunProgressEvent): { snapshot: RunSnapshot; unknownIds: boolean } {
  const merged =
    e.seq === 1 && e.postings !== undefined
      ? { postings: [...e.postings], unknownIds: false }
      : mergePostings(s.postings, e.postings ?? []);

  const next: RunSnapshot = {
    version: e.version,
    runId: s.runId,
    runType: s.runType,
    runStatus: e.runStatus,
    seq: e.seq,
    stage: e.stage,
    message: e.message,
    current: e.current,
    total: e.total,
    done: e.done,
    startedAt: e.startedAt,
    elapsedMs: e.elapsedMs,
    postingCounts: e.postingCounts,
    postingTotal: e.postingTotal,
    postings: merged.postings,
    live: s.live,
    dismissed: s.dismissed,
  };
  const stages = e.stages ?? s.stages;
  if (stages !== undefined) next.stages = stages;
  // errorReason is present iff status is error, summary iff done: both follow the event.
  if (e.errorReason !== undefined) next.errorReason = e.errorReason;
  if (e.summary !== undefined) next.summary = e.summary;
  const sourceRunId = s.sourceRunId ?? e.summary?.sourceRunId;
  if (sourceRunId !== undefined) next.sourceRunId = sourceRunId;
  return { snapshot: next, unknownIds: merged.unknownIds };
}

/** Keep only selected ids that are still retry-eligible rows of `snapshot`. */
function pruneSelection(selection: ReadonlySet<string>, snapshot: RunSnapshot | null): ReadonlySet<string> {
  if (selection.size === 0) return selection;
  if (snapshot === null) return new Set();
  const eligible = new Set(snapshot.postings.filter(isRetryEligible).map((p) => p.jobId));
  const kept = [...selection].filter((id) => eligible.has(id));
  return kept.length === selection.size ? selection : new Set(kept);
}

/** Replace the displayed run. A different run resets the per-run view controls. */
function display(state: RunViewState, snapshot: RunSnapshot | null): RunViewState {
  const sameRun = snapshot !== null && state.displayed?.runId === snapshot.runId;
  return {
    ...state,
    displayed: snapshot,
    lastSeq: snapshot?.seq ?? 0,
    needsReconcile: false,
    filter: sameRun ? state.filter : "all",
    retrySelection: sameRun ? pruneSelection(state.retrySelection, snapshot) : new Set(),
  };
}

function reduceEvent(state: RunViewState, e: RunProgressEvent): RunViewState {
  const shown = state.displayed;
  const isNewRunStart = e.seq === 1 && e.runStatus === "queued";

  if (shown === null) {
    if (isNewRunStart) {
      return display(state, snapshotFromEvent(e));
    }
    // A mid-run event with no displayed run cannot be rendered without the full list.
    return state.needsReconcile ? state : { ...state, needsReconcile: true };
  }

  if (e.runId !== shown.runId || e.runType !== shown.runType) {
    if (isNewRunStart && e.runId !== shown.runId) {
      return display(state, snapshotFromEvent(e));
    }
    return state;
  }

  if (e.seq <= state.lastSeq) {
    return state;
  }

  const gap = e.seq > state.lastSeq + 1;
  const { snapshot, unknownIds } = applyEvent(shown, e);
  return {
    ...state,
    displayed: snapshot,
    lastSeq: e.seq,
    needsReconcile: state.needsReconcile || gap || unknownIds,
    retrySelection: pruneSelection(state.retrySelection, snapshot),
  };
}

function reduceAccepted(state: RunViewState, accepted: RunAccepted): RunViewState {
  const shown = state.displayed;
  const snapshot = accepted.snapshot;
  // A fresh acceptance supersedes an earlier "another run is in progress" notice.
  const base = state.notice?.kind === "inProgress" ? withoutNotice(state) : state;

  // Events for this run may have arrived before the command resolved; never regress.
  if (shown !== null && shown.runId === snapshot.runId && state.lastSeq >= snapshot.seq) {
    if (shown.sourceRunId === undefined && snapshot.sourceRunId !== undefined) {
      return { ...base, displayed: { ...shown, sourceRunId: snapshot.sourceRunId } };
    }
    return base;
  }
  return display(base, snapshot);
}

function withoutNotice(state: RunViewState): RunViewState {
  const next = { ...state };
  delete next.notice;
  return next;
}

function withNotice(state: RunViewState, kind: NoticeKind, message: string, now: number, reconcile: boolean): RunViewState {
  return {
    ...state,
    notice: notice(kind, message, now),
    needsReconcile: state.needsReconcile || reconcile,
  };
}

export function reduceRun(state: RunViewState, action: RunAction): RunViewState {
  switch (action.type) {
    case "startPending":
      return state.startPending ? state : { ...state, startPending: true };

    case "startSettled":
      return state.startPending ? { ...state, startPending: false } : state;

    case "accepted":
      return reduceAccepted(state, action.accepted);

    case "event":
      return reduceEvent(state, action.event);

    case "snapshotLoaded": {
      const snapshot = action.snapshot !== null && action.snapshot.dismissed ? null : action.snapshot;
      const next = display(state, snapshot);
      // A successful retrieval resolves an earlier retrieval notice.
      return next.notice?.kind === "retrieval" ? withoutNotice(next) : next;
    }

    case "snapshotFailed":
      return withNotice(
        state,
        "retrieval",
        action.message ?? "Couldn't load the latest run status. Showing the last known state.",
        action.now,
        false,
      );

    case "invalidEvent":
      return withNotice(
        state,
        "validation",
        action.message ?? "Received an invalid progress update. Showing the last known state.",
        action.now,
        true,
      );

    case "unsupportedVersion":
      return withNotice(
        state,
        "compatibility",
        action.message ?? "Received a progress update from an incompatible version. Showing the last known state.",
        action.now,
        true,
      );

    case "rejectedInProgress":
      // Never a displayed run; reconcile so the owning run is shown if it is visible.
      return withNotice(state, "inProgress", IN_PROGRESS_MESSAGE, action.now, true);

    case "actionFailed":
      return withNotice(
        state,
        "action",
        action.message ?? "The run action failed. Showing the last known state.",
        action.now,
        false,
      );

    case "refreshFailed":
      return withNotice(
        state,
        "refresh",
        action.message ?? "Couldn't refresh job data. Showing the previous data.",
        action.now,
        false,
      );

    case "clearNotice": {
      if (state.notice === undefined) return state;
      if (action.now !== undefined && state.notice.expiresAt > action.now) return state;
      return withoutNotice(state);
    }

    case "dismissed": {
      const shown = state.displayed;
      if (shown === null || shown.runId !== action.runId || !isTerminalRunStatus(shown.runStatus)) {
        return state;
      }
      return display(state, null);
    }

    case "setFilter":
      return state.filter === action.filter ? state : { ...state, filter: action.filter };

    case "toggleRetry": {
      const row = state.displayed?.postings.find((p) => p.jobId === action.jobId);
      if (row === undefined || !isRetryEligible(row)) return state;
      const selection = new Set(state.retrySelection);
      if (selection.has(action.jobId)) selection.delete(action.jobId);
      else selection.add(action.jobId);
      return { ...state, retrySelection: selection };
    }

    case "selectAllNeedingAttention": {
      const ids = (state.displayed?.postings ?? []).filter(needsAttention).map((p) => p.jobId);
      return { ...state, retrySelection: new Set([...state.retrySelection, ...ids]) };
    }

    case "clearRetrySelection":
      return state.retrySelection.size === 0 ? state : { ...state, retrySelection: new Set() };

    case "announced": {
      const fresh = action.keys.filter((k) => !state.announcedErrors.has(k));
      if (fresh.length === 0) return state;
      return { ...state, announcedErrors: new Set([...state.announcedErrors, ...fresh]) };
    }

    default: {
      const _exhaustive: never = action;
      return _exhaustive;
    }
  }
}

// ---- Selectors ----

/** Rows shown under the current filter, in ordinal order (Req 9.7). */
export function visiblePostings(state: RunViewState): PostingProgress[] {
  const rows = state.displayed?.postings ?? [];
  return state.filter === "attention" ? rows.filter(needsAttention) : [...rows];
}

export function retryEligiblePostings(snapshot: RunSnapshot | null): PostingProgress[] {
  return (snapshot?.postings ?? []).filter(isRetryEligible);
}

export function attentionPostings(snapshot: RunSnapshot | null): PostingProgress[] {
  return (snapshot?.postings ?? []).filter(needsAttention);
}

/** Retry needs a terminal source run and a non-empty selection. */
export function canRetry(state: RunViewState): boolean {
  return (
    state.displayed !== null && isTerminalRunStatus(state.displayed.runStatus) && state.retrySelection.size > 0
  );
}

/** Selected ids in the run's ordinal order, for `retry_run_cmd`. */
export function retrySelectionIds(state: RunViewState): string[] {
  return (state.displayed?.postings ?? []).filter((p) => state.retrySelection.has(p.jobId)).map((p) => p.jobId);
}

// ---- Labels (Req 12.6: every status has text) ----

export const RUN_STATUS_LABELS: Record<RunStatus, string> = {
  queued: "Queued",
  active: "Running",
  canceling: "Canceling",
  canceled: "Canceled",
  completed: "Completed",
  completed_with_errors: "Completed with errors",
  error: "Error",
};

export const POSTING_STATUS_LABELS: Record<PostingStatus, string> = {
  queued: "Queued",
  active: "Checking",
  completed: "Done",
  error: "Error",
  canceled: "Canceled",
};

/** Labels for a checked result. `unknown` here means a check ran but was inconclusive. */
export const POSTING_STATE_LABELS: Record<PostingStateValue, string> = {
  active: "Open",
  inactive: "Closed",
  unknown: "Couldn't confirm",
};

export const RUN_TYPE_LABELS: Record<RunType, string> = {
  jobsCycle: "Run jobs",
  postingCheck: "Check postings",
  careerCheck: "Check career sources",
};

export const STAGE_LABELS: Record<LegacyStage, string> = {
  postings: "Posting checks",
  watches: "ATS watches",
  careers: "Careers pages",
  csv: "CSV sync",
  cycle: "Finishing",
};

export const STAGE_OUTCOME_LABELS: Record<StageOutcome, string> = {
  not_started: "Not started",
  in_progress: "In progress",
  succeeded: "Succeeded",
  failed: "Failed",
  skipped: "Skipped",
};

export function runStatusLabel(status: RunStatus): string {
  return RUN_STATUS_LABELS[status];
}

export function postingStatusLabel(status: PostingStatus): string {
  return POSTING_STATUS_LABELS[status];
}

export function postingStateLabel(state: PostingStateValue): string {
  return POSTING_STATE_LABELS[state];
}

/** "Done: Open", "Checking", "Error", ... */
export function postingRowStatusText(row: RowStatus): string {
  const status = POSTING_STATUS_LABELS[row.status];
  if (row.status === "completed" && row.postingState !== undefined) {
    return `${status}: ${POSTING_STATE_LABELS[row.postingState]}`;
  }
  return status;
}

/** Compact elapsed duration: "0s", "45s", "2m 05s", "1h 02m". */
export function formatElapsed(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) return `${hours}h ${String(minutes).padStart(2, "0")}m`;
  if (minutes > 0) return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
  return `${seconds}s`;
}

export type StageView = {
  name: StageName;
  label: string;
  outcome: StageOutcome;
  outcomeLabel: string;
  current: number;
  total: number;
  error?: string;
};

export type RunViewModel = {
  runId: string;
  runType: RunType;
  runTypeLabel: string;
  status: RunStatus;
  statusLabel: string;
  terminal: boolean;
  stage: LegacyStage;
  stageLabel: string;
  /** Legacy `current` / `total` of the current stage. */
  completed: number;
  total: number;
  postingCounts: PostingCounts;
  postingTotal: number;
  elapsedMs: number;
  elapsedLabel: string;
  canCancel: boolean;
  errorReason?: string;
  stages?: StageView[];
};

function stageView(s: StageProgress): StageView {
  const v: StageView = {
    name: s.name,
    label: STAGE_LABELS[s.name],
    outcome: s.outcome,
    outcomeLabel: STAGE_OUTCOME_LABELS[s.outcome],
    current: s.current,
    total: s.total,
  };
  if (s.error !== undefined) v.error = s.error;
  return v;
}

/**
 * Presentation model for the run panel (Req 2.1, 2.2). With `now`, a non-terminal run's
 * elapsed time advances from `startedAt` instead of freezing at the last event.
 */
export function runViewModel(snapshot: RunSnapshot, now?: number): RunViewModel {
  const terminal = isTerminalRunStatus(snapshot.runStatus);
  let elapsedMs = terminal && snapshot.summary !== undefined ? snapshot.summary.durationMs : snapshot.elapsedMs;
  if (!terminal && now !== undefined) {
    const started = Date.parse(snapshot.startedAt);
    if (Number.isFinite(started)) elapsedMs = Math.max(elapsedMs, now - started);
  }
  const vm: RunViewModel = {
    runId: snapshot.runId,
    runType: snapshot.runType,
    runTypeLabel: RUN_TYPE_LABELS[snapshot.runType],
    status: snapshot.runStatus,
    statusLabel: RUN_STATUS_LABELS[snapshot.runStatus],
    terminal,
    stage: snapshot.stage,
    stageLabel: STAGE_LABELS[snapshot.stage],
    completed: snapshot.current,
    total: snapshot.total,
    postingCounts: snapshot.postingCounts,
    postingTotal: snapshot.postingTotal,
    elapsedMs,
    elapsedLabel: formatElapsed(elapsedMs),
    canCancel: canCancel(snapshot.runStatus),
  };
  if (snapshot.errorReason !== undefined) vm.errorReason = snapshot.errorReason;
  if (snapshot.stages !== undefined) vm.stages = snapshot.stages.map(stageView);
  return vm;
}

// ---- Announcements (Req 12.3, 12.4) ----

/** Polite progress announcements happen at most once per 10% step or this interval. */
export const PROGRESS_ANNOUNCE_INTERVAL_MS = 5_000;

/** Bookkeeping for throttled polite progress announcements. Keep it next to the state. */
export type AnnouncementThrottle = {
  runId: string | null;
  stage: LegacyStage | null;
  /** Last announced progress, and its 10% bucket (0..10). */
  current: number;
  total: number;
  bucket: number;
  lastPoliteAt: number | null;
};

export const initialAnnouncementThrottle: AnnouncementThrottle = {
  runId: null,
  stage: null,
  current: 0,
  total: 0,
  bucket: 0,
  lastPoliteAt: null,
};

export type Announcements = {
  /** Text for the polite `role="status"` region, when something changed worth saying. */
  polite?: string;
  /** Texts for the assertive region, each spoken once. */
  assertive: string[];
  /** Keys for `assertive`; dispatch `{ type: "announced", keys }` after speaking. */
  announcedKeys: string[];
  throttle: AnnouncementThrottle;
};

export function runErrorKey(runId: string): string {
  return `${runId}:run`;
}

export function postingErrorKey(runId: string, jobId: string): string {
  return `${runId}:${jobId}`;
}

function bucketOf(current: number, total: number): number {
  if (total <= 0) return 10;
  return Math.min(10, Math.floor((current * 10) / total));
}

function progressText(s: RunSnapshot): string {
  const type = RUN_TYPE_LABELS[s.runType];
  if (s.runType === "postingCheck" || s.stage === "postings") {
    return `${type}: ${s.current} of ${s.total} postings checked`;
  }
  return `${type}: ${STAGE_LABELS[s.stage]} ${s.current} of ${s.total}`;
}

function statusText(s: RunSnapshot): string {
  const type = RUN_TYPE_LABELS[s.runType];
  const status = RUN_STATUS_LABELS[s.runStatus].toLowerCase();
  const summary = s.summary;
  if (summary !== undefined && s.runType !== "careerCheck") {
    const o = summary.postingOutcomes;
    return (
      `${type} ${status}. ${o.active} open, ${o.closed} closed, ${o.unknown} couldn't confirm, ` +
      `${o.error} errors, ${o.canceled} canceled.`
    );
  }
  if (isTerminalRunStatus(s.runStatus)) {
    return `${type} ${status}.`;
  }
  return `${type} ${status}. ${progressText(s)}.`;
}

function throttleFor(s: RunSnapshot, now: number): AnnouncementThrottle {
  return {
    runId: s.runId,
    stage: s.stage,
    current: s.current,
    total: s.total,
    bucket: bucketOf(s.current, s.total),
    lastPoliteAt: now,
  };
}

/**
 * Compute live-region announcements for a state change.
 *
 * - Polite: every run-status change (or a newly displayed run) is announced. Aggregate progress
 *   is announced only when it crosses a new 10% step, changes stage, or at least 5 s have passed
 *   since the last polite announcement.
 * - Assertive: each posting Error (`runId:jobId`) and run Error (`runId:run`) is announced once,
 *   using `next.announcedErrors` to skip keys already spoken.
 */
export function buildAnnouncements(
  prev: RunViewState,
  next: RunViewState,
  throttle: AnnouncementThrottle,
  now: number,
): Announcements {
  const s = next.displayed;
  if (s === null) {
    return { assertive: [], announcedKeys: [], throttle: initialAnnouncementThrottle };
  }

  const assertive: string[] = [];
  const announcedKeys: string[] = [];
  const speak = (key: string, text: string) => {
    if (next.announcedErrors.has(key) || announcedKeys.includes(key)) return;
    announcedKeys.push(key);
    assertive.push(text);
  };
  for (const p of s.postings) {
    if (p.status === "error") {
      const reason = p.reason ? `: ${p.reason}` : "";
      speak(postingErrorKey(s.runId, p.jobId), `Error checking ${p.title} at ${p.companyName}${reason}`);
    }
  }
  if (s.runStatus === "error") {
    const reason = s.errorReason ? `: ${s.errorReason}` : "";
    speak(runErrorKey(s.runId), `${RUN_TYPE_LABELS[s.runType]} failed${reason}`);
  }

  const before = prev.displayed;
  const statusChanged =
    before === null ||
    before.runId !== s.runId ||
    before.runStatus !== s.runStatus ||
    throttle.runId !== s.runId;
  if (statusChanged) {
    return { polite: statusText(s), assertive, announcedKeys, throttle: throttleFor(s, now) };
  }

  const progressChanged =
    s.stage !== throttle.stage || s.current !== throttle.current || s.total !== throttle.total;
  if (!progressChanged || isTerminalRunStatus(s.runStatus)) {
    return { assertive, announcedKeys, throttle };
  }

  const bucket = bucketOf(s.current, s.total);
  const intervalElapsed =
    throttle.lastPoliteAt === null || now - throttle.lastPoliteAt >= PROGRESS_ANNOUNCE_INTERVAL_MS;
  if (s.stage !== throttle.stage || bucket > throttle.bucket || intervalElapsed) {
    return { polite: progressText(s), assertive, announcedKeys, throttle: throttleFor(s, now) };
  }
  return { assertive, announcedKeys, throttle };
}
