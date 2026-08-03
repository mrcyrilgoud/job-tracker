import { useCallback, useState } from "react";

export type Feedback = { tone: "positive" | "negative"; text: string };

/**
 * Tracks in-flight actions by key so a button can show its own progress without
 * disabling unrelated controls. A single shared `busy` boolean meant syncing one
 * company greyed out every other company's buttons.
 *
 * Feedback is keyed the same way, so the result of an action can be rendered
 * next to the control that started it rather than in a page-level banner.
 */
export function usePendingActions() {
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  const [feedback, setFeedback] = useState<Readonly<Record<string, Feedback>>>({});

  const isPending = useCallback((key: string) => pending.has(key), [pending]);

  const setKeyFeedback = useCallback((key: string, value: Feedback | null) => {
    setFeedback((current) => {
      const next = { ...current };
      if (value) {
        next[key] = value;
      } else {
        delete next[key];
      }
      return next;
    });
  }, []);

  /**
   * Runs `action` under `key`. `describe` turns the result into the message
   * shown beside the control; returning null clears it. Errors surface in the
   * same place, so a failure is never silent.
   */
  const run = useCallback(
    async <T>(
      key: string,
      action: () => Promise<T>,
      describe?: (result: T) => Feedback | null,
      fallbackError = "That didn't work. Try again.",
    ): Promise<T | undefined> => {
      setPending((current) => new Set(current).add(key));
      setKeyFeedback(key, null);
      try {
        const result = await action();
        setKeyFeedback(key, describe ? describe(result) : null);
        return result;
      } catch (err) {
        setKeyFeedback(key, {
          tone: "negative",
          text: err instanceof Error ? err.message : fallbackError,
        });
        return undefined;
      } finally {
        setPending((current) => {
          const next = new Set(current);
          next.delete(key);
          return next;
        });
      }
    },
    [setKeyFeedback],
  );

  return { isPending, run, feedback, setFeedback: setKeyFeedback };
}
