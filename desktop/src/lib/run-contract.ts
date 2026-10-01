/**
 * Progress_Contract v1: TypeScript mirror of `src-tauri/src/runs/progress.rs`.
 *
 * Wire rules (design.md, "Progress contract v1"):
 * - camelCase field names; `RunType` values are camelCase, every other enum is snake_case.
 * - Optional fields are absent, never `null`. A `null` optional is malformed.
 * - String bounds are UTF-8 byte lengths (the Rust `MAX_*_BYTES` constants).
 * - Counts are non-negative integers no greater than 2^53 − 1, and `current <= total`.
 *
 * The `parse*` functions never throw. They return a discriminated result:
 * `{ ok: true, value }` or `{ ok: false, kind, message }`, where `kind` is
 * `unsupported_version` for a well-formed version other than 1 and `invalid`
 * for everything else. Parsed values contain only contract fields; unknown
 * extra keys are dropped, matching serde's default behavior.
 */

export const PROGRESS_CONTRACT_VERSION = 1;
/** Unchanged Tauri event channel shared with the legacy runner. */
export const RUN_PROGRESS_EVENT = "jobs-runner-progress";

/** Run and job identifiers. */
export const MAX_ID_BYTES = 64;
/** Job title. */
export const MAX_TITLE_BYTES = 300;
/** Company name. */
export const MAX_COMPANY_BYTES = 200;
/** Posting, requested, and final URLs. */
export const MAX_URL_BYTES = 2048;
/** Legacy `message`. */
export const MAX_MESSAGE_BYTES = 500;
/** Classification_Reason, failure reason, run error reason, stage error. */
export const MAX_REASON_BYTES = 500;
/** Reason codes, failure categories, evidence-category values. */
export const MAX_CATEGORY_BYTES = 64;
/** `Number.MAX_SAFE_INTEGER` (2^53 − 1). */
export const MAX_COUNT = 9_007_199_254_740_991;

export const CONTRACT_BOUNDS = {
  MAX_ID_BYTES,
  MAX_TITLE_BYTES,
  MAX_COMPANY_BYTES,
  MAX_URL_BYTES,
  MAX_MESSAGE_BYTES,
  MAX_REASON_BYTES,
  MAX_CATEGORY_BYTES,
} as const;

export const RUN_TYPES = ["jobsCycle", "postingCheck", "careerCheck"] as const;
export const RUN_STATUSES = [
  "queued",
  "active",
  "canceling",
  "canceled",
  "completed",
  "completed_with_errors",
  "error",
] as const;
export const POSTING_STATUSES = ["queued", "active", "completed", "error", "canceled"] as const;
export const POSTING_STATES = ["active", "inactive", "unknown"] as const;
export const STAGE_NAMES = ["postings", "watches", "careers", "csv"] as const;
export const LEGACY_STAGES = [...STAGE_NAMES, "cycle"] as const;
export const STAGE_OUTCOMES = ["not_started", "in_progress", "succeeded", "failed", "skipped"] as const;
export const PROVIDERS = ["greenhouse", "lever", "ashby"] as const;
export const PROVIDER_SIGNALS = [
  "listed_open",
  "listed_closed",
  "absent_from_listing",
  "listing_unavailable",
] as const;
/** `ContentSignal` names in canonical (declaration) order. */
export const CONTENT_SIGNALS = [
  "title_match",
  "company_match",
  "apply_enabled",
  "apply_disabled",
  "closure_copy_matched",
  "closure_copy_unmatched",
  "consent_page",
  "auth_page",
  "anti_bot",
  "access_denied",
  "generic_careers",
] as const;
/** Evidence `FailureCategory` names. */
export const FAILURE_CATEGORIES = [
  "timeout",
  "dns_timeout",
  "dns",
  "connect",
  "blocked_destination",
  "invalid_url",
  "redirect_failure",
  "too_many_redirects",
  "too_large",
  "provider_temporary",
  "provider_failure",
  "internal",
  "persistence",
] as const;

export type RunType = (typeof RUN_TYPES)[number];
export type RunStatus = (typeof RUN_STATUSES)[number];
export type PostingStatus = (typeof POSTING_STATUSES)[number];
/** UI label "Closed" corresponds to `inactive`. */
export type PostingStateValue = (typeof POSTING_STATES)[number];
export type StageName = (typeof STAGE_NAMES)[number];
export type LegacyStage = (typeof LEGACY_STAGES)[number];
export type StageOutcome = (typeof STAGE_OUTCOMES)[number];
export type EvidenceProvider = (typeof PROVIDERS)[number];
export type ProviderSignalKind = (typeof PROVIDER_SIGNALS)[number];
export type ContentSignal = (typeof CONTENT_SIGNALS)[number];
export type FailureCategory = (typeof FAILURE_CATEGORIES)[number];

export type PostingCounts = {
  queued: number;
  active: number;
  completed: number;
  error: number;
  canceled: number;
};

export type EvidenceProviderView = {
  provider: EvidenceProvider;
  signal: ProviderSignalKind;
  postingId: string;
  httpStatus?: number;
};

export type EvidenceView = {
  evidenceVersion: number;
  attemptedAt: string;
  requestedUrl: string;
  finalUrl?: string;
  httpStatus?: number;
  redirectStatuses?: number[];
  provider?: EvidenceProviderView;
  content?: ContentSignal[];
  failureCategory?: FailureCategory;
};

export type PostingProgress = {
  jobId: string;
  title: string;
  companyName: string;
  postingUrl: string;
  status: PostingStatus;
  /** Present iff `status === "completed"`. */
  postingState?: PostingStateValue;
  reasonCode?: string;
  /** Present iff `status` is completed or error. */
  reason?: string;
  /** Evidence failure category, or a run-level category such as `run_aborted`. */
  failureCategory?: string;
  evidence?: EvidenceView;
};

export type StageProgress = {
  name: StageName;
  outcome: StageOutcome;
  current: number;
  total: number;
  error?: string;
};

export type PostingOutcomes = {
  active: number;
  /** Posting_State `inactive`. */
  closed: number;
  unknown: number;
  error: number;
  canceled: number;
};

export type RunSummary = {
  runId: string;
  runType: RunType;
  status: RunStatus;
  startedAt: string;
  finishedAt: string;
  durationMs: number;
  postingOutcomes: PostingOutcomes;
  stateChanges: number;
  stages?: StageProgress[];
  sourceRunId?: string;
};

export type RunProgressEvent = {
  version: typeof PROGRESS_CONTRACT_VERSION;
  runId: string;
  runType: RunType;
  runStatus: RunStatus;
  /** Present iff this event carries a run-status transition. */
  previousRunStatus?: RunStatus;
  /** 1-based, +1 per event per run. */
  seq: number;
  emittedAt: string;
  // Legacy fields, same names and meanings as the pre-v1 runner event.
  stage: LegacyStage;
  message: string;
  current: number;
  total: number;
  /** True only on the terminal event. */
  done: boolean;
  startedAt: string;
  elapsedMs: number;
  /** Runs with stage progress, including Jobs_Cycle and Career_Check. */
  stages?: StageProgress[];
  postingCounts: PostingCounts;
  postingTotal: number;
  /** Full list when `seq === 1`, otherwise changed entries only. */
  postings?: PostingProgress[];
  /** Present iff `runStatus === "error"`. */
  errorReason?: string;
  /** Present iff `done`. */
  summary?: RunSummary;
};

export type RunSnapshot = {
  version: typeof PROGRESS_CONTRACT_VERSION;
  runId: string;
  runType: RunType;
  runStatus: RunStatus;
  seq: number;
  stage: LegacyStage;
  message: string;
  current: number;
  total: number;
  done: boolean;
  startedAt: string;
  elapsedMs: number;
  stages?: StageProgress[];
  postingCounts: PostingCounts;
  postingTotal: number;
  errorReason?: string;
  summary?: RunSummary;
  /** Always the full list, in ordinal order. */
  postings: PostingProgress[];
  /** True when this process owns the run and will emit events for it. */
  live: boolean;
  sourceRunId?: string;
  dismissed: boolean;
};

export type RunAccepted = {
  runId: string;
  snapshot: RunSnapshot;
};

export type ContractErrorKind = "unsupported_version" | "invalid";

export type ParseResult<T> =
  | { ok: true; value: T }
  | { ok: false; kind: ContractErrorKind; message: string };

/** Thrown by API wrappers when a backend response fails contract validation. */
export class RunContractError extends Error {
  readonly kind: ContractErrorKind;

  constructor(kind: ContractErrorKind, message: string) {
    super(message);
    this.name = "RunContractError";
    this.kind = kind;
  }
}

/** Unwrap a parse result, throwing [`RunContractError`] on failure. */
export function unwrapContract<T>(result: ParseResult<T>): T {
  if (result.ok) {
    return result.value;
  }
  throw new RunContractError(result.kind, result.message);
}

const encoder = new TextEncoder();

/** UTF-8 encoded length of `s`, the unit of every contract string bound. */
export function utf8ByteLength(s: string): number {
  return encoder.encode(s).length;
}

// ---- Internal decoder ----

/** Internal only; always caught by `run()` so no parser throws to its caller. */
class Invalid extends Error {
  readonly kind: ContractErrorKind;

  constructor(message: string, kind: ContractErrorKind = "invalid") {
    super(message);
    this.kind = kind;
  }
}

type Obj = Record<string, unknown>;

function run<T>(input: unknown, decode: (v: unknown) => T): ParseResult<T> {
  try {
    return { ok: true, value: decode(input) };
  } catch (err) {
    if (err instanceof Invalid) {
      return { ok: false, kind: err.kind, message: err.message };
    }
    return { ok: false, kind: "invalid", message: err instanceof Error ? err.message : String(err) };
  }
}

function describe(v: unknown): string {
  if (v === null) return "null";
  if (Array.isArray(v)) return "array";
  return typeof v;
}

function asObject(v: unknown, path: string): Obj {
  if (typeof v !== "object" || v === null || Array.isArray(v)) {
    throw new Invalid(`${path} must be an object, got ${describe(v)}`);
  }
  return v as Obj;
}

function has(o: Obj, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(o, key);
}

function required<T>(o: Obj, key: string, path: string, decode: (v: unknown, p: string) => T): T {
  const p = `${path}.${key}`;
  if (!has(o, key) || o[key] === undefined) {
    throw new Invalid(`${p} is required`);
  }
  return decode(o[key], p);
}

/** Absent → undefined. Present `null` is malformed: optionals are absent, never null. */
function optional<T>(o: Obj, key: string, path: string, decode: (v: unknown, p: string) => T): T | undefined {
  if (!has(o, key) || o[key] === undefined) {
    return undefined;
  }
  const p = `${path}.${key}`;
  if (o[key] === null) {
    throw new Invalid(`${p} must be absent rather than null`);
  }
  return decode(o[key], p);
}

/** Assign `value` to `key` only when present, so absent optionals stay absent. */
function setOpt<T extends object, K extends keyof T>(target: T, key: K, value: T[K] | undefined): void {
  if (value !== undefined) {
    target[key] = value;
  }
}

function str(v: unknown, path: string): string {
  if (typeof v !== "string") {
    throw new Invalid(`${path} must be a string, got ${describe(v)}`);
  }
  return v;
}

function boundedStr(max: number) {
  return (v: unknown, path: string): string => {
    const s = str(v, path);
    const bytes = utf8ByteLength(s);
    if (bytes > max) {
      throw new Invalid(`${path} is ${bytes} bytes, exceeds ${max}`);
    }
    return s;
  };
}

const idStr = boundedStr(MAX_ID_BYTES);
const titleStr = boundedStr(MAX_TITLE_BYTES);
const companyStr = boundedStr(MAX_COMPANY_BYTES);
const urlStr = boundedStr(MAX_URL_BYTES);
const messageStr = boundedStr(MAX_MESSAGE_BYTES);
const reasonStr = boundedStr(MAX_REASON_BYTES);
const categoryStr = boundedStr(MAX_CATEGORY_BYTES);

function bool(v: unknown, path: string): boolean {
  if (typeof v !== "boolean") {
    throw new Invalid(`${path} must be a boolean, got ${describe(v)}`);
  }
  return v;
}

function intInRange(max: number, label: string) {
  return (v: unknown, path: string): number => {
    if (typeof v !== "number" || !Number.isInteger(v)) {
      throw new Invalid(`${path} must be an integer, got ${describe(v)}`);
    }
    if (v < 0 || v > max) {
      throw new Invalid(`${path} must be ${label}, got ${v}`);
    }
    return v;
  };
}

/** BoundedCount / u64 count: 0 ..= 2^53 − 1. */
const count = intInRange(MAX_COUNT, `between 0 and ${MAX_COUNT}`);
/** u16 (HTTP status). */
const u16 = intInRange(65_535, "between 0 and 65535");
/** u32 (evidence version). */
const u32 = intInRange(4_294_967_295, "between 0 and 4294967295");

function oneOf<const T extends readonly string[]>(values: T, label: string) {
  const set = new Set<string>(values);
  return (v: unknown, path: string): T[number] => {
    const s = str(v, path);
    if (!set.has(s)) {
      throw new Invalid(`${path} is not a known ${label}: ${JSON.stringify(s)}`);
    }
    return s as T[number];
  };
}

const runType = oneOf(RUN_TYPES, "run type");
const runStatus = oneOf(RUN_STATUSES, "run status");
const postingStatus = oneOf(POSTING_STATUSES, "posting status");
const postingState = oneOf(POSTING_STATES, "posting state");
const stageName = oneOf(STAGE_NAMES, "stage name");
const legacyStage = oneOf(LEGACY_STAGES, "stage");
const stageOutcome = oneOf(STAGE_OUTCOMES, "stage outcome");
const provider = oneOf(PROVIDERS, "provider");
const providerSignal = oneOf(PROVIDER_SIGNALS, "provider signal");
const contentSignal = oneOf(CONTENT_SIGNALS, "content signal");
const failureCategory = oneOf(FAILURE_CATEGORIES, "failure category");

function arrayOf<T>(decode: (v: unknown, p: string) => T) {
  return (v: unknown, path: string): T[] => {
    if (!Array.isArray(v)) {
      throw new Invalid(`${path} must be an array, got ${describe(v)}`);
    }
    return v.map((item, i) => decode(item, `${path}[${i}]`));
  };
}

function currentWithinTotal(current: number, total: number, path: string): void {
  if (current > total) {
    throw new Invalid(`${path}.current (${current}) exceeds ${path}.total (${total})`);
  }
}

function validateRunConditionals(
  runStatus: RunStatus,
  done: boolean,
  summary: RunSummary | undefined,
  errorReason: string | undefined,
  path: string,
): void {
  if (done !== (summary !== undefined)) {
    throw new Invalid(`${path}.summary must be present exactly when done is true`);
  }
  if ((runStatus === "error") !== (errorReason !== undefined)) {
    throw new Invalid(`${path}.errorReason must be present exactly when runStatus is error`);
  }
}

/** Checked before anything else so a future version reports a compatibility error. */
function version(o: Obj, path: string): typeof PROGRESS_CONTRACT_VERSION {
  const v = required(o, "version", path, (x, p) => {
    if (typeof x !== "number" || !Number.isInteger(x) || x < 0) {
      throw new Invalid(`${p} must be a non-negative integer, got ${describe(x)}`);
    }
    return x;
  });
  if (v !== PROGRESS_CONTRACT_VERSION) {
    throw new Invalid(
      `unsupported progress contract version ${v} (expected ${PROGRESS_CONTRACT_VERSION})`,
      "unsupported_version",
    );
  }
  return PROGRESS_CONTRACT_VERSION;
}

function decodePostingCounts(v: unknown, path: string): PostingCounts {
  const o = asObject(v, path);
  return {
    queued: required(o, "queued", path, count),
    active: required(o, "active", path, count),
    completed: required(o, "completed", path, count),
    error: required(o, "error", path, count),
    canceled: required(o, "canceled", path, count),
  };
}

function decodeEvidenceProvider(v: unknown, path: string): EvidenceProviderView {
  const o = asObject(v, path);
  const out: EvidenceProviderView = {
    provider: required(o, "provider", path, provider),
    signal: required(o, "signal", path, providerSignal),
    postingId: required(o, "postingId", path, idStr),
  };
  setOpt(out, "httpStatus", optional(o, "httpStatus", path, u16));
  return out;
}

function decodeEvidence(v: unknown, path: string): EvidenceView {
  const o = asObject(v, path);
  const out: EvidenceView = {
    evidenceVersion: required(o, "evidenceVersion", path, u32),
    attemptedAt: required(o, "attemptedAt", path, str),
    requestedUrl: required(o, "requestedUrl", path, urlStr),
  };
  setOpt(out, "finalUrl", optional(o, "finalUrl", path, urlStr));
  setOpt(out, "httpStatus", optional(o, "httpStatus", path, u16));
  setOpt(out, "redirectStatuses", optional(o, "redirectStatuses", path, arrayOf(u16)));
  setOpt(out, "provider", optional(o, "provider", path, decodeEvidenceProvider));
  setOpt(out, "content", optional(o, "content", path, arrayOf(contentSignal)));
  setOpt(out, "failureCategory", optional(o, "failureCategory", path, failureCategory));
  return out;
}

function decodePosting(v: unknown, path: string): PostingProgress {
  const o = asObject(v, path);
  const out: PostingProgress = {
    jobId: required(o, "jobId", path, idStr),
    title: required(o, "title", path, titleStr),
    companyName: required(o, "companyName", path, companyStr),
    postingUrl: required(o, "postingUrl", path, urlStr),
    status: required(o, "status", path, postingStatus),
  };
  setOpt(out, "postingState", optional(o, "postingState", path, postingState));
  setOpt(out, "reasonCode", optional(o, "reasonCode", path, categoryStr));
  setOpt(out, "reason", optional(o, "reason", path, reasonStr));
  setOpt(out, "failureCategory", optional(o, "failureCategory", path, categoryStr));
  setOpt(out, "evidence", optional(o, "evidence", path, decodeEvidence));
  return out;
}

function decodeStage(v: unknown, path: string): StageProgress {
  const o = asObject(v, path);
  const out: StageProgress = {
    name: required(o, "name", path, stageName),
    outcome: required(o, "outcome", path, stageOutcome),
    current: required(o, "current", path, count),
    total: required(o, "total", path, count),
  };
  currentWithinTotal(out.current, out.total, path);
  setOpt(out, "error", optional(o, "error", path, reasonStr));
  return out;
}

function decodeOutcomes(v: unknown, path: string): PostingOutcomes {
  const o = asObject(v, path);
  return {
    active: required(o, "active", path, count),
    closed: required(o, "closed", path, count),
    unknown: required(o, "unknown", path, count),
    error: required(o, "error", path, count),
    canceled: required(o, "canceled", path, count),
  };
}

function decodeSummary(v: unknown, path: string): RunSummary {
  const o = asObject(v, path);
  const out: RunSummary = {
    runId: required(o, "runId", path, idStr),
    runType: required(o, "runType", path, runType),
    status: required(o, "status", path, runStatus),
    startedAt: required(o, "startedAt", path, str),
    finishedAt: required(o, "finishedAt", path, str),
    durationMs: required(o, "durationMs", path, count),
    postingOutcomes: required(o, "postingOutcomes", path, decodeOutcomes),
    stateChanges: required(o, "stateChanges", path, count),
  };
  setOpt(out, "stages", optional(o, "stages", path, arrayOf(decodeStage)));
  setOpt(out, "sourceRunId", optional(o, "sourceRunId", path, idStr));
  return out;
}

function decodeEvent(v: unknown, path: string): RunProgressEvent {
  const o = asObject(v, path);
  const out: RunProgressEvent = {
    version: version(o, path),
    runId: required(o, "runId", path, idStr),
    runType: required(o, "runType", path, runType),
    runStatus: required(o, "runStatus", path, runStatus),
    seq: required(o, "seq", path, count),
    emittedAt: required(o, "emittedAt", path, str),
    stage: required(o, "stage", path, legacyStage),
    message: required(o, "message", path, messageStr),
    current: required(o, "current", path, count),
    total: required(o, "total", path, count),
    done: required(o, "done", path, bool),
    startedAt: required(o, "startedAt", path, str),
    elapsedMs: required(o, "elapsedMs", path, count),
    postingCounts: required(o, "postingCounts", path, decodePostingCounts),
    postingTotal: required(o, "postingTotal", path, count),
  };
  currentWithinTotal(out.current, out.total, path);
  setOpt(out, "previousRunStatus", optional(o, "previousRunStatus", path, runStatus));
  setOpt(out, "stages", optional(o, "stages", path, arrayOf(decodeStage)));
  setOpt(out, "postings", optional(o, "postings", path, arrayOf(decodePosting)));
  setOpt(out, "errorReason", optional(o, "errorReason", path, reasonStr));
  setOpt(out, "summary", optional(o, "summary", path, decodeSummary));
  if (out.seq === 1 && out.postings === undefined) {
    throw new Invalid(`${path}.postings is required on the first event`);
  }
  validateRunConditionals(out.runStatus, out.done, out.summary, out.errorReason, path);
  return out;
}

function decodeSnapshot(v: unknown, path: string): RunSnapshot {
  const o = asObject(v, path);
  const out: RunSnapshot = {
    version: version(o, path),
    runId: required(o, "runId", path, idStr),
    runType: required(o, "runType", path, runType),
    runStatus: required(o, "runStatus", path, runStatus),
    seq: required(o, "seq", path, count),
    stage: required(o, "stage", path, legacyStage),
    message: required(o, "message", path, messageStr),
    current: required(o, "current", path, count),
    total: required(o, "total", path, count),
    done: required(o, "done", path, bool),
    startedAt: required(o, "startedAt", path, str),
    elapsedMs: required(o, "elapsedMs", path, count),
    postingCounts: required(o, "postingCounts", path, decodePostingCounts),
    postingTotal: required(o, "postingTotal", path, count),
    postings: required(o, "postings", path, arrayOf(decodePosting)),
    live: required(o, "live", path, bool),
    dismissed: required(o, "dismissed", path, bool),
  };
  currentWithinTotal(out.current, out.total, path);
  setOpt(out, "stages", optional(o, "stages", path, arrayOf(decodeStage)));
  setOpt(out, "errorReason", optional(o, "errorReason", path, reasonStr));
  setOpt(out, "summary", optional(o, "summary", path, decodeSummary));
  setOpt(out, "sourceRunId", optional(o, "sourceRunId", path, idStr));
  validateRunConditionals(out.runStatus, out.done, out.summary, out.errorReason, path);
  return out;
}

function decodeAccepted(v: unknown, path: string): RunAccepted {
  const o = asObject(v, path);
  return {
    runId: required(o, "runId", path, idStr),
    snapshot: required(o, "snapshot", path, decodeSnapshot),
  };
}

// ---- Public parsers ----

/** Validate one `jobs-runner-progress` payload against Progress_Contract v1. */
export function parseRunEvent(input: unknown): ParseResult<RunProgressEvent> {
  return run(input, (v) => decodeEvent(v, "event"));
}

/** Validate a `get_run_cmd` / `get_current_run_cmd` / `cancel_run_cmd` snapshot. */
export function parseRunSnapshot(input: unknown): ParseResult<RunSnapshot> {
  return run(input, (v) => decodeSnapshot(v, "snapshot"));
}

/** Validate a `start_run_cmd` / `retry_run_cmd` response. */
export function parseRunAccepted(input: unknown): ParseResult<RunAccepted> {
  return run(input, (v) => decodeAccepted(v, "accepted"));
}

/** Validate a standalone Run_Summary. */
export function parseRunSummary(input: unknown): ParseResult<RunSummary> {
  return run(input, (v) => decodeSummary(v, "summary"));
}
