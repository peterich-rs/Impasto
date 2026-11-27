import Contacts
import ImpastoPermissionsCore

public enum ContactsPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        ["contacts": PermissionStrategy(status: { status() }, request: { _ in
            guard status() == .denied else { return status() }
            try PermissionFailure.requireUsage("NSContactsUsageDescription")
            _ = try await CNContactStore().requestAccess(for: .contacts)
            return status()
        })]
    }

    private static func status() -> PermissionStatus {
        let current = CNContactStore.authorizationStatus(for: .contacts)
        #if os(iOS)
        if #available(iOS 18, *), current == .limited { return .limited }
        #endif
        switch current {
        case .notDetermined: return .denied
        case .denied: return .permanentlyDenied
        case .restricted: return .restricted
        case .authorized: return .granted
        default: return .restricted
        }
    }
}
