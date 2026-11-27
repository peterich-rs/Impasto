import AppTrackingTransparency
import ImpastoPermissionsCore
import UIKit

public enum TrackingPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        ["appTrackingTransparency": PermissionStrategy(status: { status(ATTrackingManager.trackingAuthorizationStatus) }, request: { _ in
            let current = ATTrackingManager.trackingAuthorizationStatus
            guard current == .notDetermined else { return status(current) }
            guard UIApplication.shared.applicationState == .active else {
                throw PermissionFailure("host-unavailable", "Tracking authorization requires an active application")
            }
            try PermissionFailure.requireUsage("NSUserTrackingUsageDescription")
            return status(await ATTrackingManager.requestTrackingAuthorization())
        })]
    }

    private static func status(_ current: ATTrackingManager.AuthorizationStatus) -> PermissionStatus {
        switch current {
        case .notDetermined: return .denied
        case .denied: return .permanentlyDenied
        case .restricted: return .restricted
        case .authorized: return .granted
        @unknown default: return .restricted
        }
    }
}
