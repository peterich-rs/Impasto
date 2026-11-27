import ApplicationServices
import CoreGraphics
import Foundation
import ImpastoPermissionsCore

public enum DesktopPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        [
            "screenCapture": PermissionStrategy(status: { result(CGPreflightScreenCaptureAccess()) }, request: { _ in
                result(CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess())
            }),
            "accessibility": PermissionStrategy(status: { result(AXIsProcessTrusted()) }, request: { _ in
                let key = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
                return result(AXIsProcessTrustedWithOptions([key: true] as CFDictionary))
            }),
            "fullDiskAccess": PermissionStrategy(status: { fullDiskStatus() }),
        ]
    }

    private static func result(_ granted: Bool) -> PermissionStatus { granted ? .granted : .denied }

    private static func fullDiskStatus() -> PermissionStatus {
        let home = FileManager.default.homeDirectoryForCurrentUser
        for path in ["Library/Application Support/com.apple.TCC/TCC.db", "Library/Safari/History.db"] {
            let url = home.appendingPathComponent(path)
            if let file = try? FileHandle(forReadingFrom: url) {
                try? file.close()
                return .granted
            }
        }
        return .denied
    }
}
