import ImpastoPermissionsCore
import UserNotifications

public enum NotificationPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        let center = UNUserNotificationCenter.current()
        return [
            "notification": PermissionStrategy(status: { status(await center.notificationSettings()) }, request: { options in
                var requested = UNAuthorizationOptions()
                if options?.alert ?? true { requested.insert(.alert) }
                if options?.badge ?? true { requested.insert(.badge) }
                if options?.sound ?? true { requested.insert(.sound) }
                if options?.provisional == true { requested.insert(.provisional) }
                if options?.criticalAlert == true { requested.insert(.criticalAlert) }
                _ = try await center.requestAuthorization(options: requested)
                return status(await center.notificationSettings())
            }, service: {
                switch status(await center.notificationSettings()) {
                case .granted, .provisional, .limited: return .enabled
                default: return .disabled
                }
            }),
            "criticalAlerts": PermissionStrategy(status: { critical(await center.notificationSettings()) }, request: { _ in
                _ = try await center.requestAuthorization(options: [.criticalAlert])
                return critical(await center.notificationSettings())
            }),
        ]
    }

    private static func status(_ settings: UNNotificationSettings) -> PermissionStatus {
        switch settings.authorizationStatus {
        case .notDetermined: return .denied
        case .denied: return .permanentlyDenied
        case .authorized: return .granted
        case .provisional: return .provisional
        case .ephemeral: return .limited
        @unknown default: return .restricted
        }
    }

    private static func critical(_ settings: UNNotificationSettings) -> PermissionStatus {
        if settings.criticalAlertSetting == .enabled { return .granted }
        if settings.authorizationStatus == .denied { return .permanentlyDenied }
        return settings.criticalAlertSetting == .notSupported ? .restricted : .denied
    }
}
