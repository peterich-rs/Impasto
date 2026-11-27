import ImpastoPermissionsCore
import MediaPlayer

public enum MediaLibraryPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        let strategy = PermissionStrategy(status: { map(MPMediaLibrary.authorizationStatus()) }, request: { _ in
            let current = MPMediaLibrary.authorizationStatus()
            guard current == .notDetermined else { return map(current) }
            try PermissionFailure.requireUsage("NSAppleMusicUsageDescription")
            return await withCheckedContinuation { continuation in
                MPMediaLibrary.requestAuthorization { continuation.resume(returning: map($0)) }
            }
        })
        return ["audio": strategy, "mediaLibrary": strategy]
    }

    nonisolated private static func map(_ status: MPMediaLibraryAuthorizationStatus) -> PermissionStatus {
        switch status {
        case .notDetermined: return .denied
        case .denied: return .permanentlyDenied
        case .restricted: return .restricted
        case .authorized: return .granted
        @unknown default: return .restricted
        }
    }
}
