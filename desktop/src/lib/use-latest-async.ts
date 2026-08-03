import { useCallback, useEffect, useRef, useState } from "react";

export type LatestAsyncState<T> = {
  data: T | undefined;
  error: string | null;
  initialLoading: boolean;
  refreshing: boolean;
};

/**
 * Monotonic request coordination for page loads.
 * Only the latest request for the current key may commit state.
 * Tauri invoke cannot be aborted, so the sequence guard is mandatory.
 */
export function useLatestAsync<T>(
  key: string,
  loader: () => Promise<T>,
  options?: { errorMessage?: string },
): LatestAsyncState<T> & { reload: (opts?: { quiet?: boolean }) => Promise<void> } {
  const [data, setData] = useState<T | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [initialLoading, setInitialLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const sequenceRef = useRef(0);
  const keyRef = useRef(key);
  const dataRef = useRef(data);
  const loaderRef = useRef(loader);
  const errorMessage = options?.errorMessage ?? "Failed to load";

  loaderRef.current = loader;
  keyRef.current = key;
  dataRef.current = data;

  const reload = useCallback(async (opts?: { quiet?: boolean }) => {
    const requestKey = keyRef.current;
    const sequence = ++sequenceRef.current;
    const quiet = Boolean(opts?.quiet) && dataRef.current !== undefined;

    if (quiet) {
      setRefreshing(true);
    } else {
      setInitialLoading(true);
    }
    setError(null);

    try {
      const result = await loaderRef.current();
      if (sequence !== sequenceRef.current || requestKey !== keyRef.current) {
        return;
      }
      setData(result);
    } catch (err) {
      if (sequence !== sequenceRef.current || requestKey !== keyRef.current) {
        return;
      }
      setError(err instanceof Error ? err.message : errorMessage);
    } finally {
      if (sequence === sequenceRef.current) {
        setInitialLoading(false);
        setRefreshing(false);
      }
    }
  }, [errorMessage]);

  useEffect(() => {
    void reload();
  }, [key, reload]);

  return { data, error, initialLoading, refreshing, reload };
}
