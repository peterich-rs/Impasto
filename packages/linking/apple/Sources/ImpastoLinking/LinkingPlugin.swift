import Foundation
import ImpastoHost
#if os(iOS)
import SafariServices
import UIKit
#else
import AppKit
#endif

@MainActor
public final class ImpastoLinkingPlugin: NSObject, ImpastoPlugin {
    public let identifier = "@impasto/linking"
    private weak var host: ImpastoHost?
    #if os(iOS)
    private var pending: ImpastoNativeRequest?
    private weak var browser: SFSafariViewController?
    #endif

    public static func register(with host: ImpastoHost) { host.registerPlugin(Self()) }
    public func installed(in host: ImpastoHost) { self.host = host }

    private struct Open: Decodable {
        struct Options: Decodable {
            let mode: String?
            let toolbarColor: String?
            let dismissButtonStyle: String?
        }
        let url: URL
        let options: Options
    }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        do {
            switch request.operation {
            case "\(identifier)/canOpen":
                let url = try request.decodeJSON(URL.self)
                #if os(iOS)
                let allowed = UIApplication.shared.canOpenURL(url)
                let scheme = url.scheme?.lowercased() ?? ""
                if !allowed && !["http", "https", "mailto", "tel"].contains(scheme) {
                    let schemes = Bundle.main.object(forInfoDictionaryKey: "LSApplicationQueriesSchemes") as? [String] ?? []
                    if !schemes.contains(where: { $0.caseInsensitiveCompare(scheme) == .orderedSame }) {
                        NSLog("Impasto Linking: add %@ to LSApplicationQueriesSchemes to query this scheme", scheme)
                    }
                }
                request.resolveJSON(allowed)
                #else
                request.resolveJSON(NSWorkspace.shared.urlForApplication(toOpen: url) != nil)
                #endif
            case "\(identifier)/open":
                let options = try request.decodeJSON(Open.self)
                #if os(iOS)
                if options.options.mode == "inAppBrowser" {
                    present(options, request: request)
                } else {
                    UIApplication.shared.open(options.url) { opened in
                        Task { @MainActor in request.resolveJSON(opened) }
                    }
                }
                #else
                request.resolveJSON(NSWorkspace.shared.open(options.url))
                #endif
            default: return false
            }
        } catch {
            request.fail(code: "platform", message: error.localizedDescription)
        }
        return true
    }

    #if os(iOS)
    private func present(_ options: Open, request: ImpastoNativeRequest) {
        guard pending == nil, let controller = host?.viewController,
              controller.presentedViewController == nil,
              controller.viewIfLoaded?.window?.windowScene?.activationState == .foregroundActive else {
            request.fail(code: "host-unavailable", message: "No foreground window for the in-app browser")
            return
        }
        let browser = SFSafariViewController(url: options.url)
        browser.delegate = self
        if let color = options.options.toolbarColor, let rgb = UInt32(color.dropFirst(), radix: 16) {
            browser.preferredBarTintColor = UIColor(red: CGFloat(rgb >> 16) / 255,
                green: CGFloat((rgb >> 8) & 255) / 255, blue: CGFloat(rgb & 255) / 255, alpha: 1)
        }
        switch options.options.dismissButtonStyle {
        case "close": browser.dismissButtonStyle = .close
        case "cancel": browser.dismissButtonStyle = .cancel
        default: browser.dismissButtonStyle = .done
        }
        pending = request
        self.browser = browser
        controller.present(browser, animated: true)
    }
    #endif

    public func detached() {
        #if os(iOS)
        let request = pending
        pending = nil
        browser?.dismiss(animated: false)
        browser = nil
        request?.fail(code: "host-unavailable", message: "The in-app browser detached")
        #endif
    }

    public func uninstalled() { detached() }
}

#if os(iOS)
extension ImpastoLinkingPlugin: @preconcurrency SFSafariViewControllerDelegate {
    public func safariViewControllerDidFinish(_ controller: SFSafariViewController) {
        guard controller === browser else { return }
        let request = pending
        pending = nil
        browser = nil
        request?.resolveJSON(true)
    }
}
#endif
