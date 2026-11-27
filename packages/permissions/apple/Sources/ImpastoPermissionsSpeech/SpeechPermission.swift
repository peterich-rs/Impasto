import ImpastoPermissionsCore
import Speech

public enum SpeechPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        ["speech": PermissionStrategy(status: { status() }, request: { _ in
            guard status() == .denied else { return status() }
            try PermissionFailure.requireUsage("NSSpeechRecognitionUsageDescription")
            return await withCheckedContinuation { continuation in
                SFSpeechRecognizer.requestAuthorization { continuation.resume(returning: map($0)) }
            }
        })]
    }

    nonisolated private static func map(_ status: SFSpeechRecognizerAuthorizationStatus) -> PermissionStatus {
        switch status {
        case .notDetermined: return .denied
        case .denied: return .permanentlyDenied
        case .restricted: return .restricted
        case .authorized: return .granted
        @unknown default: return .restricted
        }
    }

    private static func status() -> PermissionStatus { map(SFSpeechRecognizer.authorizationStatus()) }
}
