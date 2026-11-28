import { driver } from "./driver";
import type { ForegroundPresentationOptions, NotificationCategory, NotificationChannel, NotificationRequest,
  NotificationResponse, NotificationTrigger, PendingNotification } from "./types";
export type * from "./types";

async function invoke<T>(operation: string, value: unknown = null): Promise<T> {
  return JSON.parse(await driver().invoke(operation, JSON.stringify(value))) as T;
}
function decode<T>(stream: ReadableStream<string>): ReadableStream<T> {
  const reader = stream.getReader();
  return new ReadableStream<T>({
    async pull(controller) {
      try {
        const step = await reader.read();
        if (step.done) { reader.releaseLock(); controller.close(); }
        else controller.enqueue(JSON.parse(step.value) as T);
      } catch (error) { reader.releaseLock(); controller.error(error); }
    },
    async cancel(reason) { try { await reader.cancel(reason); } finally { reader.releaseLock(); } },
  });
}

export const show = (request: NotificationRequest) => driver().show(JSON.stringify(request));
export const schedule = (request: NotificationRequest, trigger: NotificationTrigger) =>
  driver().schedule(JSON.stringify(request), JSON.stringify(trigger));
export const cancel = (id: string) => invoke<void>("cancel", id);
export const cancelAll = () => invoke<void>("cancelAll");
export const getPending = () => invoke<PendingNotification[]>("getPending");
export const dismissDelivered = (id?: string) => invoke<void>("dismissDelivered", id);
export const createChannel = (channel: NotificationChannel) => invoke<void>("createChannel", channel);
export const deleteChannel = (id: string) => invoke<void>("deleteChannel", id);
export const listChannels = () => invoke<NotificationChannel[]>("listChannels");
export const registerCategories = (categories: readonly NotificationCategory[]) => invoke<void>("registerCategories", categories);
export const setBadgeCount = (count: number) => invoke<void>("setBadgeCount", count);
export const getBadgeCount = () => invoke<number>("getBadgeCount");
export const setForegroundPresentation = (options: ForegroundPresentationOptions) => invoke<void>("setForegroundPresentation", options);
export const getInitialResponse = () => invoke<NotificationResponse | null>("getInitialResponse");
export const watchResponses = (signal?: AbortSignal) => decode<NotificationResponse>(driver().watchResponses(signal));
export const watchForeground = (signal?: AbortSignal) => decode<NotificationRequest>(driver().watchForeground(signal));
export function onResponse(listener: (response: NotificationResponse) => void): () => void {
  const reader = watchResponses().getReader();
  void (async () => {
    try {
      for (;;) {
        const item = await reader.read();
        if (item.done) break;
        listener(item.value);
      }
    } catch (error) { console.error(error); }
    finally { reader.releaseLock(); }
  })();
  return () => { void reader.cancel().catch(() => undefined); };
}
export const checkPermission = () => driver().checkPermission();
export const requestPermission = (options: { alert?: boolean; badge?: boolean; sound?: boolean; provisional?: boolean; criticalAlert?: boolean } = {}) =>
  driver().requestPermission(JSON.stringify(options));
