import AVFoundation
import ImpastoPermissionsCore

public enum MicrophonePermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        ["microphone": PermissionStrategy(status: { status() }, request: { _ in
            guard status() == .denied else { return status() }
            try PermissionFailure.requireUsage("NSMicrophoneUsageDescription")
            _ = await AVCaptureDevice.requestAccess(for: .audio)
            return status()
        })]
    }

    private static func status() -> PermissionStatus {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .notDetermined: return .denied
        case .denied: return .permanentlyDenied
        case .restricted: return .restricted
        case .authorized: return .granted
        @unknown default: return .restricted
        }
    }
}
