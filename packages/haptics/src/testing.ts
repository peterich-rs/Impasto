import { installMock } from "./driver";
import type { ImpactStyle, NotificationFeedbackType } from "./types";
type Event = { operation: "impact"; style: ImpactStyle } | { operation: "selection" | "cancel" } |
  { operation: "notification"; type: NotificationFeedbackType } | { operation: "vibrate"; durationMs: number } |
  { operation: "vibratePattern"; timingsMs: readonly number[]; amplitudes?: readonly number[] };
export function mockHaptics({ supported = true } = {}) {
  const events: Event[] = [];
  const restore = installMock({
    async isSupported() { return supported; },
    async impact(style) { events.push({ operation: "impact", style: style ?? "medium" }); },
    async selection() { events.push({ operation: "selection" }); },
    async notification(type) { events.push({ operation: "notification", type }); },
    async vibrate(durationMs) { events.push({ operation: "vibrate", durationMs: durationMs ?? 400 }); },
    async vibratePattern(timingsMs, amplitudes) { events.push({ operation: "vibratePattern", timingsMs, amplitudes: amplitudes ?? undefined }); },
    async cancel() { events.push({ operation: "cancel" }); },
  });
  return { events: events as readonly Event[], restore };
}
