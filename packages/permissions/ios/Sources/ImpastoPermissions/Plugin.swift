import ImpastoHost
import AVFoundation
import Foundation
import Photos
import UIKit

@MainActor
public final class ImpastoPermissionsPlugin: ImpastoPlugin {
    public let identifier = "@impasto/permissions"
    private var work: [UInt64: Task<Void, Never>] = [:]

    public init() {}

    public static func register(with host: ImpastoHost) {
        host.registerPlugin(Self())
    }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        switch request.operation {
        case "@impasto/permissions/status":
            request.resolve(MediaPermission(request.payload)?.status ?? "restricted")
        case "@impasto/permissions/request":
            guard let names = try? JSONDecoder().decode([String].self, from: Data(request.payload.utf8)) else {
                request.fail("Invalid permission request")
                return true
            }
            work[request.id] = Task { @MainActor [weak self] in
                defer { self?.work.removeValue(forKey: request.id) }
                do {
                    let statuses = try await MediaPermissionPrompt.shared.request(names)
                    try Task.checkCancellation()
                    let data = try JSONEncoder().encode(statuses)
                    request.resolve(String(decoding: data, as: UTF8.self))
                } catch {
                    request.fail(error.localizedDescription)
                }
            }
        case "@impasto/permissions/settings":
            work[request.id] = Task { @MainActor [weak self] in
                defer { self?.work.removeValue(forKey: request.id) }
                guard let url = URL(string: UIApplication.openSettingsURLString) else {
                    request.resolve("false")
                    return
                }
                request.resolve(await UIApplication.shared.open(url) ? "true" : "false")
            }
        default:
            return false
        }
        return true
    }

    public func detached() {
        for task in work.values { task.cancel() }
        work.removeAll()
    }
}

@MainActor
private final class MediaPermissionPrompt {
    static let shared = MediaPermissionPrompt()

    // UIKit permission prompts are process-wide, even for independent scenes.
    private var busy = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    func request(_ names: [String]) async throws -> [String] {
        if busy {
            await withCheckedContinuation { waiters.append($0) }
        } else {
            busy = true
        }
        defer {
            if waiters.isEmpty { busy = false }
            else { waiters.removeFirst().resume() }
        }
        var statuses: [String] = []
        for name in names {
            try Task.checkCancellation()
            guard let permission = MediaPermission(name) else {
                statuses.append("restricted")
                continue
            }
            statuses.append(try await permission.request())
        }
        return statuses
    }
}

private enum MediaPermission {
    case capture(AVMediaType, key: String)
    case photos

    init?(_ name: String) {
        switch name {
        case "camera": self = .capture(.video, key: "NSCameraUsageDescription")
        case "microphone": self = .capture(.audio, key: "NSMicrophoneUsageDescription")
        case "photos", "photoLibrary": self = .photos
        default: return nil
        }
    }

    var status: String {
        switch self {
        case .capture(let media, _):
            switch AVCaptureDevice.authorizationStatus(for: media) {
            case .notDetermined: return "denied"
            case .restricted: return "restricted"
            case .denied: return "permanentlyDenied"
            case .authorized: return "granted"
            @unknown default: return "restricted"
            }
        case .photos:
            switch PHPhotoLibrary.authorizationStatus(for: .readWrite) {
            case .notDetermined: return "denied"
            case .restricted: return "restricted"
            case .denied: return "permanentlyDenied"
            case .authorized: return "granted"
            case .limited: return "limited"
            @unknown default: return "restricted"
            }
        }
    }

    func request() async throws -> String {
        guard status == "denied" else { return status }
        let key: String
        switch self {
        case .capture(_, let usageKey): key = usageKey
        case .photos: key = "NSPhotoLibraryUsageDescription"
        }
        guard let description = Bundle.main.object(forInfoDictionaryKey: key) as? String,
              !description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else { throw MissingUsageDescription(key: key) }

        switch self {
        case .capture(let media, _): _ = await AVCaptureDevice.requestAccess(for: media)
        case .photos: _ = await PHPhotoLibrary.requestAuthorization(for: .readWrite)
        }
        return status
    }
}

private struct MissingUsageDescription: LocalizedError {
    let key: String
    var errorDescription: String? { "Configure \(key) before requesting this iOS permission" }
}
