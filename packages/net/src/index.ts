export type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

/**
 * Each `run` aborts the previous request. The signal passed to `fetch` is
 * the new controller, so an older response cannot win.
 */
export function createFetch(fetchImpl: FetchLike = fetch): {
  run: (input: string, init?: RequestInit) => Promise<Response>;
  abort: () => void;
} {
  let current: AbortController | null = null;
  return {
    run(input, init) {
      current?.abort();
      const controller = new AbortController();
      current = controller;
      return fetchImpl(input, { ...init, signal: controller.signal });
    },
    abort() {
      current?.abort();
      current = null;
    },
  };
}

/** Alias used by call sites that want a hook-shaped name. */
export function useFetch(fetchImpl?: FetchLike): ReturnType<typeof createFetch> {
  return createFetch(fetchImpl);
}
