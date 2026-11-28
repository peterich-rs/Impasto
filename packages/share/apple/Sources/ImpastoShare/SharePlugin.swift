import Foundation
import ImpastoHost
#if os(iOS)
import LinkPresentation
import UIKit
#else
import AppKit
#endif

private struct ShareOptions: Decodable {
    struct Attachment: Decodable { let path: String }
    struct Anchor: Decodable { let x: CGFloat; let y: CGFloat; let width: CGFloat; let height: CGFloat }
    let text: String?
    let url: URL?
    let title: String?
    let subject: String?
    let files: [Attachment]
    let anchor: Anchor?

    var items: [Any] {
        var items: [Any] = []
        if let text, !text.isEmpty { items.append(text) }
        if let url { items.append(url) }
        items.append(contentsOf: files.map { URL(fileURLWithPath: $0.path) })
        return items
    }

    func rectangle(in bounds: CGRect, flipped: Bool) -> CGRect {
        let middle = CGRect(x: bounds.midX, y: bounds.midY, width: 1, height: 1)
        guard let anchor else { return middle }
        let y = flipped ? bounds.minY + anchor.y : bounds.maxY - anchor.y - anchor.height
        let requested = CGRect(x: bounds.minX + anchor.x, y: y, width: anchor.width, height: anchor.height)
        let visible = requested.intersection(bounds)
        return visible.isEmpty ? middle : visible
    }
}

@MainActor
public final class ImpastoSharePlugin: NSObject, ImpastoPlugin {
    public let identifier = "@impasto/share"
    private weak var host: ImpastoHost?
    private var pending: ImpastoNativeRequest?
    #if os(iOS)
    private weak var sheet: UIActivityViewController?
    #else
    private var picker: NSSharingServicePicker?
    private var service: NSSharingService?
    #endif

    public static func register(with host: ImpastoHost) { host.registerPlugin(Self()) }
    public func installed(in host: ImpastoHost) { self.host = host }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        guard request.operation == "\(identifier)/share" else { return false }
        guard pending == nil else {
            request.fail(code: "busy", message: "A share sheet is already open")
            return true
        }
        do {
            let options = try request.decodeJSON(ShareOptions.self)
            #if os(iOS)
            guard let controller = host?.viewController, controller.presentedViewController == nil,
                  controller.viewIfLoaded?.window?.windowScene?.activationState == .foregroundActive else {
                request.fail(code: "host-unavailable", message: "No visible window for sharing")
                return true
            }
            var items = options.items
            if let first = items.first, options.title != nil || options.subject != nil {
                items[0] = ActivityItem(item: first, title: options.title, subject: options.subject)
            }
            let sheet = UIActivityViewController(activityItems: items, applicationActivities: nil)
            if let popover = sheet.popoverPresentationController {
                popover.sourceView = controller.view
                popover.sourceRect = options.rectangle(in: controller.view.bounds, flipped: true)
                if options.anchor == nil { popover.permittedArrowDirections = [] }
            }
            pending = request
            self.sheet = sheet
            sheet.completionWithItemsHandler = { [weak self] activity, completed, _, error in
                Task { @MainActor in
                    guard let self, self.pending?.id == request.id else { return }
                    if let error { self.fail(error) }
                    else { self.finish(completed ? "success" : "dismissed", activity: activity?.rawValue) }
                }
            }
            controller.present(sheet, animated: true)
            #else
            guard let view = host?.window?.contentView else {
                request.fail(code: "host-unavailable", message: "No visible window for sharing")
                return true
            }
            let picker = NSSharingServicePicker(items: options.items)
            picker.delegate = self
            self.picker = picker
            pending = request
            subject = options.subject
            picker.show(relativeTo: options.rectangle(in: view.bounds, flipped: view.isFlipped),
                        of: view, preferredEdge: .minY)
            #endif
        } catch { request.fail(code: "platform", message: error.localizedDescription) }
        return true
    }

    #if os(macOS)
    private var subject: String?

    private func take() -> ImpastoNativeRequest? {
        let request = pending
        pending = nil
        picker?.delegate = nil
        service?.delegate = nil
        picker = nil
        service = nil
        subject = nil
        return request
    }
    #else
    private func take() -> ImpastoNativeRequest? {
        let request = pending
        pending = nil
        sheet = nil
        return request
    }
    #endif

    fileprivate func finish(_ status: String, activity: String? = nil) {
        var result = ["status": status]
        if let activity { result["activity"] = activity }
        take()?.resolveJSON(result)
    }

    fileprivate func fail(_ error: Error) {
        take()?.fail(code: "platform", message: error.localizedDescription)
    }

    public func detached() {
        #if os(iOS)
        sheet?.dismiss(animated: false)
        #endif
        take()?.fail(code: "host-unavailable", message: "The share sheet detached")
    }

    public func uninstalled() { detached() }
}

#if os(iOS)
@MainActor
private final class ActivityItem: NSObject, @preconcurrency UIActivityItemSource {
    let item: Any
    let title: String?
    let subject: String?

    init(item: Any, title: String?, subject: String?) {
        self.item = item
        self.title = title
        self.subject = subject
    }

    func activityViewControllerPlaceholderItem(_ controller: UIActivityViewController) -> Any { item }
    func activityViewController(_ controller: UIActivityViewController, itemForActivityType activityType: UIActivity.ActivityType?) -> Any? { item }
    func activityViewController(_ controller: UIActivityViewController, subjectForActivityType activityType: UIActivity.ActivityType?) -> String {
        subject ?? title ?? ""
    }
    func activityViewControllerLinkMetadata(_ controller: UIActivityViewController) -> LPLinkMetadata? {
        guard let title else { return nil }
        let metadata = LPLinkMetadata()
        metadata.title = title
        metadata.originalURL = item as? URL
        metadata.url = metadata.originalURL
        return metadata
    }
}
#else
extension ImpastoSharePlugin: @preconcurrency NSSharingServicePickerDelegate, NSSharingServiceDelegate {
    public func sharingServicePicker(_ sharingServicePicker: NSSharingServicePicker, sharingServicesForItems items: [Any],
                                     proposedSharingServices services: [NSSharingService]) -> [NSSharingService] {
        if services.isEmpty {
            Task { @MainActor [weak self, weak sharingServicePicker] in
                guard let self, let sharingServicePicker, sharingServicePicker === self.picker else { return }
                self.finish("unavailable")
            }
        }
        return services
    }

    public func sharingServicePicker(_ sharingServicePicker: NSSharingServicePicker,
                                     didChoose service: NSSharingService?) {
        guard sharingServicePicker === picker else { return }
        guard let service else { finish("dismissed"); return }
        self.service = service
        service.subject = subject
    }

    public func sharingServicePicker(_ sharingServicePicker: NSSharingServicePicker,
                                     delegateFor sharingService: NSSharingService) -> (any NSSharingServiceDelegate)? {
        self
    }

    public func sharingService(_ sharingService: NSSharingService, didShareItems items: [Any]) {
        if sharingService === service { finish("success", activity: sharingService.title) }
    }

    public func sharingService(_ sharingService: NSSharingService, didFailToShareItems items: [Any], error: Error) {
        guard sharingService === service else { return }
        if (error as NSError).code == NSUserCancelledError { finish("dismissed") }
        else { fail(error) }
    }
}
#endif
