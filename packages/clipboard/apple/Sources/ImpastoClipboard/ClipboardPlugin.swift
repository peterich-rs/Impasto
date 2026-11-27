import Foundation
import ImpastoHost
import UniformTypeIdentifiers
#if os(iOS)
import UIKit
#else
import AppKit
#endif

private struct Content: Codable {
    var text: String?
    var html: String?
    var uri: String?
    var imagePng: String?
}

private struct ImageError: LocalizedError {
    var errorDescription: String? { "Clipboard images must be PNG and fit within 32 MiB" }
}

@MainActor
public final class ImpastoClipboardPlugin: ImpastoPlugin {
    private struct Control: Decodable { let topic: String }
    public let identifier = "@impasto/clipboard"
    private var topic: String { "\(identifier)/change" }
    private weak var host: ImpastoHost?
    private var lastCount: Int?
    #if os(iOS)
    private var observer: (any NSObjectProtocol)?
    #else
    private var timer: Timer?
    #endif
    private static let imageLimit = 32 * 1024 * 1024

    public static func register(with host: ImpastoHost) { host.registerPlugin(Self()) }
    public func installed(in host: ImpastoHost) { self.host = host }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        do {
            switch request.operation {
            case "\(identifier)/availableFormats": request.resolveJSON(formats())
            case "\(identifier)/hasStrings": request.resolveJSON(formats().contains("text"))
            case "\(identifier)/hasImages": request.resolveJSON(formats().contains("image"))
            case "\(identifier)/hasHtml": request.resolveJSON(formats().contains("html"))
            case "\(identifier)/read": request.resolveJSON(try read())
            case "\(identifier)/write": try write(request.decodeJSON(Content.self)); request.resolve()
            case "\(identifier)/clear": clear(); request.resolve()
            case "@impasto/events/start" where matches(request): start(); request.resolve()
            case "@impasto/events/stop" where matches(request): detached(); request.resolve()
            default: return false
            }
        } catch {
            request.fail(code: error is ImageError ? "invalid-image" : "platform", message: error.localizedDescription)
        }
        return true
    }

    private func matches(_ request: ImpastoNativeRequest) -> Bool {
        (try? request.decodeJSON(Control.self).topic) == topic
    }

    private func formats() -> [String] {
        var result: [String] = []
        #if os(iOS)
        let board = UIPasteboard.general
        if board.hasStrings { result.append("text") }
        if board.contains(pasteboardTypes: [UTType.html.identifier]) { result.append("html") }
        if board.hasURLs { result.append("uri") }
        if board.hasImages { result.append("image") }
        #else
        let types = NSPasteboard.general.types ?? []
        if types.contains(.string) { result.append("text") }
        if types.contains(.html) { result.append("html") }
        if types.contains(.URL) || types.contains(.fileURL) { result.append("uri") }
        if types.contains(.png) || types.contains(.tiff) { result.append("image") }
        #endif
        return result
    }

    private func read() throws -> Content {
        #if os(iOS)
        let board = UIPasteboard.general
        let rawHtml = board.value(forPasteboardType: UTType.html.identifier)
        let html = (rawHtml as? String) ?? (rawHtml as? Data).flatMap { String(data: $0, encoding: .utf8) }
        let image = board.image
        if let image, image.size.width * image.size.height * image.scale * image.scale * 4 > CGFloat(Self.imageLimit) {
            throw ImageError()
        }
        let png = image?.pngData()
        let text = board.string
        let uri = board.url?.absoluteString
        #else
        let board = NSPasteboard.general
        let text = board.string(forType: .string)
        let html = board.string(forType: .html)
        let uri = board.string(forType: .URL) ?? board.string(forType: .fileURL)
        var png = board.data(forType: .png)
        if png == nil, let raw = board.data(forType: .tiff), raw.count <= Self.imageLimit,
           let bitmap = NSBitmapImageRep(data: raw) {
            guard Double(bitmap.pixelsWide) * Double(bitmap.pixelsHigh) * 4 <= Double(Self.imageLimit) else {
                throw ImageError()
            }
            png = bitmap.representation(using: .png, properties: [:])
        }
        #endif
        if let png, png.count > Self.imageLimit { throw ImageError() }
        return Content(text: text, html: html, uri: uri, imagePng: png?.base64EncodedString())
    }

    private func write(_ content: Content) throws {
        let png = try content.imagePng.map { value -> Data in
            guard value.utf8.count <= (Self.imageLimit + 2) / 3 * 4,
                  let data = Data(base64Encoded: value), data.count <= Self.imageLimit else { throw ImageError() }
            return data
        }
        #if os(iOS)
        var item: [String: Any] = [:]
        if let text = content.text { item[UTType.utf8PlainText.identifier] = text }
        if let html = content.html { item[UTType.html.identifier] = Data(html.utf8) }
        if let string = content.uri {
            guard let url = URL(string: string), url.scheme != nil else { throw CocoaError(.fileWriteInvalidFileName) }
            item[UTType.url.identifier] = url
        }
        if let png { item[UTType.png.identifier] = png }
        UIPasteboard.general.items = [item]
        #else
        let item = NSPasteboardItem()
        if let text = content.text { item.setString(text, forType: .string) }
        if let html = content.html { item.setString(html, forType: .html) }
        if let uri = content.uri { item.setString(uri, forType: .URL) }
        if let png { item.setData(png, forType: .png) }
        NSPasteboard.general.clearContents()
        guard NSPasteboard.general.writeObjects([item]) else { throw CocoaError(.fileWriteUnknown) }
        #endif
    }

    private func clear() {
        #if os(iOS)
        UIPasteboard.general.items = []
        #else
        NSPasteboard.general.clearContents()
        #endif
    }

    private var changeCount: Int {
        #if os(iOS)
        UIPasteboard.general.changeCount
        #else
        NSPasteboard.general.changeCount
        #endif
    }

    private func start() {
        guard lastCount == nil else { return }
        lastCount = changeCount
        #if os(iOS)
        observer = NotificationCenter.default.addObserver(forName: UIPasteboard.changedNotification, object: UIPasteboard.general, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.update() }
        }
        #else
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.update() }
        }
        #endif
    }

    private func update() {
        guard let last = lastCount, last != changeCount else { return }
        lastCount = changeCount
        guard let payload = try? JSONEncoder().encode(formats()) else { return }
        host?.emitPluginEvent(topic: topic, payload: String(decoding: payload, as: UTF8.self))
    }

    #if os(iOS)
    public func sceneActivityChanged(_ activity: ImpastoSceneActivity) {
        if case .foreground = activity { update() }
    }
    #endif

    public func detached() {
        lastCount = nil
        #if os(iOS)
        if let observer { NotificationCenter.default.removeObserver(observer) }
        observer = nil
        #else
        timer?.invalidate()
        timer = nil
        #endif
    }

    public func uninstalled() { detached() }
}
