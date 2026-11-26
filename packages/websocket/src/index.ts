import * as native from "@impasto/websocket/native";
import {
  connectWith,
  READY_STATE_CLOSED,
  READY_STATE_CLOSING,
  READY_STATE_CONNECTING,
  READY_STATE_OPEN,
  WebSocketClient,
  type ConnectOptions,
  type WebSocketCloseInfo,
  type WebSocketMessage,
  type WebSocketTransport,
} from "./client.ts";

export {
  READY_STATE_CLOSED,
  READY_STATE_CLOSING,
  READY_STATE_CONNECTING,
  READY_STATE_OPEN,
  WebSocketClient,
};
export type {
  ConnectOptions,
  WebSocketCloseInfo,
  WebSocketMessage,
  WebSocketTransport,
};

export function connect(
  url: string,
  options?: ConnectOptions,
): Promise<WebSocketClient> {
  return connectWith(native, url, options);
}
