// The pinned reference implementation is used only by Node comparison tests.
export * from "solid-js";
export { flush as batch } from "solid-js";
export function catchError<T>(run: () => T, handler: (error: unknown) => void): T | undefined {
  try { return run(); } catch (error) { handler(error); }
}
