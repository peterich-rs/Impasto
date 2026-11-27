import EventKit
import ImpastoPermissionsCore

public enum CalendarPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        ["calendarWriteOnly": strategy(writeOnly: true), "calendarFullAccess": strategy(writeOnly: false)]
    }

    private static func strategy(writeOnly: Bool) -> PermissionStrategy {
        PermissionStrategy(status: { status(writeOnly: writeOnly) }, request: { _ in
            let current = status(writeOnly: writeOnly)
            guard current == .denied || current == .limited else { return current }
            let store = EKEventStore()
            if #available(iOS 17, macOS 14, *) {
                if writeOnly {
                    try PermissionFailure.requireUsage("NSCalendarsWriteOnlyAccessUsageDescription")
                    _ = try await store.requestWriteOnlyAccessToEvents()
                } else {
                    try PermissionFailure.requireUsage("NSCalendarsFullAccessUsageDescription")
                    _ = try await store.requestFullAccessToEvents()
                }
            } else {
                try PermissionFailure.requireUsage("NSCalendarsUsageDescription")
                _ = try await store.requestAccess(to: .event)
            }
            return status(writeOnly: writeOnly)
        })
    }

    private static func status(writeOnly: Bool) -> PermissionStatus {
        let current = EKEventStore.authorizationStatus(for: .event)
        if #available(iOS 17, macOS 14, *) {
            if current == .fullAccess { return .granted }
            if current == .writeOnly { return writeOnly ? .granted : .limited }
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
