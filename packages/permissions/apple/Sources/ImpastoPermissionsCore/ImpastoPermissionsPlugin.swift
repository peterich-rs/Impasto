import Foundation
import ImpastoHost
#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

@MainActor
public final class ImpastoPermissionsPlugin: ImpastoPlugin {
    public let identifier = "@impasto/permissions"
    private let owner = UUID()
    private let enabled: Set<String>
    private var strategies: [String: PermissionStrategy] = [:]
    private weak var host: ImpastoHost?
    private var work: [UInt64: Work] = [:]

    private struct Work {
        let token: UUID
        let request: ImpastoNativeRequest
        let task: Task<Void, Never>
    }

    private struct RequestBatch: Decodable {
        let names: [String]
        let notification: NotificationOptions?
    }

    private init(enabled: Set<String>, registrars: [any PermissionRegistrar.Type]) {
        self.enabled = enabled
        for registrar in registrars {
            strategies.merge(registrar.strategies()) { current, _ in current }
        }
    }

    public static func register(with host: ImpastoHost, enabled: Set<String>, registrars: [any PermissionRegistrar.Type]) {
        host.registerPlugin(Self(enabled: enabled, registrars: registrars))
    }

    public func installed(in host: ImpastoHost) { self.host = host }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        guard let action = request.operation.split(separator: "/").last,
              request.operation.hasPrefix("\(identifier)/") else { return false }
        switch action {
        case "status":
            run(request) { try await self.strategy(request.payload).status().rawValue }
        case "service":
            run(request) { try await self.strategy(request.payload).service().rawValue }
        case "rationale":
            run(request) {
                _ = try self.strategy(request.payload)
                return "false"
            }
        case "settings":
            run(request) { String(await Self.openSettings(request.payload)) }
        case "limitedPicker":
            prompt(request) {
                guard let picker = self.strategies["photos"]?.picker,
                      self.enabled.contains("photos") || self.enabled.contains("videos") else { return "false" }
                guard let host = self.host else { throw PermissionFailure("host-unavailable", "The permission host has detached") }
                return String(try await picker(host))
            }
        case "request":
            prompt(request) {
                let batch = try JSONDecoder().decode(RequestBatch.self, from: Data(request.payload.utf8))
                let strategies = try batch.names.map(self.strategy)
                if batch.notification?.criticalAlert == true { _ = try self.strategy("criticalAlerts") }
                var statuses: [String] = []
                for strategy in strategies {
                    try Task.checkCancellation()
                    guard request.isPending else { throw CancellationError() }
                    statuses.append(try await strategy.request(batch.notification).rawValue)
                }
                return String(decoding: try JSONEncoder().encode(statuses), as: UTF8.self)
            }
        default: return false
        }
        return true
    }

    public func detached() {
        PermissionPromptQueue.shared.cancel(owner)
        let outstanding = work
        work.removeAll()
        for item in outstanding.values {
            item.task.cancel()
            item.request.fail(PermissionFailure.encode(CancellationError()))
        }
    }

    #if os(iOS)
    public func uninstalled() { detached() }
    #endif

    private func run(_ request: ImpastoNativeRequest, operation: @escaping () async throws -> String) {
        let token = UUID()
        let task = Task { @MainActor in
            defer {
                if self.work[request.id]?.token == token {
                    self.work.removeValue(forKey: request.id)
                }
            }
            await Self.finish(request, operation)
        }
        work[request.id] = Work(token: token, request: request, task: task)
    }

    private func prompt(_ request: ImpastoNativeRequest, operation: @escaping () async throws -> String) {
        PermissionPromptQueue.shared.enqueue(owner, cancel: {
            request.fail(PermissionFailure.encode(CancellationError()))
        }) {
            await Self.finish(request, operation)
        }
    }

    private static func finish(_ request: ImpastoNativeRequest, _ operation: () async throws -> String) async {
        guard request.isPending else { return }
        do {
            try Task.checkCancellation()
            let payload = try await operation()
            try Task.checkCancellation()
            request.resolve(payload)
        } catch {
            request.fail(PermissionFailure.encode(error))
        }
    }

    private func strategy(_ name: String) throws -> PermissionStrategy {
        if Self.configurable.contains(name) {
            guard enabled.contains(name), let strategy = strategies[name] else {
                throw PermissionFailure("not-enabled", "Enable \(name) in [plugins.\"@impasto/permissions\"].enabled")
            }
            return strategy
        }
        return PermissionStrategy(status: { Self.coreStatus(name) })
    }

    private static let configurable: Set<String> = {
        let common: Set<String> = [
        "camera", "microphone", "speech", "photos", "photosAddOnly", "videos", "location", "locationWhenInUse",
        "locationAlways", "notification", "criticalAlerts", "contacts", "calendarWriteOnly", "calendarFullAccess",
        "reminders", "bluetooth", "bluetoothScan", "bluetoothConnect", "bluetoothAdvertise",
    ]
    #if os(iOS)
        return common.union(["audio", "mediaLibrary", "sensors", "activityRecognition", "appTrackingTransparency", "assistant"])
    #elseif os(macOS)
        return common.union(["screenCapture", "accessibility", "fullDiskAccess"])
    #endif
    }()

    private static func coreStatus(_ name: String) -> PermissionStatus {
        switch name {
        case "storage", "accessMediaLocation", "scheduleExactAlarm":
            return .granted
        case "backgroundRefresh":
            #if os(iOS)
            switch UIApplication.shared.backgroundRefreshStatus {
            case .available: return .granted
            case .denied: return .permanentlyDenied
            case .restricted: return .restricted
            @unknown default: return .restricted
            }
            #else
            return .granted
            #endif
        #if os(macOS)
        case "audio": return .granted
        #endif
        default: return .restricted
        }
    }

    private static func openSettings(_ name: String) async -> Bool {
        #if os(iOS)
        let page: String
        if #available(iOS 16, *), name == "notification" || name == "criticalAlerts" {
            page = UIApplication.openNotificationSettingsURLString
        } else {
            page = UIApplication.openSettingsURLString
        }
        guard let url = URL(string: page) else { return false }
        if await UIApplication.shared.open(url) { return true }
        guard page != UIApplication.openSettingsURLString,
              let fallback = URL(string: UIApplication.openSettingsURLString) else { return false }
        return await UIApplication.shared.open(fallback)
        #else
        let pane = "x-apple.systempreferences:com.apple.preference.security?Privacy"
        let suffix = switch name {
        case "screenCapture": "_ScreenCapture"
        case "accessibility": "_Accessibility"
        case "fullDiskAccess": "_AllFiles"
        case "camera": "_Camera"
        case "microphone", "speech": "_Microphone"
        case "location", "locationWhenInUse", "locationAlways": "_LocationServices"
        case "contacts": "_Contacts"
        case "calendarFullAccess", "calendarWriteOnly": "_Calendars"
        case "reminders": "_Reminders"
        case "photos", "photosAddOnly", "videos": "_Photos"
        case "bluetooth", "bluetoothScan", "bluetoothConnect", "bluetoothAdvertise": "_Bluetooth"
        default: ""
        }
        if let url = URL(string: pane + suffix), NSWorkspace.shared.open(url) { return true }
        guard !suffix.isEmpty, let url = URL(string: pane) else { return false }
        return NSWorkspace.shared.open(url)
        #endif
    }
}

/// System permission prompts are process-wide across independent host windows.
@MainActor
private final class PermissionPromptQueue {
    static let shared = PermissionPromptQueue()

    private struct Job {
        let owner: UUID
        let cancel: () -> Void
        let operation: () async -> Void
    }

    private var waiting: [Job] = []
    private var active: (Job, Task<Void, Never>)?

    func enqueue(_ owner: UUID, cancel: @escaping () -> Void, operation: @escaping () async -> Void) {
        waiting.append(Job(owner: owner, cancel: cancel, operation: operation))
        advance()
    }

    func cancel(_ owner: UUID) {
        let cancelled = waiting.filter { $0.owner == owner }
        waiting.removeAll { $0.owner == owner }
        for job in cancelled { job.cancel() }
        if let (job, task) = active, job.owner == owner {
            job.cancel()
            task.cancel()
        }
    }

    private func advance() {
        guard active == nil, !waiting.isEmpty else { return }
        let job = waiting.removeFirst()
        active = (job, Task { @MainActor in
            await job.operation()
            self.active = nil
            self.advance()
        })
    }
}
