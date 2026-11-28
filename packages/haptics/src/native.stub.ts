import type { ImpactStyle, NotificationFeedbackType } from "./types";
export function isHapticsHostAvailable() { return false; }
export async function isSupported(): Promise<boolean> { return false; }
export async function impact(_style?: ImpactStyle): Promise<void> {}
export async function selection(): Promise<void> {}
export async function notification(_kind: NotificationFeedbackType): Promise<void> {}
export async function vibrate(_durationMs?: number): Promise<void> {}
export async function vibratePattern(_timingsMs: readonly number[], _amplitudes?: readonly number[]): Promise<void> {}
export async function cancel(): Promise<void> {}
