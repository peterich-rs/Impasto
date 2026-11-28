export type LocationAccuracy =
  | "lowest"
  | "low"
  | "medium"
  | "high"
  | "best"
  | "bestForNavigation";

export type LocationServiceStatus = "enabled" | "disabled";

export type GeolocatorErrorCode =
  | "permission-denied"
  | "permission-permanently-denied"
  | "service-disabled"
  | "timeout"
  | "cancelled"
  | "position-unavailable"
  | "host-unavailable"
  | "platform";

export interface Position {
  readonly latitude: number;
  readonly longitude: number;
  /** 水平定位精度半径（米） */
  readonly accuracy: number;
  readonly altitude: number | null;
  readonly altitudeAccuracy: number | null;
  /** 航向角（0..360 度，正北为 0），不可用时 null */
  readonly heading: number | null;
  readonly headingAccuracy: number | null;
  /** 速度（米/秒），不可用时 null */
  readonly speed: number | null;
  readonly speedAccuracy: number | null;
  readonly timestampMs: number;
  readonly isMocked: boolean;
}

export interface Coordinates {
  readonly latitude: number;
  readonly longitude: number;
}

export interface CurrentPositionOptions {
  readonly accuracy?: LocationAccuracy; // 默认 "high"
  readonly timeoutMs?: number; // 默认 15_000
  /** 若存在比当前早不超过 maximumAgeMs 的缓存定位，允许直接复用 */
  readonly maximumAgeMs?: number;
  /** 是否在未授权时自动通过 @impasto/permissions 请求 LocationWhenInUse，默认 true */
  readonly ensurePermission?: boolean;
  readonly signal?: AbortSignal;
}

export interface WatchPositionOptions {
  readonly accuracy?: LocationAccuracy; // 默认 "high"
  /** 最小位置变化触发距离（米），默认 10 */
  readonly distanceFilterMeters?: number;
  /** Android 优先采样最小间隔（毫秒），默认 2_000 */
  readonly intervalMs?: number;
  /** 允许后台持续定位（需在 app.toml 开启 background feature 并获 LocationAlways 授权），默认 false */
  readonly allowBackground?: boolean;
  readonly ensurePermission?: boolean;
  readonly signal?: AbortSignal;
}
