import Foundation
import ImpastoHost
import UniformTypeIdentifiers
#if os(iOS)
import UIKit
#else
import AppKit
#endif

private struct PickOptions: Decodable {
    let multiple: Bool?
    let accept: [String]?
}

private struct SaveOptions: Decodable {
    let path: String
    let suggestedName: String
    let mimeType: String?
}

private struct ImportedFile: Encodable, Sendable {
    let name: String
    let relativePath: String
    let size: Int
    let mimeType: String?
}

@MainActor
public final class ImpastoFileSystemPlugin: NSObject, ImpastoPlugin {
    public let identifier = "@impasto/file-system"
    private weak var host: ImpastoHost?
    private var pending: ImpastoNativeRequest?
    private var importing: Task<[ImportedFile], Error>?
    #if os(iOS)
    private weak var picker: UIDocumentPickerViewController?
    private var directoryAccess: Set<URL> = []
    #else
    private var panel: NSSavePanel?
    #endif

    public static func register(with host: ImpastoHost) { host.registerPlugin(Self()) }

    public func installed(in host: ImpastoHost) { self.host = host }

    public func detached() {
        importing?.cancel()
        importing = nil
        pending?.fail(code: "cancelled", message: "Document picker detached")
        pending = nil
        #if os(iOS)
        picker?.dismiss(animated: false)
        picker = nil
        for url in directoryAccess { url.stopAccessingSecurityScopedResource() }
        directoryAccess.removeAll()
        #else
        panel?.cancel(nil)
        panel = nil
        #endif
    }

    public func uninstalled() { detached() }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        guard request.operation.hasPrefix("\(identifier)/"),
              ["pickFiles", "pickDirectory", "saveFile"].contains(action(request)) else { return false }
        guard request.isPending else { return true }
        guard pending == nil else {
            request.fail(code: "host-unavailable", message: "A document panel is already open")
            return true
        }
        do {
            switch action(request) {
            case "pickFiles":
                let options = try request.decodeJSON(PickOptions.self)
                try pick(request, types: try Self.types(options.accept ?? []), multiple: options.multiple == true, directory: false)
            case "pickDirectory":
                try pick(request, types: [.folder], multiple: false, directory: true)
            default:
                try save(request, options: request.decodeJSON(SaveOptions.self))
            }
        } catch {
            request.fail(code: error is MissingWindow ? "host-unavailable" : "io", message: error.localizedDescription)
        }
        return true
    }

    private struct MissingWindow: LocalizedError {
        var errorDescription: String? { "No foreground window for the document picker" }
    }

    private func action(_ request: ImpastoNativeRequest) -> String {
        String(request.operation.split(separator: "/").last ?? "")
    }

    private static func types(_ filters: [String]) throws -> [UTType] {
        if filters.isEmpty { return [.data] }
        return try filters.map { filter in
            switch filter.lowercased() {
            case "image", "image/*": return .image
            case "audio", "audio/*": return .audio
            case "video", "video/*": return .movie
            case "document", "*/*": return .data
            default:
                let type = filter.hasPrefix(".") ? UTType(filenameExtension: String(filter.dropFirst())) : UTType(mimeType: filter)
                guard let type else { throw CocoaError(.fileReadUnsupportedScheme) }
                return type
            }
        }
    }

    private func pick(_ request: ImpastoNativeRequest, types: [UTType], multiple: Bool, directory: Bool) throws {
        #if os(iOS)
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: types, asCopy: false)
        picker.allowsMultipleSelection = multiple
        try present(picker, request: request)
        #else
        guard let window = host?.window else { throw MissingWindow() }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = directory
        panel.canChooseFiles = !directory
        panel.allowsMultipleSelection = multiple
        panel.allowedContentTypes = directory ? [] : types
        self.panel = panel
        pending = request
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self, self.panel === panel else { return }
            self.panel = nil
            self.selected(response == .OK ? panel.urls : [], request: request)
        }
        #endif
    }

    private func save(_ request: ImpastoNativeRequest, options: SaveOptions) throws {
        let source = URL(fileURLWithPath: options.path)
        #if os(iOS)
        try present(UIDocumentPickerViewController(forExporting: [source], asCopy: true), request: request)
        #else
        guard let window = host?.window else { throw MissingWindow() }
        let panel = NSSavePanel()
        panel.nameFieldStringValue = options.suggestedName
        if let mime = options.mimeType, let type = UTType(mimeType: mime) { panel.allowedContentTypes = [type] }
        self.panel = panel
        pending = request
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self, self.panel === panel else { return }
            self.panel = nil
            self.pending = nil
            guard response == .OK, let url = panel.url else {
                request.resolveJSON(false)
                return
            }
            Task { @MainActor in
                do {
                    try await Task.detached { try DocumentImport.export(source, to: url) }.value
                    request.resolveJSON(true)
                } catch {
                    request.fail(code: "io", message: error.localizedDescription)
                }
            }
        }
        #endif
    }

    #if os(iOS)
    private func present(_ picker: UIDocumentPickerViewController, request: ImpastoNativeRequest) throws {
        guard let controller = host?.viewController, controller.presentedViewController == nil,
              controller.viewIfLoaded?.window?.windowScene?.activationState == .foregroundActive else { throw MissingWindow() }
        self.picker = picker
        picker.delegate = self
        pending = request
        controller.present(picker, animated: true)
    }
    #endif

    fileprivate func selected(_ urls: [URL], request: ImpastoNativeRequest) {
        guard pending?.id == request.id else { return }
        switch action(request) {
        case "pickDirectory":
            pending = nil
            guard let url = urls.first else { request.resolve(); return }
            #if os(iOS)
            if !directoryAccess.contains(url) && url.startAccessingSecurityScopedResource() {
                directoryAccess.insert(url)
            }
            #endif
            request.resolveJSON(["name": url.lastPathComponent, "path": url.path])
        case "saveFile":
            pending = nil
            request.resolveJSON(!urls.isEmpty)
        default:
            let task = Task.detached { try DocumentImport.copy(urls) }
            importing = task
            Task { @MainActor [weak self] in
                defer {
                    if let self, self.pending?.id == request.id {
                        self.pending = nil
                        self.importing = nil
                    }
                }
                do {
                    let result = try await task.value
                    if !request.resolveJSON(result) {
                        Task.detached { DocumentImport.discard(result) }
                    }
                } catch {
                    request.fail(code: error is CancellationError ? "cancelled" : "io", message: error.localizedDescription)
                }
            }
        }
    }
}

#if os(iOS)
extension ImpastoFileSystemPlugin: UIDocumentPickerDelegate {
    public func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        documentPicker(controller, didPickDocumentsAt: [])
    }

    public func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        guard controller === picker, let pending else { return }
        picker = nil
        selected(urls, request: pending)
    }
}
#endif

private enum DocumentImport {
    static func copy(_ urls: [URL]) throws -> [ImportedFile] {
        var imported: [ImportedFile] = []
        do {
            for url in urls { imported.append(try copyFile(url)) }
            return imported
        } catch {
            discard(imported)
            throw error
        }
    }

    static func copyFile(_ url: URL) throws -> ImportedFile {
        let access = url.startAccessingSecurityScopedResource()
        defer { if access { url.stopAccessingSecurityScopedResource() } }
        let name = url.lastPathComponent
        guard !name.isEmpty, name != ".", name != "..", !name.contains(where: { "/\\:".contains($0) }),
              name.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) })
        else { throw CocoaError(.fileReadInvalidFileName) }
        let manager = FileManager.default
        let base = try manager.url(for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let root = try importRoot(base, create: true)
        let id = UUID().uuidString
        let directory = root.appendingPathComponent(id, isDirectory: true)
        let destination = directory.appendingPathComponent(name)
        try manager.createDirectory(at: directory, withIntermediateDirectories: false)
        do {
            try Task.checkCancellation()
            var coordinationError: NSError?
            var result: Result<ImportedFile, Error>?
            NSFileCoordinator().coordinate(readingItemAt: url, options: .withoutChanges, error: &coordinationError) { source in
                result = Result {
                    let metadata = try source.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey, .contentTypeKey])
                    guard metadata.isRegularFile == true, metadata.isSymbolicLink != true else {
                        throw CocoaError(.fileReadUnsupportedScheme)
                    }
                    try manager.copyItem(at: source, to: destination)
                    return ImportedFile(name: name, relativePath: "Imports/\(id)/\(name)", size: metadata.fileSize ?? 0,
                                        mimeType: metadata.contentType?.preferredMIMEType)
                }
            }
            if let coordinationError { throw coordinationError }
            guard let result else { throw CocoaError(.fileReadUnknown) }
            try Task.checkCancellation()
            return try result.get()
        } catch {
            try? manager.removeItem(at: directory)
            throw error
        }
    }

    private static func importRoot(_ base: URL, create: Bool) throws -> URL {
        let root = base.resolvingSymlinksInPath().appendingPathComponent("Imports", isDirectory: true)
        if create { try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true) }
        let values = try root.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
        guard values.isDirectory == true, values.isSymbolicLink != true else { throw CocoaError(.fileWriteNoPermission) }
        return root
    }

    static func discard(_ files: [ImportedFile]) {
        let manager = FileManager.default
        guard let base = try? manager.url(for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: false),
              let root = try? importRoot(base, create: false) else { return }
        for file in files {
            guard let id = file.relativePath.split(separator: "/").dropFirst().first else { continue }
            try? manager.removeItem(at: root.appendingPathComponent(String(id)))
        }
    }

    #if os(macOS)
    static func export(_ source: URL, to destination: URL) throws {
        let manager = FileManager.default
        var coordinationError: NSError?
        var result: Result<Void, Error>?
        NSFileCoordinator().coordinate(writingItemAt: destination, options: .forReplacing, error: &coordinationError) { target in
            result = Result {
                let stage = target.deletingLastPathComponent().appendingPathComponent(".impasto-\(UUID().uuidString)")
                defer { try? manager.removeItem(at: stage) }
                try manager.copyItem(at: source, to: stage)
                if manager.fileExists(atPath: target.path) { _ = try manager.replaceItemAt(target, withItemAt: stage) }
                else { try manager.moveItem(at: stage, to: target) }
            }
        }
        if let coordinationError { throw coordinationError }
        guard let result else { throw CocoaError(.fileWriteUnknown) }
        try result.get()
    }
    #endif
}
