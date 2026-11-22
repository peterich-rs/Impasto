import * as native from "./native.js";
import {
  createWebSocketClass,
  WebSocketCloseEvent,
  WebSocketMessageEvent,
} from "./compat-impl.ts";

export const WebSocket = createWebSocketClass(native);
export { WebSocketCloseEvent, WebSocketMessageEvent };
export default WebSocket;
