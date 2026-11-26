import ImpastoHost
import Foundation
import UIKit
import UniformTypeIdentifiers

@MainActor
public final class ImpastoFileSystemPlugin: NSObject, ImpastoPlugin, UIDocumentPickerDelegate {
    public let identifier = "@impasto/file-system"
    private weak var controller: UIViewController?
    private weak var picker: UIDocumentPickerViewController?
    private var pending: ImpastoNativeRequest?
    private var importing: Task<String, Error>?

    public override init() {}

    public static func register(with host: ImpastoHost) {
        host.registerPlugin(Self())
    }

    public func attached(to controller: ImpastoViewController) {
        self.controller = controller
    }

    public func detached() {
        importing?.cancel()
        importing = nil
        pending?.fail("Document picker disconnected")
        pending = nil
        picker?.dismiss(animated: false)
        picker = nil
        controller = nil
    }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        guard request.operation == "@impasto/file-system/pick" else { return false }
        guard request.isPending else { return true }
        guard pending == nil, let controller, controller.presentedViewController == nil,
              controller.viewIfLoaded?.window?.windowScene?.activationState == .foregroundActive
        else {
            request.fail("No active iOS presentation scene for the document picker")
            return true
        }
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.data], asCopy: false)
        picker.delegate = self
        pending = request
        self.picker = picker
        controller.present(picker, animated: true)
        return true
    }

    public func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        guard controller === picker, let request = pending else { return }
        pending = nil
        picker = nil
        request.resolve()
    }

    public func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        guard controller === picker, let request = pending else { return }
        guard let url = urls.first else {
            documentPickerWasCancelled(controller)
            return
        }
        let task = Task.detached { try DocumentImport.copy(url) }
        importing = task
        Task { @MainActor [weak self] in
            defer {
                if let self, pending?.id == request.id {
                    pending = nil
                    importing = nil
                    picker = nil
                }
            }
            do {
                let path = try await task.value
                if !request.resolve(path) {
                    Task.detached { DocumentImport.discard(path) }
                }
            } catch {
                request.fail(error.localizedDescription)
            }
        }
    }
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
