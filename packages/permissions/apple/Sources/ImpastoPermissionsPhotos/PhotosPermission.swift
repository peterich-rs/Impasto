import ImpastoPermissionsCore
import ImpastoHost
import Photos
#if os(iOS)
import PhotosUI

import UIKit
#endif

public enum PhotosPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        let photos = strategy(.readWrite, key: "NSPhotoLibraryUsageDescription")
        return [
            "photos": photos,
            "videos": photos,
            "photosAddOnly": strategy(.addOnly, key: "NSPhotoLibraryAddUsageDescription"),
        ]
    }

    private static func strategy(_ level: PHAccessLevel, key: String) -> PermissionStrategy {
        let picker: ((ImpastoHost) async throws -> Bool)?
        if level == .readWrite { picker = { host in try await reselect(host) } }
        else { picker = nil }
        return PermissionStrategy(status: { map(PHPhotoLibrary.authorizationStatus(for: level)) }, request: { _ in
            let current = PHPhotoLibrary.authorizationStatus(for: level)
            guard current == .notDetermined else { return map(current) }
            try PermissionFailure.requireUsage(key)
            return map(await PHPhotoLibrary.requestAuthorization(for: level))
        }, picker: picker)
    }

    private static func map(_ status: PHAuthorizationStatus) -> PermissionStatus {
        switch status {
        case .notDetermined: return .denied
        case .denied: return .permanentlyDenied
        case .restricted: return .restricted
        case .authorized: return .granted
        case .limited: return .limited
        @unknown default: return .restricted
        }
    }

    private static func reselect(_ host: ImpastoHost) async throws -> Bool {
        #if os(iOS)
        guard PHPhotoLibrary.authorizationStatus(for: .readWrite) == .limited else { return false }
        guard let controller = host.viewController, controller.viewIfLoaded?.window != nil else {
            throw PermissionFailure("host-unavailable", "No visible controller can present the photo picker")
        }
        var presenter: UIViewController = controller
        while let next = presenter.presentedViewController { presenter = next }
        _ = await PHPhotoLibrary.shared().presentLimitedLibraryPicker(from: presenter)
        return true
        #else
        return false
        #endif
    }
}
