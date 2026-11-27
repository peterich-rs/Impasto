import CoreMotion
import Foundation
import ImpastoPermissionsCore

public enum MotionPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        let strategy = PermissionStrategy(status: { status() }, request: { _ in
            guard status() == .denied else { return status() }
            try PermissionFailure.requireUsage("NSMotionUsageDescription")
            let manager = CMMotionActivityManager()
            let end = Date()
            await withCheckedContinuation { continuation in
                manager.queryActivityStarting(from: end.addingTimeInterval(-1), to: end, to: .main) { _, _ in
                    continuation.resume()
                }
            }
            return status()
        })
        return ["sensors": strategy, "activityRecognition": strategy]
    }

    private static func status() -> PermissionStatus {
        guard CMMotionActivityManager.isActivityAvailable() else { return .restricted }
        switch CMMotionActivityManager.authorizationStatus() {
        case .notDetermined: return .denied
        case .denied: return .permanentlyDenied
        case .restricted: return .restricted
        case .authorized: return .granted
        @unknown default: return .restricted
        }
    }
}
