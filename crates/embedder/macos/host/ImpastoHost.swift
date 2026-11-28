import CImpastoMacOS
import AppKit

/// Installs AppKit plugins before starting the Rust application host.
@MainActor
public final class ImpastoHost {
    private static weak var current: ImpastoHost?
    private var plugins: [any ImpastoPlugin] = []
    private var identifiers: Set<String> = []
    private var running = false

    public init() {}

    public var window: NSWindow? {
        guard Self.current === self else { return nil }
        let app = NSApplication.shared
        return app.mainWindow ?? app.keyWindow ?? app.windows.first(where: { $0.isVisible })
    }

    public func registerPlugin(_ plugin: any ImpastoPlugin) {
        guard identifiers.insert(plugin.identifier).inserted else { return }
        plugins.append(plugin)
        plugin.installed(in: self)
    }

    @discardableResult
    public func emitPluginEvent(topic: String, payload: String) -> Bool {
        guard running else { return false }
        return topic.withCString { topic in
            payload.withCString { impasto_macos_service_event(topic, $0) }
        }
    }

    public func run() -> Int32 {
        precondition(Self.current == nil)
        Self.current = self
        running = true
        defer {
            detach()
            Self.current = nil
        }
        var callbacks = ImpastoMacOSCallbacks(
            context: Unmanaged.passUnretained(self).toOpaque(),
            open_url: { context, text in
                guard let context, let text, let url = URL(string: String(cString: text)) else {
                    return false
                }
                return MainActor.assumeIsolated {
                    let host = Unmanaged<ImpastoHost>.fromOpaque(context).takeUnretainedValue()
                    return host.plugins.contains { $0.open(url) }
                }
            },
            service_request: { context, id, operation, payload in
                guard let context, let operation, let payload else { return }
                MainActor.assumeIsolated {
                    let host = Unmanaged<ImpastoHost>.fromOpaque(context).takeUnretainedValue()
                    host.handle(id: id, operation: String(cString: operation), payload: String(cString: payload))
                }
            },
            closed: { context in
                guard let context else { return }
                MainActor.assumeIsolated {
                    Unmanaged<ImpastoHost>.fromOpaque(context).takeUnretainedValue().detach()
                }
            })
        return Int32(impasto_macos_run_host(&callbacks))
    }

    private func handle(id: UInt64, operation: String, payload: String) {
        guard running, impasto_macos_service_pending(id) else { return }
        let request = ImpastoNativeRequest(
            id: id, operation: operation, payload: payload,
            pending: { impasto_macos_service_pending(id) },
            finish: { status, value in
                if let value {
                    return value.withCString { impasto_macos_service_result(id, status, $0) }
                }
                return impasto_macos_service_result(id, status, nil)
            })
        if !plugins.contains(where: { $0.handle(request) }) {
            request.fail("No platform plugin handles \(operation)")
        }
    }

    private func detach() {
        guard running else { return }
        running = false
        for plugin in plugins.reversed() { plugin.detached() }
    }
}

@MainActor
public protocol ImpastoPlugin: AnyObject {
    var identifier: String { get }
    func installed(in host: ImpastoHost)
    func handle(_ request: ImpastoNativeRequest) -> Bool
    func open(_ url: URL) -> Bool
    func detached()
}

public extension ImpastoPlugin {
    var identifier: String { String(reflecting: type(of: self)) }
    func installed(in host: ImpastoHost) {}
    func handle(_ request: ImpastoNativeRequest) -> Bool { false }
    func open(_ url: URL) -> Bool { false }
    func detached() {}
}

/// A one-shot call from a Rust module to an installed platform plugin.
@MainActor
public struct ImpastoNativeRequest {
    public let id: UInt64
    public let operation: String
    public let payload: String
    private let pending: () -> Bool
    private let finish: (UInt32, String?) -> Bool

    init(id: UInt64, operation: String, payload: String,
         pending: @escaping () -> Bool, finish: @escaping (UInt32, String?) -> Bool) {
        self.id = id
        self.operation = operation
        self.payload = payload
        self.pending = pending
        self.finish = finish
    }

    public var isPending: Bool { pending() }

    @discardableResult
    public func resolve(_ payload: String? = nil) -> Bool { finish(0, payload) }

    @discardableResult
    public func fail(_ message: String) -> Bool { finish(2, message) }

    public func decodeJSON<T: Decodable>(_ type: T.Type) throws -> T {
        try JSONDecoder().decode(type, from: Data(payload.utf8))
    }

    @discardableResult
    public func resolveJSON(_ value: some Encodable) -> Bool {
        do {
            return resolve(String(decoding: try JSONEncoder().encode(value), as: UTF8.self))
        } catch {
            return fail(code: "platform", message: error.localizedDescription)
        }
    }

    @discardableResult
    public func fail(code: String, message: String) -> Bool {
        let payload = try? JSONEncoder().encode(["code": code, "message": message])
        return fail(payload.map { String(decoding: $0, as: UTF8.self) } ?? message)
    }

}
