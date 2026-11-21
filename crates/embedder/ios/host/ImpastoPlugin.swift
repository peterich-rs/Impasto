import UIKit

public enum ImpastoSceneActivity: Sendable {
    case detached
    case foreground
    case inactive
    case hidden
    case background
}

/// A method invocation and its UTF-8 JSON payload from the current script session.
public struct ImpastoPluginCall: Sendable {
    public let method: String
    public let payload: Data
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
    func open(_ url: URL) -> Bool
    func handleMemoryWarning()

    /// Runs on the main actor. Return UTF-8 JSON or throw to reject the script promise.
    /// A scene change, teardown or timeout cancels the task. Observe task cancellation
    /// before performing asynchronous side effects.
    func handle(_ call: ImpastoPluginCall) async throws -> Data
    func uninstalled()
}

public extension ImpastoPlugin {
    func installed(in host: ImpastoHost) {}
    func attached(to controller: ImpastoViewController) {}
    func detached() {}
    func sceneActivityChanged(_ activity: ImpastoSceneActivity) {}
    func open(_ url: URL) -> Bool { false }
    func handleMemoryWarning() {}
    func handle(_ call: ImpastoPluginCall) async throws -> Data {
        throw UnimplementedPluginMethod(method: call.method)
    }
    func uninstalled() {}
}

private struct UnimplementedPluginMethod: LocalizedError {
    let method: String
    var errorDescription: String? { "Native plugin does not provide method '\(method)'" }
}
