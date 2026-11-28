import Foundation
import ImpastoHost
import Network

private struct State: Codable, Equatable {
    let types: [String]
    let isConnected: Bool
    let isExpensive: Bool
    let isConstrained: Bool

    init(_ path: NWPath) {
        var types: [String] = []
        isConnected = path.status == .satisfied
        if isConnected {
            for (type, name) in [(NWInterface.InterfaceType.wifi, "wifi"), (.cellular, "cellular"), (.wiredEthernet, "ethernet")] {
                if path.usesInterfaceType(type) { types.append(name) }
            }
            if path.usesInterfaceType(.other) {
                let tunnel = path.availableInterfaces.contains { $0.type == .other && ["utun", "ipsec", "ppp"].contains(where: $0.name.hasPrefix) }
                types.append(tunnel ? "vpn" : "other")
            }
        }
        self.types = types.isEmpty ? [isConnected ? "other" : "none"] : types
        isExpensive = isConnected && path.isExpensive
        isConstrained = isConnected && path.isConstrained
    }
}

@MainActor
public final class ImpastoConnectivityPlugin: ImpastoPlugin {
    private struct Control: Decodable { let topic: String }
    public let identifier = "@impasto/connectivity"
    private var topic: String { "\(identifier)/status" }
    private weak var host: ImpastoHost?
    private var monitor: NWPathMonitor?
    private var token = UUID()
    private var listening = false
    private var recent: State?
    private var requests: [ImpastoNativeRequest] = []
    private let queue = DispatchQueue(label: "dev.impasto.connectivity")

    public static func register(with host: ImpastoHost) { host.registerPlugin(Self()) }
    public func installed(in host: ImpastoHost) { self.host = host }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        switch request.operation {
        case topic:
            if let recent { request.resolveJSON(recent) }
            else {
                requests.removeAll { !$0.isPending }
                requests.append(request)
                start()
            }
        case "@impasto/events/start" where matches(request):
            listening = true
            if let recent { emit(recent) }
            start()
            request.resolve()
        case "@impasto/events/stop" where matches(request):
            listening = false
            stopIfIdle()
            request.resolve()
        default: return false
        }
        return true
    }

    private func matches(_ request: ImpastoNativeRequest) -> Bool {
        (try? request.decodeJSON(Control.self).topic) == topic
    }

    private func start() {
        guard monitor == nil else { return }
        let monitor = NWPathMonitor()
        let token = UUID()
        self.token = token
        self.monitor = monitor
        monitor.pathUpdateHandler = { [weak self] path in
            let state = State(path)
            Task { @MainActor in
                guard let self, self.token == token else { return }
                self.update(state)
            }
        }
        monitor.start(queue: queue)
    }

    private func update(_ state: State) {
        let prior = recent
        recent = state
        let waiting = requests
        requests.removeAll()
        for request in waiting { request.resolveJSON(state) }
        if listening && prior != state { emit(state) }
        stopIfIdle()
    }

    private func emit(_ state: State) {
        guard let bytes = try? JSONEncoder().encode(state) else { return }
        host?.emitPluginEvent(topic: topic, payload: String(decoding: bytes, as: UTF8.self))
    }

    private func stopIfIdle() {
        guard !listening && requests.isEmpty else { return }
        monitor?.cancel()
        monitor = nil
        recent = nil
        token = UUID()
    }

    public func detached() {
        for request in requests { request.fail(code: "cancelled", message: "The network monitor detached") }
        requests.removeAll()
        listening = false
        stopIfIdle()
    }

    public func uninstalled() { detached() }
}
