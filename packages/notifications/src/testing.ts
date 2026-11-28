import type { PermissionStatus } from "@impasto/permissions";
import { installMock } from "./driver";
import type { NotificationCategory, NotificationChannel, NotificationRequest, NotificationResponse, NotificationTrigger } from "./types";

function events() {
  const subscribers = new Map<ReadableStreamDefaultController<string>, () => void>();
  return {
    watch(signal?: AbortSignal) {
      let detach = () => {};
      return new ReadableStream<string>({
        start(controller) {
          if (signal?.aborted) { controller.error(signal.reason); return; }
          const abort = () => { detach(); controller.error(signal?.reason); };
          detach = () => { subscribers.delete(controller); signal?.removeEventListener("abort", abort); };
          signal?.addEventListener("abort", abort, { once: true });
          subscribers.set(controller, detach);
        },
        cancel() { detach(); },
      });
    },
    emit(item: unknown) { for (const controller of subscribers.keys()) controller.enqueue(JSON.stringify(item)); },
    close() { for (const [controller, detach] of subscribers) { detach(); controller.close(); } },
  };
}

export function mockNotifications(options: { initialResponse?: NotificationResponse; permission?: PermissionStatus } = {}) {
  let initial = options.initialResponse;
  let badge = 0;
  let categories: NotificationCategory[] = [];
  let permission = options.permission ?? "granted";
  const responses = events();
  const foreground = events();
  const channels = new Map<string, NotificationChannel>();
  const pending = new Map<string, { request: NotificationRequest; trigger: NotificationTrigger }>();
  const history: NotificationRequest[] = [];
  const delivered = new Map<string, NotificationRequest>();
  const fire = (request: NotificationRequest) => { history.push(request); delivered.set(request.id, request); };
  const requirePermission = () => {
    if (!["granted", "limited", "provisional"].includes(permission)) throw new Error("[notifications:permission-denied] Notification authorization is required");
  };
  const restore = installMock({
    async show(payload) { requirePermission(); fire(JSON.parse(payload)); },
    async schedule(payload, trigger) {
      requirePermission();
      const request: NotificationRequest = JSON.parse(payload);
      pending.set(request.id, { request, trigger: JSON.parse(trigger) });
    },
    async invoke(op, payload) {
      const input = JSON.parse(payload);
      let result: unknown = null;
      switch (op) {
        case "cancel": pending.delete(input); break;
        case "cancelAll": pending.clear(); break;
        case "dismissDelivered": if (input === null) delivered.clear(); else delivered.delete(input); break;
        case "getPending": result = [...pending.values()].map(({ request, trigger }) => ({
          id: request.id, title: request.title, body: request.body, trigger, data: request.data ?? {},
        })); break;
        case "createChannel": channels.set(input.id, input); break;
        case "deleteChannel": channels.delete(input); break;
        case "listChannels": result = [...channels.values()]; break;
        case "registerCategories": categories = input; break;
        case "getBadgeCount": result = badge; break;
        case "setBadgeCount": badge = input; break;
        case "getInitialResponse": result = initial ?? null; initial = undefined; break;
      }
      return JSON.stringify(result);
    },
    checkPermission: async () => permission,
    requestPermission: async () => permission,
    watchResponses: responses.watch,
    watchForeground: foreground.watch,
  });
  return {
    history, channels, pending, delivered,
    get categories() { return categories; },
    setPermission(next: PermissionStatus) { permission = next; },
    fireScheduled(id: string) {
      const item = pending.get(id);
      if (!item) return;
      fire(item.request);
      if (item.trigger.kind === "date" || !item.trigger.repeats) pending.delete(id);
    },
    simulateNotificationResponse(value: NotificationResponse) { responses.emit(value); },
    simulateForegroundNotification(value: NotificationRequest) { foreground.emit(value); },
    restore() { responses.close(); foreground.close(); restore(); },
  };
}
