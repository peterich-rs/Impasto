import UIKit

public enum ImpastoSceneActivity: Sendable {
    case detached
    case foreground
    case inactive
    case hidden
    case background
}

/// Native plugins are installed per host. UIKit lifecycle callbacks run on
/// the main actor and receive only the controller owned by that host.
@MainActor
public protocol ImpastoPlugin: AnyObject {
    var identifier: String { get }
    func installed(in host: ImpastoHost)
    func attached(to controller: ImpastoViewController)
    func detached()
    func sceneActivityChanged(_ activity: ImpastoSceneActivity)
    func handle(_ request: ImpastoNativeRequest) -> Bool
    func open(_ url: URL) -> Bool
    func handleMemoryWarning()

    func uninstalled()
}

public extension ImpastoPlugin {
    func installed(in host: ImpastoHost) {}
    func attached(to controller: ImpastoViewController) {}
    func detached() {}
    func sceneActivityChanged(_ activity: ImpastoSceneActivity) {}
    func handle(_ request: ImpastoNativeRequest) -> Bool { false }
    func open(_ url: URL) -> Bool { false }
    func handleMemoryWarning() {}
    func uninstalled() {}
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
}
