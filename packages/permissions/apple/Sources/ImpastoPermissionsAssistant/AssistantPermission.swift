import ImpastoPermissionsCore
import Intents

public enum AssistantPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        ["assistant": PermissionStrategy(status: { map(INPreferences.siriAuthorizationStatus()) }, request: { _ in
            let current = INPreferences.siriAuthorizationStatus()
            guard current == .notDetermined else { return map(current) }
            try PermissionFailure.requireUsage("NSSiriUsageDescription")
            return await withCheckedContinuation { continuation in
                INPreferences.requestSiriAuthorization { continuation.resume(returning: map($0)) }
            }
        })]
    }

    nonisolated private static func map(_ status: INSiriAuthorizationStatus) -> PermissionStatus {
        switch status {
        case .notDetermined: return .denied
        case .denied: return .permanentlyDenied
        case .restricted: return .restricted
        case .authorized: return .granted
        @unknown default: return .restricted
        }
    }
}
