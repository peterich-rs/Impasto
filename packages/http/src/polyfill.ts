import { fetch, FormData, Headers, Request, Response } from "./fetch.ts";

export function installFetchPolyfill(
  target: Record<string, unknown> = globalThis as Record<string, unknown>,
): void {
  target.fetch = fetch;
  target.Request = Request;
  target.Response = Response;
  target.Headers = Headers;
  target.FormData = FormData;
}

installFetchPolyfill();

export { fetch, FormData, Headers, Request, Response };
