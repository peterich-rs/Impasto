export type ConnectivityType = "wifi" | "cellular" | "ethernet" | "vpn" | "bluetooth" | "other" | "none";
export type ConnectivityErrorCode = "host-unavailable" | "platform";
export interface ConnectivityState {
  readonly types: readonly ConnectivityType[];
  readonly isConnected: boolean;
  readonly isExpensive: boolean;
  readonly isConstrained: boolean;
}
