/**
 * RunMonitorProvider: the single app-level owner of Run state (design.md,
 * "Component 11: Frontend"). It wires the pure `reduceRun` reducer to the three
 * data sources — the `jobs-runner-progress` event stream, the reconciler, and
 * the discovery/external-run pollers — and exposes actions plus a settle
 * subscription to the rest of the UI.
 *
 * Everything time-dependent (notice expiry, announcement throttling, backoff)
 * flows through `Date.now()` at the edges; the reducer itself stays pure.
 *
 * Degrades gracefully outside the desktop shell (`isDesktopShell() === false`):
 * no listener, no polling, and the actions reject with the shell-required error
 * from the api layer rather than throwing here.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactElement,
  type ReactNode,
} from "react";

import { api } from "@/lib/api";
import { RunContractError, type RunSnapshot, type RunType } from "@/lib/run-contract";
import { isRunInProgressError, parseAppError } from "@/lib/run-errors";
import {
  buildAnnouncements,
  canRetry,
  initialAnnouncementThrottle,
  initialRunViewState,
  isTerminalRunStatus,
  reduceRun,
  retrySelectionIds,
  type AnnouncementThrottle,
  type RunAction,
  type RunFilter,
  type RunViewState,
} from "@/lib/run-state";
import { isDesktopShell } from "@/lib/tauri";

/** Poll cadence for a run owned by another process (design.md external polling). */
const EXTERNAL_POLL_MS = 1_000;
/** Poll cadence to discover launchd/CLI runs when nothing is displayed. */
const DISCOVERY_POLL_MS = 5_000;
/** Reconcile debounce; backoff grows from here on repeated failure. */
const RECONCILE_MIN_BACKOFF_MS = 250;
const RECONCILE_MAX_BACKOFF_MS = 4_000;
/** How often the notice-expiry timer checks for an expired notice. */
const NOTICE_SWEEP_MS = 1_000;

/** The `{polite, assertive}` texts the live regions render (Req 12.3, 12.4). */
export type LiveAnnouncements = {
  polite?: string;
  assertive: string[];
};

/** Callback fired once per run when the displayed run first becomes terminal. */
export type RunSettledCallback = (snapshot: RunSnapshot) => void;

export type RunMonitorContextValue = {
  /** The full reducer state (displayed run, notice, filter, retry selection, ...). */
  state: RunViewState;
  /** Latest live-region texts. `assertive` entries are already de-duplicated. */
  announcements: LiveAnnouncements;

  /** Start a Jobs_Cycle or Posting_Check_Run. In-progress maps to a notice. */
  start: (runType: RunType) => Promise<void>;
  /** Request cancellation of the displayed run. */
  cancel: () => Promise<void>;
  /** Retry the current `retrySelection` against the displayed (terminal) run. */
  retry: () => Promise<void>;
  /** Dismiss the displayed terminal run's summary. */
  dismiss: () => Promise<void>;

  setFilter: (filter: RunFilter) => void;
  toggleRetry: (jobId: string) => void;
  selectAllNeedingAttention: () => void;
  clearRetrySelection: () => void;

  /**
   * Subscribe to run-settled events. The callback fires exactly once per runId
   * the first time that run becomes terminal (Completed, Completed_With_Errors,
   * Canceled, or Error). Error is included so pages still refresh after a failed
   * run. Returns an unsubscribe function (Req 9.5, 9.6).
   */
  onRunSettled: (cb: RunSettledCallback) => () => void;

  /** Report that a page refresh after settle failed, so the panel can show it (Req 9.9). */
  reportRefreshFailed: (message?: string) => void;
};

const RunMonitorContext = createContext<RunMonitorContextValue | null>(null);

export function useRunMonitor(): RunMonitorContextValue {
  const ctx = useContext(RunMonitorContext);
  if (ctx === null) {
    throw new Error("useRunMonitor must be used within a RunMonitorProvider");
  }
  return ctx;
}

export function RunMonitorProvider({ children }: { children: ReactNode }): ReactElement {
  const [state, dispatch] = useReducer(reduceRun, initialRunViewState);

  // Announcement throttle bookkeeping lives in a ref: it is derived output, not state.
  const throttleRef = useRef<AnnouncementThrottle>(initialAnnouncementThrottle);
  const [announcements, setAnnouncements] = useState<LiveAnnouncements>({ assertive: [] });

  // `onRunSettled` subscribers and the set of runIds already settled.
  const settledCallbacksRef = useRef<Set<RunSettledCallback>>(new Set());
  const settledRunsRef = useRef<Set<string>>(new Set());

  // Keep a live view of state for callbacks (actions, pollers) without re-subscribing.
  const stateRef = useRef(state);
  stateRef.current = state;

  // Reconcile backoff, reset on every success.
  const reconcileBackoffRef = useRef(RECONCILE_MIN_BACKOFF_MS);

  // The reducer state drives rendering; this ref closes the gap between two
  // rapid clicks before React commits the pending state update.
  const startPendingRef = useRef(false);

  const desktop = isDesktopShell();

  // ---- Announcements: recompute on every state transition ----
  const prevStateRef = useRef<RunViewState>(initialRunViewState);
  useEffect(() => {
    const prev = prevStateRef.current;
    prevStateRef.current = state;
    if (prev === state) return;

    const result = buildAnnouncements(prev, state, throttleRef.current, Date.now());
    throttleRef.current = result.throttle;
    if (result.announcedKeys.length > 0) {
      dispatch({ type: "announced", keys: result.announcedKeys });
    }
    if (result.polite !== undefined || result.assertive.length > 0) {
      setAnnouncements((current) => ({
        polite: result.polite ?? current.polite,
        assertive: result.assertive,
      }));
    }
  }, [state]);

  // ---- onRunSettled: fire once per run when it first becomes terminal ----
  useEffect(() => {
    const shown = state.displayed;
    if (shown === null || !isTerminalRunStatus(shown.runStatus)) return;
    if (settledRunsRef.current.has(shown.runId)) return;
    settledRunsRef.current.add(shown.runId);
    for (const cb of settledCallbacksRef.current) {
      try {
        cb(shown);
      } catch (err) {
        console.error("onRunSettled callback failed", err);
      }
    }
  }, [state.displayed]);

  // ---- Reconciler: load a snapshot, translating contract errors to notices ----
  const reconcile = useCallback(
    async (runId: string | null) => {
      if (!desktop) return;
      try {
        const snapshot =
          runId === null ? await api.getCurrentRun() : await api.getRun(runId);
        dispatch({ type: "snapshotLoaded", snapshot });
        reconcileBackoffRef.current = RECONCILE_MIN_BACKOFF_MS;
      } catch (err) {
        if (err instanceof RunContractError && err.kind === "unsupported_version") {
          dispatch({ type: "unsupportedVersion", message: err.message, now: Date.now() });
        } else if (err instanceof RunContractError) {
          dispatch({ type: "invalidEvent", message: err.message, now: Date.now() });
        } else {
          dispatch({ type: "snapshotFailed", now: Date.now() });
        }
        // Grow backoff for the next needsReconcile-driven attempt.
        reconcileBackoffRef.current = Math.min(
          RECONCILE_MAX_BACKOFF_MS,
          reconcileBackoffRef.current * 2,
        );
      }
    },
    [desktop],
  );

  // ---- Event listener: one subscription, validated payloads ----
  useEffect(() => {
    if (!desktop) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    api
      .listenRunProgress((result) => {
        if (result.ok) {
          dispatch({ type: "event", event: result.value });
        } else if (result.kind === "unsupported_version") {
          dispatch({ type: "unsupportedVersion", message: result.message, now: Date.now() });
        } else {
          dispatch({ type: "invalidEvent", message: result.message, now: Date.now() });
        }
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err) => {
        console.error("Failed to subscribe to run progress", err);
      });

    return () => {
      cancelled = true;
      if (unlisten !== null) unlisten();
    };
  }, [desktop]);

  // ---- Restore on mount (Req 2.5) ----
  useEffect(() => {
    if (!desktop) return;
    void reconcile(null);
  }, [desktop, reconcile]);

  // ---- Reconcile whenever a gap is detected (Req 2.7, 11.11), debounced with backoff ----
  useEffect(() => {
    if (!desktop || !state.needsReconcile) return;
    const runId = state.displayed?.runId ?? null;
    const delay = reconcileBackoffRef.current;
    const timer = setTimeout(() => {
      void reconcile(runId);
    }, delay);
    return () => clearTimeout(timer);
  }, [desktop, state.needsReconcile, state.displayed?.runId, reconcile]);

  // ---- Polling: external non-terminal run (1s) or discovery (5s) ----
  useEffect(() => {
    if (!desktop) return;
    const shown = state.displayed;
    const externalRunning =
      shown !== null && shown.live === false && !isTerminalRunStatus(shown.runStatus);

    if (externalRunning) {
      const timer = setInterval(() => void reconcile(shown.runId), EXTERNAL_POLL_MS);
      return () => clearInterval(timer);
    }
    if (shown === null) {
      const timer = setInterval(() => void reconcile(null), DISCOVERY_POLL_MS);
      return () => clearInterval(timer);
    }
    return undefined;
  }, [desktop, state.displayed, reconcile]);

  // ---- Notice expiry sweep ----
  useEffect(() => {
    if (state.notice === undefined) return;
    const timer = setInterval(() => {
      dispatch({ type: "clearNotice", now: Date.now() });
    }, NOTICE_SWEEP_MS);
    return () => clearInterval(timer);
  }, [state.notice]);

  // ---- Actions ----
  const runAction = useCallback(async (fn: () => Promise<RunAction | null>) => {
    try {
      const action = await fn();
      if (action !== null) dispatch(action);
    } catch (err) {
      if (isRunInProgressError(err)) {
        dispatch({ type: "rejectedInProgress", now: Date.now() });
        void reconcile(null);
      } else {
        const parts = parseAppError(err);
        dispatch({
          type: "actionFailed",
          message: "Run action failed: " + (parts.message || "unknown error"),
          now: Date.now(),
        });
      }
    }
  }, [reconcile]);

  const start = useCallback(
    (runType: RunType) => {
      if (startPendingRef.current) return Promise.resolve();
      startPendingRef.current = true;
      dispatch({ type: "startPending" });
      return runAction(async () => {
        const accepted = await api.startRun(runType);
        return { type: "accepted", accepted };
      }).finally(() => {
        startPendingRef.current = false;
        dispatch({ type: "startSettled" });
      });
    },
    [runAction],
  );

  const retry = useCallback(
    () =>
      runAction(async () => {
        const current = stateRef.current;
        if (!canRetry(current) || current.displayed === null) return null;
        const jobIds = retrySelectionIds(current);
        if (jobIds.length === 0) return null;
        const accepted = await api.retryRun({
          sourceRunId: current.displayed.runId,
          jobIds,
        });
        return { type: "accepted", accepted };
      }),
    [runAction],
  );

  const cancel = useCallback(
    () =>
      runAction(async () => {
        const shown = stateRef.current.displayed;
        if (shown === null) return null;
        const snapshot = await api.cancelRun(shown.runId);
        return { type: "snapshotLoaded", snapshot };
      }),
    [runAction],
  );

  const dismiss = useCallback(
    () =>
      runAction(async () => {
        const shown = stateRef.current.displayed;
        if (shown === null || !isTerminalRunStatus(shown.runStatus)) return null;
        const runId = shown.runId;
        await api.dismissRun(runId);
        return { type: "dismissed", runId };
      }),
    [runAction],
  );

  const setFilter = useCallback((filter: RunFilter) => dispatch({ type: "setFilter", filter }), []);
  const toggleRetry = useCallback((jobId: string) => dispatch({ type: "toggleRetry", jobId }), []);
  const selectAllNeedingAttention = useCallback(
    () => dispatch({ type: "selectAllNeedingAttention" }),
    [],
  );
  const clearRetrySelection = useCallback(() => dispatch({ type: "clearRetrySelection" }), []);

  const reportRefreshFailed = useCallback(
    (message?: string) => dispatch({ type: "refreshFailed", message, now: Date.now() }),
    [],
  );

  const onRunSettled = useCallback((cb: RunSettledCallback) => {
    settledCallbacksRef.current.add(cb);
    return () => {
      settledCallbacksRef.current.delete(cb);
    };
  }, []);

  const value = useMemo<RunMonitorContextValue>(
    () => ({
      state,
      announcements,
      start,
      cancel,
      retry,
      dismiss,
      setFilter,
      toggleRetry,
      selectAllNeedingAttention,
      clearRetrySelection,
      onRunSettled,
      reportRefreshFailed,
    }),
    [
      state,
      announcements,
      start,
      cancel,
      retry,
      dismiss,
      setFilter,
      toggleRetry,
      selectAllNeedingAttention,
      clearRetrySelection,
      onRunSettled,
      reportRefreshFailed,
    ],
  );

  return <RunMonitorContext.Provider value={value}>{children}</RunMonitorContext.Provider>;
}
