import Foundation
import ImpastoHost
import UserNotifications
#if os(iOS)
import UIKit
#else
import AppKit
#endif

private struct RoutedResponse: Sendable {
    let identity: String
    let payload: Data
}

@MainActor
private final class NotificationRouter: NSObject, UNUserNotificationCenterDelegate {
    static let shared = NotificationRouter()
    weak var active: ImpastoNotificationsPlugin?
    var initial: RoutedResponse?
    private var recent: [String] = []
    var policy: UNNotificationPresentationOptions = [.banner, .list, .sound, .badge]

    static var center: UNUserNotificationCenter? {
        guard Bundle.main.bundleIdentifier != nil, Bundle.main.bundleURL.pathExtension == "app" else { return nil }
        return UNUserNotificationCenter.current()
    }

    nonisolated private static func preparedResponse(_ response: UNNotificationResponse) -> RoutedResponse? {
        let request = response.notification.request
        let normal = response.actionIdentifier == UNNotificationDefaultActionIdentifier
        var payload: [String: Any] = [
            "notificationId": request.identifier,
            "actionId": normal ? "default" : response.actionIdentifier,
            "data": request.content.userInfo["impasto.data"] as? [String: String] ?? [:],
            "receivedAtMs": Date().timeIntervalSince1970 * 1000,
        ]
        if let input = response as? UNTextInputNotificationResponse { payload["userText"] = input.userText }
        guard let encoded = try? JSONSerialization.data(withJSONObject: payload) else { return nil }
        let key = "\(request.identifier)|\(response.actionIdentifier)|\(response.notification.date.timeIntervalSince1970)"
        return RoutedResponse(identity: key, payload: encoded)
    }

    nonisolated static func foreground(_ request: UNNotificationRequest) -> Data? {
        let content = request.content
        let data: [String: Any] = [
            "id": request.identifier, "title": content.title, "subtitle": content.subtitle,
            "body": content.body, "categoryId": content.categoryIdentifier, "threadId": content.threadIdentifier,
            "data": content.userInfo["impasto.data"] as? [String: String] ?? [:],
        ]
        return try? JSONSerialization.data(withJSONObject: data)
    }

    private func remember(_ response: RoutedResponse) -> Bool {
        guard !recent.contains(response.identity) else { return false }
        recent.append(response.identity)
        if recent.count > 32 { recent.removeFirst() }
        return true
    }

    func sceneResponse(_ response: UNNotificationResponse, in plugin: ImpastoNotificationsPlugin) {
        guard let routed = Self.preparedResponse(response) else { return }
        _ = remember(routed)
        plugin.acceptInitial(routed)
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let data = Self.preparedResponse(response) else { return }
        await MainActor.run {
            guard self.remember(data) else { return }
            if let active = self.active { active.receive(data) }
            else if self.initial == nil { self.initial = data }
        }
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        let payload = Self.foreground(notification.request)
        return await MainActor.run {
            if let payload { self.active?.emit("foreground", payload) }
            return self.policy
        }
    }
}

private struct InvalidNotification: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

@MainActor
public final class ImpastoNotificationsPlugin: ImpastoPlugin {
    public let identifier = "@impasto/notifications"
    private weak var host: ImpastoHost?
    private var initial: RoutedResponse?
    private var responses: [RoutedResponse] = []
    private var listening = Set<String>()
    private var critical = false
    private var badge = 0

    public static func prepareLaunch() { NotificationRouter.center?.delegate = NotificationRouter.shared }

    public static func register(with host: ImpastoHost) { host.registerPlugin(Self()) }
    public static func register(with host: ImpastoHost, enabled: Set<String>, registrars: [any ImpastoNotificationsFeature.Type]) {
        let plugin = Self()
        plugin.critical = enabled.contains("criticalAlerts")
        host.registerPlugin(plugin)
    }
    public func installed(in host: ImpastoHost) {
        self.host = host
        Self.prepareLaunch()
        let router = NotificationRouter.shared
        router.active = self
        if let waiting = router.initial {
            initial = waiting
            router.initial = nil
        }
    }
    #if os(iOS)
    public func sceneConnected(_ options: UIScene.ConnectionOptions) {
        if let response = options.notificationResponse { NotificationRouter.shared.sceneResponse(response, in: self) }
    }
    public func sceneActivityChanged(_ activity: ImpastoSceneActivity) {
        if case .foreground = activity { NotificationRouter.shared.active = self }
    }
    #endif

    fileprivate func acceptInitial(_ response: RoutedResponse) {
        responses.removeAll { $0.identity == response.identity }
        if initial == nil { initial = response }
    }
    fileprivate func receive(_ response: RoutedResponse) {
        if listening.contains("response") { emit("response", response.payload) }
        else {
            responses.append(response)
            if responses.count > 16 { responses.removeFirst() }
        }
    }
    fileprivate func emit(_ topic: String, _ payload: Data) {
        guard listening.contains(topic) else { return }
        // The Rust stream transports the encoded object as one string.
        if let wire = try? JSONEncoder().encode(String(decoding: payload, as: UTF8.self)) {
            host?.emitPluginEvent(topic: "\(identifier)/\(topic)", payload: String(decoding: wire, as: UTF8.self))
        }
    }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        if request.operation == "@impasto/events/start" || request.operation == "@impasto/events/stop" {
            guard let config = try? json(request.payload) as? [String: Any],
                  let topic = config["topic"] as? String, topic.hasPrefix("\(identifier)/") else { return false }
            guard NotificationRouter.center != nil else { request.fail(code: "host-unavailable", message: "Notifications require an application bundle"); return true }
            let suffix = String(topic.dropFirst(identifier.count + 1))
            if request.operation.hasSuffix("/start") {
                listening.insert(suffix)
                if suffix == "response" { for response in responses { emit("response", response.payload) }; responses.removeAll() }
            } else { listening.remove(suffix) }
            request.resolve()
            return true
        }
        guard request.operation.hasPrefix("\(identifier)/") else { return false }
        guard let center = NotificationRouter.center else { request.fail(code: "host-unavailable", message: "Notifications require an application bundle"); return true }
        let action = String(request.operation.dropFirst(identifier.count + 1))
        Task { @MainActor in
            do {
                let input = try json(request.payload)
                let fields = input as? [String: Any] ?? [:]
                switch action {
                case "submit":
                    guard let config = fields["request"] as? [String: Any] else { throw InvalidNotification(message: "Missing notification request") }
                    let notice = try await make(config: config, trigger: fields["trigger"] as? [String: Any])
                    try await center.add(notice)
                    request.resolve()
                case "cancel":
                    guard let id = input as? String else { throw InvalidNotification(message: "Missing notification identifier") }
                    center.removePendingNotificationRequests(withIdentifiers: [id]); request.resolve()
                case "cancelAll":
                    center.removeAllPendingNotificationRequests(); request.resolve()
                case "getPending":
                    let pending = await center.pendingNotificationRequests()
                    let values: [[String: Any]] = pending.compactMap { item in
                        guard let trigger = item.content.userInfo["impasto.trigger"] as? String,
                              let trigger = try? json(trigger) else { return nil }
                        return ["id": item.identifier, "title": item.content.title, "body": item.content.body,
                                "trigger": trigger, "data": item.content.userInfo["impasto.data"] as? [String: String] ?? [:]]
                    }
                    try reply(request, values)
                case "dismissDelivered":
                    if let id = input as? String { center.removeDeliveredNotifications(withIdentifiers: [id]) }
                    else { center.removeAllDeliveredNotifications() }
                    request.resolve()
                case "createChannel", "deleteChannel": request.resolve()
                case "listChannels": try reply(request, [String]())
                case "registerCategories":
                    guard let list = input as? [[String: Any]] else { throw InvalidNotification(message: "Invalid notification categories") }
                    center.setNotificationCategories(Set(try list.map(category)))
                    request.resolve()
                case "setBadgeCount":
                    guard let count = input as? Int, count >= 0 else { throw InvalidNotification(message: "Invalid badge count") }
                    try await setBadge(center, count)
                    request.resolve()
                case "getBadgeCount":
                    #if os(iOS)
                    request.resolveJSON(UIApplication.shared.applicationIconBadgeNumber)
                    #else
                    request.resolveJSON(badge)
                    #endif
                case "setForegroundPresentation":
                    var policy: UNNotificationPresentationOptions = []
                    if fields["alert"] as? Bool == true { policy.formUnion([.banner, .list]) }
                    if fields["sound"] as? Bool == true { policy.insert(.sound) }
                    if fields["badge"] as? Bool == true { policy.insert(.badge) }
                    NotificationRouter.shared.policy = policy
                    request.resolve()
                case "getInitialResponse":
                    if let initial { request.resolve(String(decoding: initial.payload, as: UTF8.self)); self.initial = nil }
                    else { request.resolve() }
                default: request.fail(code: "invalid-argument", message: "Unknown notification operation")
                }
            } catch let error as InvalidNotification { request.fail(code: "invalid-argument", message: error.message) }
            catch { request.fail(code: "platform", message: "The notification operation failed") }
        }
        return true
    }

    private func setBadge(_ center: UNUserNotificationCenter, _ count: Int) async throws {
        #if os(iOS)
        if #available(iOS 16.0, *) { try await center.setBadgeCount(count) }
        else { UIApplication.shared.applicationIconBadgeNumber = count }
        #else
        try await center.setBadgeCount(count)
        NSApp.dockTile.badgeLabel = count == 0 ? nil : String(count)
        #endif
        badge = count
    }

    private func category(_ config: [String: Any]) throws -> UNNotificationCategory {
        guard let id = config["id"] as? String else { throw InvalidNotification(message: "Missing category identifier") }
        let actions = try (config["actions"] as? [[String: Any]] ?? []).map { action -> UNNotificationAction in
            guard let id = action["id"] as? String, let title = action["title"] as? String else {
                throw InvalidNotification(message: "Missing notification action")
            }
            var opts: UNNotificationActionOptions = []
            if action["foreground"] as? Bool != false { opts.insert(.foreground) }
            if action["destructive"] as? Bool == true { opts.insert(.destructive) }
            if action["authenticationRequired"] as? Bool == true { opts.insert(.authenticationRequired) }
            if let text = action["textInput"] as? [String: Any] {
                return UNTextInputNotificationAction(identifier: id, title: title, options: opts,
                    textInputButtonTitle: text["buttonTitle"] as? String ?? "Send", textInputPlaceholder: text["placeholder"] as? String ?? "")
            }
            return UNNotificationAction(identifier: id, title: title, options: opts)
        }
        return UNNotificationCategory(identifier: id, actions: actions, intentIdentifiers: [], options: [])
    }

    private func make(config: [String: Any], trigger: [String: Any]?) async throws -> UNNotificationRequest {
        guard let id = config["id"] as? String, let title = config["title"] as? String, let body = config["body"] as? String
        else { throw InvalidNotification(message: "Missing notification fields") }
        let content = UNMutableNotificationContent()
        content.title = title
        content.subtitle = config["subtitle"] as? String ?? ""
        content.body = body
        content.categoryIdentifier = config["categoryId"] as? String ?? ""
        content.threadIdentifier = config["threadId"] as? String ?? ""
        if let badge = config["badge"] as? NSNumber { content.badge = badge }
        let sound = config["sound"] as? String ?? "default"
        if sound == "default" { content.sound = .default }
        else if sound != "none" { content.sound = UNNotificationSound(named: UNNotificationSoundName(rawValue: sound)) }
        switch config["interruptionLevel"] as? String {
        case "passive": content.interruptionLevel = .passive
        case "timeSensitive": content.interruptionLevel = .timeSensitive
        case "critical":
            guard critical else { throw InvalidNotification(message: "Critical alerts are not enabled") }
            content.interruptionLevel = .critical
            if sound != "none" { content.sound = .defaultCritical }
        default: content.interruptionLevel = .active
        }
        content.userInfo["impasto.data"] = config["data"] as? [String: String] ?? [:]
        if let trigger, let bytes = try? JSONSerialization.data(withJSONObject: trigger) {
            content.userInfo["impasto.trigger"] = String(decoding: bytes, as: UTF8.self)
        }
        let attachments = (config["attachments"] as? [[String: Any]] ?? []).compactMap { fields -> (String, URL)? in
            guard let path = fields["path"] as? String else { return nil }
            return (fields["id"] as? String ?? UUID().uuidString, URL(fileURLWithPath: path))
        }
        content.attachments = try await Task.detached { try Self.copyAttachments(attachments) }.value
        return UNNotificationRequest(identifier: id, content: content, trigger: try makeTrigger(trigger))
    }

    nonisolated private static func copyAttachments(_ sources: [(String, URL)]) throws -> [UNNotificationAttachment] {
        guard !sources.isEmpty else { return [] }
        let manager = FileManager.default
        let directory = manager.temporaryDirectory.appendingPathComponent("impasto-notif-staging", isDirectory: true)
        try manager.createDirectory(at: directory, withIntermediateDirectories: true)
        return try sources.map { (id, source) in
            let copy = directory.appendingPathComponent(UUID().uuidString).appendingPathExtension(source.pathExtension)
            try manager.copyItem(at: source, to: copy)
            do { return try UNNotificationAttachment(identifier: id, url: copy) }
            catch { try? manager.removeItem(at: copy); throw error }
        }
    }

    private func makeTrigger(_ source: [String: Any]?) throws -> UNNotificationTrigger? {
        guard let source else { return nil }
        let repeats = source["repeats"] as? Bool ?? false
        switch source["kind"] as? String {
        case "date":
            guard let at = source["atMs"] as? Double else { throw InvalidNotification(message: "Invalid notification date") }
            let delay = at / 1000 - Date().timeIntervalSince1970
            guard delay > 0 else { throw InvalidNotification(message: "Notification date is in the past") }
            return UNTimeIntervalNotificationTrigger(timeInterval: max(1, delay), repeats: false)
        case "interval":
            guard let seconds = source["everySeconds"] as? Double, seconds >= (repeats ? 60 : 1)
            else { throw InvalidNotification(message: "Invalid notification interval") }
            return UNTimeIntervalNotificationTrigger(timeInterval: seconds, repeats: repeats)
        case "calendar":
            var components = DateComponents()
            components.year = source["year"] as? Int; components.month = source["month"] as? Int
            components.day = source["day"] as? Int; components.weekday = source["weekday"] as? Int
            components.hour = source["hour"] as? Int; components.minute = source["minute"] as? Int; components.second = source["second"] as? Int
            if let zone = source["timeZone"] as? String {
                guard let timezone = TimeZone(identifier: zone) else { throw InvalidNotification(message: "Invalid notification time zone") }
                components.timeZone = timezone
            }
            let trigger = UNCalendarNotificationTrigger(dateMatching: components, repeats: repeats)
            guard trigger.nextTriggerDate() != nil else { throw InvalidNotification(message: "No future calendar match") }
            return trigger
        default: throw InvalidNotification(message: "Unknown notification trigger")
        }
    }

    private func json(_ text: String) throws -> Any { try JSONSerialization.jsonObject(with: Data(text.utf8), options: [.fragmentsAllowed]) }
    private func reply(_ request: ImpastoNativeRequest, _ value: Any) throws {
        request.resolve(String(decoding: try JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed]), as: UTF8.self))
    }
    public func detached() {
        listening.removeAll(); responses.removeAll()
        if NotificationRouter.shared.active === self { NotificationRouter.shared.active = nil }
    }
    public func uninstalled() { detached() }
}
public protocol ImpastoNotificationsFeature {}
