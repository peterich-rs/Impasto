import type { ShareOptions, ShareResult } from "./types";
export function isShareHostAvailable() { return false; }
export async function share(_options: ShareOptions): Promise<ShareResult> {
  throw Object.assign(new Error("Sharing requires a native application host"), { code: "host-unavailable" });
}
