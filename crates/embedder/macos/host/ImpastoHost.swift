import CImpastoMacOS
import AppKit

/// Installs AppKit plugins before starting the Rust application host.
@MainActor
public final class ImpastoHost {
    private static weak var current: ImpastoHost?
    private var plugins: [any ImpastoPlugin] = []

    public init() {}

    public var window: NSWindow? {
        guard Self.current === self else { return nil }
        return NSApplication.shared.mainWindow ?? NSApplication.shared.keyWindow
    }

    public func registerPlugin(_ plugin: any ImpastoPlugin) {
        plugins.append(plugin)
        plugin.installed(in: self)
    }

    public func run() -> Int32 {
        precondition(Self.current == nil)
        Self.current = self
        defer { Self.current = nil }
        return Int32(impasto_macos_run { text in
            guard let text, let url = URL(string: String(cString: text)) else { return false }
            return MainActor.assumeIsolated {
                ImpastoHost.current?.plugins.contains { $0.open(url) } ?? false
            }
        })
    }
}

@MainActor
public protocol ImpastoPlugin: AnyObject {
    func installed(in host: ImpastoHost)
    func open(_ url: URL) -> Bool
}

public extension ImpastoPlugin {
    func installed(in host: ImpastoHost) {}
    func open(_ url: URL) -> Bool { false }
}
