import { WebSocket } from "./compat.ts";

export function installWebSocketPolyfill(
  target: Record<string, unknown> = globalThis as Record<string, unknown>,
): void {
  target.WebSocket = WebSocket;
}

installWebSocketPolyfill();

export { WebSocket };
