import type { PermissionStatus } from "@impasto/permissions";
export const isNotificationsHostAvailable = () => false;
function unavailable(): never { throw new Error("[notifications:host-unavailable] Notifications require a native host"); }
export async function show(_request: string): Promise<void> { return unavailable(); }
export async function schedule(_request: string, _trigger: string): Promise<void> { return unavailable(); }
export async function invoke(_operation: string, _payload: string): Promise<string> { return unavailable(); }
export async function checkPermission(): Promise<PermissionStatus> { return "denied"; }
export async function requestPermission(_options: string): Promise<PermissionStatus> { return "denied"; }
export function watchResponses(_signal?: AbortSignal): ReadableStream<string> { return unavailable(); }
export function watchForeground(_signal?: AbortSignal): ReadableStream<string> { return unavailable(); }
