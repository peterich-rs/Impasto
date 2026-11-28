export type BiometryType = "face" | "fingerprint" | "iris" | "opticId" | "unknown";
export type BiometricSecurityLevel = "strong" | "weak" | "none";

export type BiometricFailureReason =
  | "cancelled"
  | "user-fallback"
  | "failed"
  | "lockout"
  | "permanent-lockout"
  | "not-enrolled"
  | "passcode-not-set"
  | "not-available"
  | "system-cancelled";

export type BiometricsErrorCode =
  | "host-unavailable"
  | "invalid-argument"
  | "missing-config"
  | "busy"
  | "platform";

export interface BiometricCapabilities {
  /** 设备硬件/系统是否具备生物识别或凭据认证能力 */
  readonly isAvailable: boolean;
  /** 用户是否已在系统内录入了至少一种生物特征 */
  readonly isEnrolled: boolean;
  /** 是否已设置系统锁屏密码/PIN/图案（可用于 allowDeviceCredential） */
  readonly hasDeviceCredential: boolean;
  readonly biometryTypes: readonly BiometryType[];
  readonly securityLevel: BiometricSecurityLevel;
}

export interface AuthenticateOptions {
  /** 向用户展示的认证原因（iOS / macOS 必填，非空） */
  readonly reason: string;
  /** Android BiometricPrompt 主标题 */
  readonly title?: string;
  readonly subtitle?: string;
  readonly description?: string;
  /** 取消按钮文案（注：Android allowDeviceCredential=true 时由系统隐藏此按钮） */
  readonly cancelLabel?: string;
  /** iOS 生物识别失败后切换到回退动作的自定义按钮文案（空串表示隐藏） */
  readonly fallbackLabel?: string;
  /** 是否允许在生物识别不可用/失败时降级到设备锁屏密码/PIN/图案，默认 false */
  readonly allowDeviceCredential?: boolean;
  /** macOS 是否允许佩戴配对 Apple Watch 解锁，默认 false */
  readonly allowWatch?: boolean;
  /** Android 是否要求识别后手动点击“确认”，默认 true */
  readonly confirmationRequired?: boolean;
  readonly signal?: AbortSignal;
}

export type BiometricAuthResult =
  | { readonly authenticated: true }
  | {
      readonly authenticated: false;
      readonly reason: BiometricFailureReason;
      readonly message: string;
    };
