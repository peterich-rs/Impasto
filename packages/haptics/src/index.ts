import { driver } from "./driver";
import type { ImpactStyle, NotificationFeedbackType } from "./types";
export type * from "./types";
export const isSupported = () => driver().isSupported();
export const impact = (style?: ImpactStyle) => driver().impact(style);
export const selection = () => driver().selection();
export const notification = (type: NotificationFeedbackType) => driver().notification(type);
export const vibrate = (durationMs?: number) => driver().vibrate(durationMs);
export const vibratePattern = (timingsMs: readonly number[], amplitudes?: readonly number[]) =>
  driver().vibratePattern([...timingsMs], amplitudes?.slice());
export const cancel = () => driver().cancel();
