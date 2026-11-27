import AVFoundation
import ImpastoPermissionsCore

public enum CameraPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        ["camera": PermissionStrategy(status: { status() }, request: { _ in
            guard status() == .denied else { return status() }
            try PermissionFailure.requireUsage("NSCameraUsageDescription")
            _ = await AVCaptureDevice.requestAccess(for: .video)
            return status()
        })]
    }

    private static func status() -> PermissionStatus {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .notDetermined: return .denied
        case .denied: return .permanentlyDenied
        case .restricted: return .restricted
        case .authorized: return .granted
        @unknown default: return .restricted
        }
    }
}
