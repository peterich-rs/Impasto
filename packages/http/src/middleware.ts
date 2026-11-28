import type { DirectoryScope } from "@impasto/file-system";
import type { Headers } from "./headers.ts";
import type { HttpError, HttpRequestOptions } from "./client.ts";

export interface HttpProgressEvent { readonly loaded: number; readonly total: number | null; }
export interface CookieJarConfig { readonly mode: "memory" | "persistent"; readonly name?: string; }
export interface HttpProxyConfig { readonly url: string; readonly bypass?: readonly string[]; }
export interface HttpSecurityConfig { readonly certificatePins?: Record<string, readonly string[]>; }
export interface RetryPolicy {
  readonly maxRetries: number;
  readonly baseDelayMs?: number;
  readonly maxDelayMs?: number;
  readonly backoffFactor?: number;
  readonly jitter?: boolean;
  readonly retryMethods?: readonly string[];
  readonly retryStatuses?: readonly number[];
  readonly respectRetryAfter?: boolean;
  readonly shouldRetry?: (error: HttpError, attempt: number) => boolean | Promise<boolean>;
}
export interface DownloadToFileOptions extends HttpRequestOptions {
  readonly destinationPath: string; readonly scope?: DirectoryScope;
  readonly resume?: boolean; readonly overwrite?: boolean;
}
export interface DownloadResult {
  readonly status: number; readonly url: string; readonly redirected: boolean; readonly headers: Headers;
  readonly bytesWritten: number; readonly resumed: boolean; readonly path: string;
}
export interface FileUploadBody { readonly path: string; readonly scope?: DirectoryScope; readonly contentType?: string; }
export interface CookieEntry {
  readonly name: string; readonly value: string; readonly domain: string; readonly path: string;
  readonly expiresAtMs: number | null; readonly httpOnly: boolean; readonly secure: boolean;
}

export interface InterceptorManager<T, E = T> {
  use(onFulfilled: (value: T) => T | Promise<T>, onRejected?: (error: E) => T | Promise<T>): () => void;
}

export class Interceptors<T, E> implements InterceptorManager<T, E> {
  readonly #items = new Set<{ fulfilled: (value: T) => T | Promise<T>; rejected?: (error: E) => T | Promise<T> }>();
  use(fulfilled: (value: T) => T | Promise<T>, rejected?: (error: E) => T | Promise<T>): () => void {
    const interceptor = { fulfilled, rejected };
    this.#items.add(interceptor);
    return () => { this.#items.delete(interceptor); };
  }
  run(value: T): Promise<T> {
    let next: Promise<T> = Promise.resolve(value);
    for (const item of this.#items) next = next.then(item.fulfilled, item.rejected);
    return next;
  }
  recover(error: E): Promise<T> {
    let next = Promise.reject<T>(error);
    for (const item of this.#items) next = next.then(item.fulfilled, item.rejected);
    return next;
  }
}

export function abortableWait(milliseconds: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const cleanup = () => { clearTimeout(timer); signal?.removeEventListener("abort", abort); };
    const abort = () => { cleanup(); reject(signal?.reason ?? new DOMException("The operation was aborted.", "AbortError")); };
    const timer = setTimeout(() => { cleanup(); resolve(); }, milliseconds);
    if (signal?.aborted) abort();
    else signal?.addEventListener("abort", abort, { once: true });
  });
}

export function retryDelay(policy: RetryPolicy, retry: number, retryAfter?: string | null): number {
  const max = Math.max(0, policy.maxDelayMs ?? 10_000);
  if (retryAfter && policy.respectRetryAfter !== false) {
    const after = /^\d+$/.test(retryAfter) ? Number(retryAfter) * 1000 : Date.parse(retryAfter) - Date.now();
    if (Number.isFinite(after)) return Math.min(max, Math.max(0, after));
  }
  const delay = Math.max(0, Math.min(max, (policy.baseDelayMs ?? 250) * (policy.backoffFactor ?? 2) ** retry));
  return policy.jitter === false ? delay : delay * Math.random();
}
