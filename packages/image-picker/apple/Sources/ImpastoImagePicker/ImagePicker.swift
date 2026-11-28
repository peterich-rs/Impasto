import AVFoundation
import Foundation
import ImageIO
import ImpastoHost
import UniformTypeIdentifiers
#if os(iOS)
import PhotosUI
import UIKit
#else
import AppKit
#endif

private struct Selection: Decodable, Sendable {
    struct Processing: Decodable, Sendable {
        let fixOrientation: Bool?
        let stripMetadata: Bool?
        let maxWidth: Int?
        let maxHeight: Int?
        let quality: Int?
        let outputFormat: String?

        var requiresDecode: Bool {
            fixOrientation != false || stripMetadata != false || maxWidth != nil || maxHeight != nil
                || quality != nil || (outputFormat != nil && outputFormat != "original")
        }
    }
    let staging: String
    let mediaType: String?
    let multiple: Bool?
    let maxSelection: Int?
    let preferredCamera: String?
    let maxDurationSec: Double?
    let quality: String?
    let process: Processing?

    var types: [UTType] {
        switch mediaType {
        case "video": [.movie]
        case "any": [.image, .movie]
        default: [.image]
        }
    }
}
private struct Staged: Encodable, Sendable {
    let kind: String, path: String, fileName: String, mimeType: String
    let width: Int?, height: Int?, durationMs: Double?
}

@MainActor
public final class ImpastoImagePickerPlugin: NSObject, ImpastoPlugin {
    public let identifier = "@impasto/image-picker"
    private weak var host: ImpastoHost?
    private var pending: ImpastoNativeRequest?
    private var options: Selection?
    private var importing: Task<Void, Never>?
    private var camera = false
    #if os(iOS)
    private weak var presented: UIViewController?
    #else
    private var panel: NSOpenPanel?
    #endif

    public static func register(with host: ImpastoHost) { host.registerPlugin(Self()) }
    public static func register(with host: ImpastoHost, enabled: Set<String>, registrars: [any ImpastoImagePickerFeature.Type]) {
        let plugin = Self()
        plugin.camera = enabled.contains("camera")
        host.registerPlugin(plugin)
    }
    public func installed(in host: ImpastoHost) { self.host = host }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        guard request.operation.hasPrefix("\(identifier)/") else { return false }
        let action = String(request.operation.dropFirst(identifier.count + 1))
        if action == "checkCamera" { cameraSupport(request); return true }
        guard ["pick", "photo", "video"].contains(action) else { return false }
        guard pending == nil else { request.fail(code: "busy", message: "A media picker is already open"); return true }
        do {
            let options = try request.decodeJSON(Selection.self)
            if action == "pick" { pick(request, options) } else {
                #if os(iOS)
                capture(request, options, video: action == "video")
                #else
                request.fail(code: "not-supported", message: "System camera capture is unavailable on macOS")
                #endif
            }
        } catch { request.fail(code: "io", message: "The media picker could not open") }
        return true
    }

    @discardableResult
    private func cameraSupport(_ request: ImpastoNativeRequest, checkOnly: Bool = true) -> Bool {
        #if os(iOS)
        guard camera else { request.fail(code: "not-supported", message: "Enable the image picker camera feature"); return false }
        guard UIImagePickerController.isSourceTypeAvailable(.camera) else {
            request.fail(code: "camera-unavailable", message: "This device has no system camera"); return false
        }
        if checkOnly { request.resolve() }
        return true
        #else
        request.fail(code: "not-supported", message: "System camera capture is unavailable on macOS")
        return false
        #endif
    }

    private func pick(_ request: ImpastoNativeRequest, _ options: Selection) {
        #if os(iOS)
        guard let controller = host?.viewController, controller.presentedViewController == nil, controller.viewIfLoaded?.window != nil else {
            request.fail(code: "host-unavailable", message: "No visible window for the media picker"); return
        }
        var config = PHPickerConfiguration(photoLibrary: .shared())
        config.selectionLimit = options.multiple == true ? (options.maxSelection ?? 0) : 1
        config.preferredAssetRepresentationMode = .current
        config.filter = options.mediaType == "video" ? .videos : options.mediaType == "any" ? .any(of: [.images, .videos]) : .images
        let picker = PHPickerViewController(configuration: config)
        picker.delegate = self
        self.options = options
        pending = request
        presented = picker
        controller.present(picker, animated: true)
        #else
        guard let window = host?.window else {
            request.fail(code: "host-unavailable", message: "No visible window for the media picker"); return
        }
        let picker = NSOpenPanel()
        picker.canChooseFiles = true
        picker.canChooseDirectories = false
        picker.allowsMultipleSelection = options.multiple == true
        picker.allowedContentTypes = options.types
        panel = picker
        pending = request
        picker.beginSheetModal(for: window) { [weak self] answer in
            guard let self, self.pending?.id == request.id else { return }
            self.panel = nil
            guard answer == .OK else { self.finish(request, .success([])); return }
            let urls = Array(picker.urls.prefix(options.maxSelection ?? picker.urls.count))
            self.importing = Task { @MainActor [weak self] in
                do {
                    let staged = try await Task.detached { try await Self.stage(urls, options) }.value
                    self?.finish(request, .success(staged))
                } catch { self?.finish(request, .failure(error)) }
            }
        }
        #endif
    }

    private func finish(_ request: ImpastoNativeRequest, _ result: Result<[Staged], Error>) {
        guard pending?.id == request.id else { return }
        pending = nil
        options = nil
        importing = nil
        switch result {
        case .success(let files): request.resolveJSON(files)
        case .failure: request.fail(code: "io", message: "Could not import the selected media")
        }
    }

    public func detached() {
        pending?.fail(code: "cancelled", message: "The media picker window closed")
        pending = nil
        options = nil
        importing?.cancel()
        importing = nil
        #if os(iOS)
        presented?.dismiss(animated: false)
        presented = nil
        #else
        panel?.cancel(nil)
        panel = nil
        #endif
    }
    public func uninstalled() { detached() }

    nonisolated private static func stage(_ files: [URL], _ options: Selection) async throws -> [Staged] {
        var result: [Staged] = []
        for file in files { result.append(try await metadata(copy(file, options: options))) }
        return result
    }

    nonisolated private static func copy(_ url: URL, options: Selection, suggestedName: String? = nil, type declared: UTType? = nil) throws -> (URL, String, UTType) {
        let access = url.startAccessingSecurityScopedResource()
        defer { if access { url.stopAccessingSecurityScopedResource() } }
        let values = try url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .contentTypeKey])
        let type = declared ?? values.contentType ?? UTType(filenameExtension: url.pathExtension)
        guard values.isRegularFile == true, values.isSymbolicLink != true,
              let type, type.conforms(to: .image) || type.conforms(to: .movie) else { throw CocoaError(.fileReadUnsupportedScheme) }
        let root = URL(fileURLWithPath: options.staging, isDirectory: true)
        let originalName = suggestedName ?? url.lastPathComponent
        if type.conforms(to: .heic), let process = options.process, process.requiresDecode {
            let target = root.appendingPathComponent(UUID().uuidString).appendingPathExtension("jpg")
            guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
                  let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
                  let width = properties[kCGImagePropertyPixelWidth] as? Int,
                  let height = properties[kCGImagePropertyPixelHeight] as? Int,
                  width > 0, height > 0, Double(width) * Double(height) <= 40_000_000 else { throw CocoaError(.fileReadCorruptFile) }
            let settings: [CFString: Any] = [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: process.fixOrientation ?? true,
                kCGImageSourceThumbnailMaxPixelSize: max(width, height),
            ]
            guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, settings as CFDictionary),
                  let encoder = CGImageDestinationCreateWithURL(target as CFURL, UTType.jpeg.identifier as CFString, 1, nil)
            else { throw CocoaError(.fileReadCorruptFile) }
            var metadata = process.stripMetadata == false ? properties : [:]
            metadata[kCGImageDestinationLossyCompressionQuality] = 1.0
            if process.fixOrientation != false { metadata[kCGImagePropertyOrientation] = 1 }
            CGImageDestinationAddImage(encoder, image, metadata as CFDictionary)
            guard CGImageDestinationFinalize(encoder) else { throw CocoaError(.fileWriteUnknown) }
            return (target, originalName, .jpeg)
        }
        let target = root.appendingPathComponent(UUID().uuidString).appendingPathExtension(type.preferredFilenameExtension ?? "bin")
        try FileManager.default.copyItem(at: url, to: target)
        return (target, originalName, type)
    }

    nonisolated private static func metadata(_ item: (URL, String, UTType)) async throws -> Staged {
        let (url, name, type) = item
        var width: Int?, height: Int?, duration: Double?
        if type.conforms(to: .image) {
            if let image = CGImageSourceCreateWithURL(url as CFURL, nil),
               let metadata = CGImageSourceCopyPropertiesAtIndex(image, 0, nil) as? [CFString: Any] {
                width = metadata[kCGImagePropertyPixelWidth] as? Int
                height = metadata[kCGImagePropertyPixelHeight] as? Int
            }
        } else {
            let asset = AVURLAsset(url: url)
            if let track = try await asset.loadTracks(withMediaType: .video).first {
                let size = try await track.load(.naturalSize)
                let transform = try await track.load(.preferredTransform)
                let raw = size.applying(transform)
                if raw.width.isFinite, raw.height.isFinite { width = Int(abs(raw.width)); height = Int(abs(raw.height)) }
            }
            let seconds = try await asset.load(.duration).seconds
            if seconds.isFinite, seconds >= 0 { duration = seconds * 1000 }
        }
        return Staged(kind: type.conforms(to: .image) ? "image" : "video", path: url.path, fileName: name,
                      mimeType: type.preferredMIMEType ?? "application/octet-stream", width: width, height: height, durationMs: duration)
    }
}

public protocol ImpastoImagePickerFeature {}

#if os(iOS)
extension ImpastoImagePickerPlugin: @preconcurrency PHPickerViewControllerDelegate, @preconcurrency UIImagePickerControllerDelegate, @preconcurrency UINavigationControllerDelegate {
    public func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: true)
        guard let request = pending, let options else { return }
        importing = Task { @MainActor [weak self] in
            var files: [Staged] = []
            do {
                for result in results {
                    try Task.checkCancellation()
                    let provider = result.itemProvider
                    let primary = provider.registeredTypeIdentifiers.compactMap { UTType($0) }.first {
                        $0.conforms(to: .image) || $0.conforms(to: .movie)
                    }
                    guard let primary else { throw CocoaError(.fileReadUnsupportedScheme) }
                    let name = provider.suggestedName
                    let item: (URL, String, UTType) = try await withCheckedThrowingContinuation { continuation in
                        _ = provider.loadFileRepresentation(forTypeIdentifier: primary.identifier) { url, error in
                            continuation.resume(with: Result {
                                guard let url else { throw error ?? CocoaError(.fileReadUnknown) }
                                return try Self.copy(url, options: options, suggestedName: name, type: primary)
                            })
                        }
                    }
                    files.append(try await Self.metadata(item))
                }
                self?.finish(request, .success(files))
            } catch { self?.finish(request, .failure(error)) }
        }
    }

    fileprivate func capture(_ request: ImpastoNativeRequest, _ options: Selection, video: Bool) {
        guard cameraSupport(request, checkOnly: false) else { return }
        guard let controller = host?.viewController, controller.presentedViewController == nil, controller.viewIfLoaded?.window != nil else {
            request.fail(code: "host-unavailable", message: "No visible window for the media picker"); return
        }
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        let device: UIImagePickerController.CameraDevice = options.preferredCamera == "front" ? .front : .rear
        if UIImagePickerController.isCameraDeviceAvailable(device) { picker.cameraDevice = device }
        picker.mediaTypes = [video ? UTType.movie.identifier : UTType.image.identifier]
        if video {
            picker.videoQuality = options.quality == "high" ? .typeHigh : options.quality == "low" ? .typeLow : .typeMedium
            if let duration = options.maxDurationSec, duration > 0 { picker.videoMaximumDuration = duration }
        }
        picker.delegate = self
        self.options = options
        pending = request
        presented = picker
        controller.present(picker, animated: true)
    }

    public func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
        picker.dismiss(animated: true)
        if let request = pending { finish(request, .success([])) }
    }

    public func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
        picker.dismiss(animated: true)
        guard let request = pending, let options else { return }
        let url = info[.mediaURL] as? URL
        let photo = info[.originalImage] as? UIImage
        importing = Task { @MainActor [weak self] in
            do {
                if let url {
                    self?.finish(request, .success(try await Self.stage([url], options)))
                } else if let source = photo?.jpegData(compressionQuality: 1.0) {
                    let path = URL(fileURLWithPath: options.staging).appendingPathComponent(UUID().uuidString + ".jpg")
                    try await Task.detached { try source.write(to: path, options: .atomic) }.value
                    self?.finish(request, .success([try await Self.metadata((path, "Photo.jpg", .jpeg))]))
                } else { self?.finish(request, .failure(CocoaError(.fileReadUnknown))) }
            } catch { self?.finish(request, .failure(error)) }
        }
    }
}
#endif
