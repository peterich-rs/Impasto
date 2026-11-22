import AVFoundation
import Foundation
import Photos
import UIKit
import UniformTypeIdentifiers

/// Shares WindowState between hosts that explicitly retain the same group.
public final class ImpastoWindowGroup: Sendable {
    private let address: UInt

    public init() {
        address = UInt(bitPattern: impasto_ios_window_group_new())
    }

    var handle: OpaquePointer? { OpaquePointer(bitPattern: address) }

    deinit {
        impasto_ios_window_group_free(handle)
    }
}

@MainActor
final class ImpastoPlatformServices: NSObject, UIDocumentPickerDelegate, UIAdaptivePresentationControllerDelegate {
    private struct Request: Decodable {
        let operation: String
        let argument: String
        let names: [String]
        var plugin: PluginRequest? = nil
    }

    private struct PluginRequest: Decodable {
        let identifier: String
        let method: String
        let payload: String
    }

    private final class PluginWork {
        var finish: ((UInt32, String?) -> Void)?
        var work: Task<Void, Never>?
        var timeout: Task<Void, Never>?

        init(finish: @escaping (UInt32, String?) -> Void) {
            self.finish = finish
        }

        deinit {
            work?.cancel()
            timeout?.cancel()
        }
    }

    private static let maxPluginCalls = 32
    private static let maxPluginJSONBytes = 32 * 1024
    private static let pluginTimeoutNanoseconds: UInt64 = 10_000_000_000
    private var pluginCalls: [UInt64: PluginWork] = [:]

    private enum RequestID: Equatable {
        case runtime(UInt64)
        case native(UUID)
    }

    private struct Pending {
        let id: RequestID
        let request: Request
        let file: URL?
        let isPending: () -> Bool
        let finish: (UInt32, String?) -> Void
        let presenter: () -> UIViewController?
    }

    private var pending: [Pending] = []
    private var active: Pending?
    private var modal: UIViewController?
    private var sharedFile: URL?
    private var work: Task<Void, Never>?
    private var importedDocument: Task<String, Error>?

    func accept(
        _ id: UInt64, message: String, isPending: @escaping () -> Bool,
        presenter: @escaping () -> UIViewController?,
        plugin: (String) -> (any ImpastoPlugin)?,
        pluginIsPending: @escaping () -> Bool,
        finish: @escaping (UInt32, String?) -> Void
    ) {
        guard isPending() else { return }
        guard let request = try? JSONDecoder().decode(Request.self, from: Data(message.utf8)) else {
            finish(2, "Invalid iOS platform request")
            return
        }
        if request.operation == "nativePlugin" {
            guard let call = request.plugin else {
                finish(2, "Invalid native plugin request")
                return
            }
            acceptPlugin(id, call: call, plugin: plugin(call.identifier), isPending: pluginIsPending, finish: finish)
            return
        }
        // A status query does not need to wait for a user to dismiss a sheet.
        if request.operation == "permissionStatus" {
            finish(0, MediaPermission(request.argument)?.status ?? "restricted")
            return
        }
        pending.append(Pending(id: .runtime(id), request: request, file: nil, isPending: isPending, finish: finish, presenter: presenter))
        advance()
    }

    private func acceptPlugin(
        _ id: UInt64, call: PluginRequest, plugin: (any ImpastoPlugin)?,
        isPending: @escaping () -> Bool, finish: @escaping (UInt32, String?) -> Void
    ) {
        guard isPending(), pluginCalls[id] == nil else { return }
        guard let plugin else {
            finish(2, "No native plugin is registered as '\(call.identifier)'")
            return
        }
        guard pluginCalls.count < Self.maxPluginCalls else {
            finish(2, "Too many running native plugin calls")
            return
        }
        let request = ImpastoPluginCall(method: call.method, payload: Data(call.payload.utf8))
        let invocation = PluginWork { status, payload in
            if isPending() { finish(status, payload) }
        }
        pluginCalls[id] = invocation
        invocation.work = Task { @MainActor [weak self] in
            defer { self?.pluginCalls.removeValue(forKey: id) }
            do {
                try Task.checkCancellation()
                let result = try await plugin.handle(request)
                try Task.checkCancellation()
                guard result.count <= Self.maxPluginJSONBytes else {
                    self?.finishPlugin(id, status: 2, payload: "Native plugin JSON exceeds the 32 KiB limit")
                    return
                }
                guard let json = String(data: result, encoding: .utf8), !json.utf8.contains(0) else {
                    self?.finishPlugin(id, status: 2, payload: "Invalid native plugin UTF-8 response")
                    return
                }
                self?.finishPlugin(id, status: 0, payload: json)
            } catch {
                let description = error.localizedDescription.replacingOccurrences(of: "\0", with: "\u{FFFD}")
                let message = String(decoding: description.utf8.prefix(1024), as: UTF8.self)
                self?.finishPlugin(id, status: 2, payload: message)
            }
        }
        invocation.timeout = Task { @MainActor [weak self] in
            do {
                try await Task.sleep(nanoseconds: Self.pluginTimeoutNanoseconds)
            } catch {
                return
            }
            self?.finishPlugin(id, status: 2, payload: "Native plugin call timed out")
            self?.pluginCalls[id]?.work?.cancel()
        }
    }

    private func finishPlugin(_ id: UInt64, status: UInt32, payload: String?) {
        guard let invocation = pluginCalls[id], let finish = invocation.finish else { return }
        invocation.finish = nil
        invocation.timeout?.cancel()
        invocation.timeout = nil
        finish(status, payload)
        // Keep the invocation in the budget until its native Task returns.
    }

    func cancelNativePlugins() {
        for id in pluginCalls.keys {
            finishPlugin(id, status: 2, payload: "iOS presentation context was disconnected")
            pluginCalls[id]?.work?.cancel()
        }
    }

    func share(
        file: URL, presenter: @escaping () -> UIViewController?,
        complete: @escaping (Result<Bool, Error>) -> Void
    ) {
        guard file.isFileURL else {
            complete(.failure(CocoaError(.fileReadUnsupportedScheme)))
            return
        }
        let request = Request(operation: "shareFile", argument: "", names: [])
        pending.append(Pending(
            id: .native(UUID()), request: request, file: file,
            isPending: { true },
            finish: { status, message in
                Task { @MainActor in
                    switch status {
                    case 0: complete(.success(true))
                    case 1: complete(.success(false))
                    default: complete(.failure(FileShareFailure(message: message ?? "iOS file share failed")))
                    }
                }
            },
            presenter: presenter
        ))
        advance()
    }

    func cancel() {
        let old = (active.map { [$0] } ?? []) + pending
        active = nil
        pending.removeAll()
        work?.cancel()
        work = nil
        importedDocument?.cancel()
        importedDocument = nil
        modal?.dismiss(animated: false)
        modal = nil
        releaseSharedFile()
        for request in old {
            request.finish(2, "iOS presentation context was disconnected")
        }
    }

    private func advance() {
        guard active == nil else { return }
        while !pending.isEmpty {
            let next = pending.removeFirst()
            guard next.isPending() else { continue }
            active = next
            if let presenter = next.presenter(),
               presenter.viewIfLoaded?.window?.windowScene?.activationState == .foregroundActive {
                perform(next, presenter: presenter)
            } else {
                complete(next.id, result: 2, data: "No active iOS presentation scene")
            }
            return
        }
    }

    private func perform(_ next: Pending, presenter: UIViewController) {
        let id = next.id
        switch next.request.operation {
        case "openURL":
            guard let url = URL(string: next.request.argument),
                  let scheme = url.scheme?.lowercased(),
                  !["file", "javascript", "data"].contains(scheme),
                  (scheme != "http" && scheme != "https") || url.host != nil
            else {
                complete(id, result: 2, data: "Invalid external URL")
                return
            }
            open(url, id: id, reportBoolean: false)
        case "openAppSettings":
            guard let url = URL(string: UIApplication.openSettingsURLString) else {
                complete(id, result: 0, data: "false")
                return
            }
            open(url, id: id, reportBoolean: true)
        case "shareText", "shareFile":
            guard presenter.presentedViewController == nil else {
                complete(id, result: 2, data: "The scene is already presenting a sheet")
                return
            }
            let item: Any
            if next.request.operation == "shareFile" {
                guard let file = next.file else {
                    complete(id, result: 2, data: "Invalid file URL")
                    return
                }
                if file.startAccessingSecurityScopedResource() { sharedFile = file }
                item = file
            } else {
                item = next.request.argument
            }
            let sheet = UIActivityViewController(activityItems: [item], applicationActivities: nil)
            if let popover = sheet.popoverPresentationController {
                popover.sourceView = presenter.view
                popover.sourceRect = CGRect(
                    x: presenter.view.bounds.midX, y: presenter.view.bounds.midY, width: 1, height: 1
                )
                popover.permittedArrowDirections = []
            }
            sheet.completionWithItemsHandler = { [weak self] _, completed, _, error in
                let message = error?.localizedDescription
                Task { @MainActor [weak self] in
                    self?.complete(id, result: message == nil ? (completed ? 0 : 1) : 2, data: message)
                }
            }
            modal = sheet
            sheet.presentationController?.delegate = self
            presenter.present(sheet, animated: true)
        case "pickDocument":
            guard presenter.presentedViewController == nil else {
                complete(id, result: 2, data: "The scene is already presenting a sheet")
                return
            }
            let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.data], asCopy: false)
            picker.delegate = self
            picker.presentationController?.delegate = self
            modal = picker
            presenter.present(picker, animated: true)
        case "requestPermissions":
            work = Task { @MainActor [weak self] in
                do {
                    let statuses = try await MediaPermissionPrompt.shared.request(next.request.names)
                    try Task.checkCancellation()
                    let payload = try JSONEncoder().encode(statuses)
                    self?.complete(id, result: 0, data: String(decoding: payload, as: UTF8.self))
                } catch {
                    self?.complete(id, result: 2, data: error.localizedDescription)
                }
            }
        default:
            complete(id, result: 2, data: "Unknown iOS platform operation")
        }
    }

    private func open(_ url: URL, id: RequestID, reportBoolean: Bool) {
        work = Task { @MainActor [weak self] in
            let opened = await UIApplication.shared.open(url)
            if reportBoolean {
                self?.complete(id, result: 0, data: opened ? "true" : "false")
            } else {
                self?.complete(id, result: opened ? 0 : 2, data: opened ? nil : "iOS could not open the URL")
            }
        }
    }

    @discardableResult
    private func complete(_ id: RequestID, result: UInt32, data: String? = nil) -> Bool {
        guard let active, active.id == id else { return false }
        self.active = nil
        self.modal = nil
        self.work = nil
        self.importedDocument = nil
        releaseSharedFile()
        let accepted = active.isPending()
        if accepted { active.finish(result, data) }
        // UIKit may still be dismissing the previous controller on the delegate stack.
        Task { @MainActor [weak self] in self?.advance() }
        return accepted
    }

    private func releaseSharedFile() {
        sharedFile?.stopAccessingSecurityScopedResource()
        sharedFile = nil
    }

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        guard modal === presentationController.presentedViewController, let id = active?.id,
              importedDocument == nil
        else { return }
        complete(id, result: 1)
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        guard controller === modal, let id = active?.id else { return }
        complete(id, result: 1)
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        guard controller === modal, let id = active?.id else { return }
        guard let url = urls.first else {
            complete(id, result: 1)
            return
        }
        let importTask = Task.detached { try DocumentImport.copy(url) }
        importedDocument = importTask
        work = Task { @MainActor [weak self] in
            do {
                let relativePath = try await importTask.value
                if self?.complete(id, result: 0, data: relativePath) != true {
                    Task.detached { DocumentImport.discard(relativePath) }
                }
            } catch {
                self?.complete(id, result: 2, data: error.localizedDescription)
            }
        }
    }
}

private struct FileShareFailure: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

private enum DocumentImport {
    static func copy(_ url: URL) throws -> String {
        let access = url.startAccessingSecurityScopedResource()
        defer { if access { url.stopAccessingSecurityScopedResource() } }
        let name = url.lastPathComponent
        guard !name.isEmpty, name != ".", name != "..", !name.contains("/"), !name.contains("\\"),
              name.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) })
        else { throw CocoaError(.fileReadInvalidFileName) }
        let manager = FileManager.default
        let base = try manager.url(for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let folder = "Imports/\(UUID().uuidString)"
        let destination = base.appendingPathComponent(folder, isDirectory: true)
        try manager.createDirectory(at: destination, withIntermediateDirectories: true)
        do {
            try Task.checkCancellation()
            var coordinationError: NSError?
            var copyResult: Result<Void, Error>?
            NSFileCoordinator().coordinate(readingItemAt: url, options: .withoutChanges, error: &coordinationError) { source in
                copyResult = Result {
                    let properties = try source.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
                    guard properties.isRegularFile == true, properties.isSymbolicLink != true else {
                        throw CocoaError(.fileReadUnsupportedScheme)
                    }
                    try manager.copyItem(at: source, to: destination.appendingPathComponent(name))
                }
            }
            if let coordinationError { throw coordinationError }
            guard let copyResult else { throw CocoaError(.fileReadUnknown) }
            try copyResult.get()
            try Task.checkCancellation()
            return "\(folder)/\(name)"
        } catch {
            try? manager.removeItem(at: destination)
            throw error
        }
    }

    static func discard(_ relativePath: String) {
        let manager = FileManager.default
        guard let base = try? manager.url(for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: false)
        else { return }
        try? manager.removeItem(at: base.appendingPathComponent(relativePath).deletingLastPathComponent())
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
