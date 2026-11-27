import EventKit
import ImpastoPermissionsCore

public enum RemindersPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        ["reminders": PermissionStrategy(status: { status() }, request: { _ in
            guard status() == .denied else { return status() }
            let store = EKEventStore()
            if #available(iOS 17, macOS 14, *) {
                try PermissionFailure.requireUsage("NSRemindersFullAccessUsageDescription")
                _ = try await store.requestFullAccessToReminders()
            } else {
                try PermissionFailure.requireUsage("NSRemindersUsageDescription")
                _ = try await store.requestAccess(to: .reminder)
            }
            return status()
        })]
    }

    private static func status() -> PermissionStatus {
        let current = EKEventStore.authorizationStatus(for: .reminder)
        if #available(iOS 17, macOS 14, *) {
            if current == .fullAccess { return .granted }
        } else if current == .authorized {
            return .granted
        }
        switch current {
        case .notDetermined: return .denied
        case .restricted: return .restricted
        case .denied: return .permanentlyDenied
        default: return .restricted
        }
    }
}
