import ImpastoHost
import UIKit

@MainActor
public final class ImpastoLinkingPlugin: ImpastoPlugin {
    public let identifier = "@impasto/linking"
    private weak var controller: UIViewController?
    private weak var sheet: UIActivityViewController?

    public init() {}

    public static func register(with host: ImpastoHost) {
        host.registerPlugin(Self())
    }

    public func attached(to controller: ImpastoViewController) {
        self.controller = controller
    }

    public func detached() {
        sheet?.dismiss(animated: false)
        sheet = nil
        controller = nil
    }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        switch request.operation {
        case "@impasto/linking/open":
            guard let url = URL(string: request.payload),
                  let scheme = url.scheme?.lowercased(),
                  !["file", "javascript", "data"].contains(scheme),
                  (scheme != "http" && scheme != "https") || url.host != nil
            else {
                request.fail("Invalid external URL")
                return true
            }
            Task { @MainActor in
                guard request.isPending else { return }
                if await UIApplication.shared.open(url) { request.resolve() }
                else { request.fail("iOS could not open the URL") }
            }
        case "@impasto/linking/share":
            guard let controller, controller.presentedViewController == nil,
                  controller.viewIfLoaded?.window?.windowScene?.activationState == .foregroundActive
            else {
                request.fail("No active iOS presentation scene for sharing")
                return true
            }
            let sheet = UIActivityViewController(activityItems: [request.payload], applicationActivities: nil)
            if let popover = sheet.popoverPresentationController {
                popover.sourceView = controller.view
                popover.sourceRect = CGRect(x: controller.view.bounds.midX, y: controller.view.bounds.midY, width: 1, height: 1)
                popover.permittedArrowDirections = []
            }
            sheet.completionWithItemsHandler = { _, _, _, error in
                let message = error?.localizedDescription
                Task { @MainActor in
                    if let message { request.fail(message) }
                    else { request.resolve() }
                }
            }
            self.sheet = sheet
            controller.present(sheet, animated: true)
        default:
            return false
        }
        return true
    }
}
