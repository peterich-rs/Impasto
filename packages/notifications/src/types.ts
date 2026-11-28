import type { DirectoryScope } from "@impasto/file-system";

export type NotificationImportance = "min" | "low" | "default" | "high";
export type InterruptionLevel = "passive" | "active" | "timeSensitive" | "critical";

export type NotificationsErrorCode =
  | "permission-denied"
  | "exact-alarm-denied"
  | "invalid-argument"
  | "outside-scope"
  | "file-not-found"
  | "host-unavailable"
  | "platform";

export interface NotificationAttachment {
  /** 仅允许 @impasto/file-system 沙盒相对路径（由 Rust resolve_path 校验并交原生安全复制） */
  readonly path: string;
  readonly scope?: DirectoryScope;
  readonly id?: string;
}

export interface NotificationRequest {
  /** 应用级稳定唯一 ID；相同 ID 再次触发将更新/覆盖既有通知 */
  readonly id: string;
  readonly title: string;
  readonly subtitle?: string;
  readonly body: string;
  /** Android 渠道 ID（API 26+），缺省使用内置 "impasto_default" */
  readonly channelId?: string;
  readonly categoryId?: string;
  readonly threadId?: string;
  readonly badge?: number;
  readonly sound?: "default" | "none" | string;
  readonly data?: Record<string, string>;
  readonly attachments?: readonly NotificationAttachment[];
  readonly interruptionLevel?: InterruptionLevel;
}

export type NotificationTrigger =
  | { readonly kind: "date"; readonly atMs: number; readonly exact?: boolean }
  | { readonly kind: "interval"; readonly everySeconds: number; readonly repeats?: boolean }
  | {
      readonly kind: "calendar";
      readonly year?: number;
      readonly month?: number; // 1..=12
      readonly day?: number;
      readonly weekday?: number; // 1=Sun .. 7=Sat（对齐 UNCalendarNotificationTrigger）
      readonly hour?: number;
      readonly minute?: number;
      readonly second?: number;
      readonly repeats?: boolean;
      readonly timeZone?: string;
    };

export interface NotificationChannel {
  readonly id: string;
  readonly name: string;
  readonly description?: string;
  readonly importance: NotificationImportance;
  readonly sound?: "default" | "none" | string;
  readonly vibration?: boolean;
  readonly lights?: boolean;
  readonly showBadge?: boolean;
}

export interface NotificationAction {
  readonly id: string;
  readonly title: string;
  readonly foreground?: boolean; // 点击时是否将 App 拉到前台，默认 true
  readonly destructive?: boolean;
  readonly authenticationRequired?: boolean;
  readonly textInput?: { readonly buttonTitle: string; readonly placeholder: string };
}

export interface NotificationCategory {
  readonly id: string;
  readonly actions: readonly NotificationAction[];
}

export interface PendingNotification {
  readonly id: string;
  readonly title: string;
  readonly body: string;
  readonly trigger: NotificationTrigger;
  readonly data: Record<string, string>;
}

export interface NotificationResponse {
  readonly notificationId: string;
  /** 点击通知本体时为 "default"，点击交互按钮时为对应 action.id */
  readonly actionId: string;
  readonly userText?: string;
  readonly data: Record<string, string>;
  readonly receivedAtMs: number;
}

export interface ForegroundPresentationOptions {
  readonly alert: boolean;
  readonly sound: boolean;
  readonly badge: boolean;
}
