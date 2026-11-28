import { driver, native } from "./driver";
import type { Coordinates, CurrentPositionOptions, LocationServiceStatus, Position, WatchPositionOptions } from "./types";
export type * from "./types";

export async function getCurrentPosition({ signal, ...options }: CurrentPositionOptions = {}): Promise<Position> {
  if (signal?.aborted) throw signal.reason;
  const platform = driver();
  const id = platform.nextRequestId();
  const result = platform.getCurrentPosition(id, options);
  if (!signal) return result;
  let abort = () => {};
  try {
    return await Promise.race([result, new Promise<Position>((_, reject) => {
      abort = () => { void platform.cancel(id).catch(() => undefined); reject(signal.reason); };
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) abort();
    })]);
  } finally { signal.removeEventListener("abort", abort); }
}
export async function getLastKnownPosition(): Promise<Position | null> { return await driver().getLastKnownPosition() ?? null; }
export const watchPosition = ({ signal, ...options }: WatchPositionOptions = {}): ReadableStream<Position> =>
  driver().watchPosition(options, signal);
export const watchServiceStatus = (signal?: AbortSignal): ReadableStream<LocationServiceStatus> => driver().watchServiceStatus(signal);
export const isLocationServiceEnabled = () => driver().isLocationServiceEnabled();
export const checkPermission = () => driver().checkPermission();
export const requestPermission = (scope?: "whenInUse" | "always") => driver().requestPermission(scope);
export const distanceBetween = (a: Coordinates, b: Coordinates) => native.distanceBetween(a, b);
export const bearingBetween = (a: Coordinates, b: Coordinates) => native.bearingBetween(a, b);
export const destinationPoint = (origin: Coordinates, distance: number, bearing: number): Coordinates => native.destinationPoint(origin, distance, bearing);

export function onPositionChange(listener: (position: Position) => void, options?: WatchPositionOptions): () => void {
  const reader = watchPosition(options).getReader();
  void (async () => {
    try {
      for (;;) {
        const step = await reader.read();
        if (step.done) return;
        listener(step.value);
      }
    } catch (error) {
      if (!options?.signal?.aborted) console.error(error);
    } finally { reader.releaseLock(); }
  })();
  return () => { void reader.cancel().catch(() => undefined); };
}
