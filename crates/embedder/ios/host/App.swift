import CryptoKit
import Foundation
import UIKit

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    private let sharedWindows: ImpastoWindowGroup? =
        (Bundle.main.object(forInfoDictionaryKey: "ImpastoSharedWindowState") as? Bool) == true
        ? ImpastoWindowGroup() : nil

    var windowGroup: ImpastoWindowGroup? { sharedWindows }

    func application(
        _ application: UIApplication,
        configurationForConnecting session: UISceneSession,
        options: UIScene.ConnectionOptions
    ) -> UISceneConfiguration {
        let configuration = UISceneConfiguration(name: "Impasto", sessionRole: session.role)
        configuration.sceneClass = UIWindowScene.self
        configuration.delegateClass = SceneDelegate.self
        return configuration
    }

    func application(_ application: UIApplication, didDiscardSceneSessions sessions: Set<UISceneSession>) {
        for session in sessions {
            SceneRestorationStorage.discard(session.persistentIdentifier)
        }
    }
}

final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    private var host: ImpastoHost?
    private var lastSavedState: String?
    private var currentSceneID: String?
    private var externalDisplayRegistration: AnyObject?

    func scene(
        _ scene: UIScene,
        willConnectTo session: UISceneSession,
        options: UIScene.ConnectionOptions
    ) {
        guard let scene = scene as? UIWindowScene, let resources = Bundle.main.resourceURL else { return }
        let interactive = session.role == .windowApplication
        let offersExternalDisplay = Bundle.main.object(forInfoDictionaryKey: "ImpastoPresentOnExternalDisplay") as? Bool == true
        guard interactive || (offersExternalDisplay && Self.isExternalDisplay(session.role)) else { return }
        let urls = options.urlContexts.map(\.url).sorted { $0.absoluteString < $1.absoluteString }
        let universalLinks = options.userActivities.compactMap(Self.universalLink)
            .sorted { $0.absoluteString < $1.absoluteString }
        let initialLinks = (urls + universalLinks).reduce(into: [URL]()) { seen, next in
            if !seen.contains(next) { seen.append(next) }
        }
        let identifier = session.persistentIdentifier
        currentSceneID = identifier
        // The local copy is updated during active use. UIKit's copy can predate it.
        let restored = SceneRestorationStorage.read(identifier)
            ?? SceneRestorationStorage.restore(session.stateRestorationActivity, identifier: identifier)
        lastSavedState = restored
        let windowGroup = (UIApplication.shared.delegate as? AppDelegate)?.windowGroup
        let host = ImpastoHost(
            resources: resources, initialURL: initialLinks.first,
            restoredState: restored, windowGroup: windowGroup
        )
        host.onRestorationChanged = { [weak self] in self?.saveSnapshot() }
        let window = UIWindow(windowScene: scene)
        let content = host.makeViewController()
        window.rootViewController = content
        self.host = host
        self.window = window
        host.prepare()
        for url in initialLinks.dropFirst() { host.open(url) }
        if interactive {
            window.makeKeyAndVisible()
            #if compiler(>=6.4)
            if #available(iOS 27.0, *), offersExternalDisplay {
                let configuration = UISceneConfiguration()
                configuration.sceneClass = UIWindowScene.self
                configuration.delegateClass = SceneDelegate.self
                externalDisplayRegistration = content.registerSceneAccessory(
                    .externalNonInteractive(sceneConfiguration: configuration)
                )
            }
            #endif
        } else {
            window.isHidden = false
        }
    }

    func sceneDidEnterBackground(_ scene: UIScene) {
        saveSnapshot()
    }

    func sceneDidDisconnect(_ scene: UIScene) {
        saveSnapshot()
        #if compiler(>=6.4)
        if #available(iOS 27.0, *),
           let content = window?.rootViewController,
           let registration = externalDisplayRegistration as? UISceneAccessoryRegistration {
            content.unregisterSceneAccessory(registration)
        }
        #endif
        externalDisplayRegistration = nil
        host?.close()
        host = nil
        window = nil
        currentSceneID = nil
    }

    func stateRestorationActivity(for scene: UIScene) -> NSUserActivity? {
        saveSnapshot()
        let identifier = scene.session.persistentIdentifier
        guard currentSceneID == identifier, let lastSavedState else { return nil }
        return SceneRestorationStorage.activity(for: lastSavedState, identifier: identifier)
    }

    func scene(_ scene: UIScene, openURLContexts contexts: Set<UIOpenURLContext>) {
        for context in contexts { host?.open(context.url) }
    }

    func scene(_ scene: UIScene, continue userActivity: NSUserActivity) {
        if let url = Self.universalLink(userActivity) { host?.open(url) }
    }

    private static func universalLink(_ activity: NSUserActivity) -> URL? {
        activity.activityType == NSUserActivityTypeBrowsingWeb ? activity.webpageURL : nil
    }

    private static func isExternalDisplay(_ role: UISceneSession.Role) -> Bool {
        if #available(iOS 16.0, *) { return role == .windowExternalDisplayNonInteractive }
        return role == .windowExternalDisplay
    }

    private func saveSnapshot() {
        guard let identifier = currentSceneID,
              let snapshot = host?.restorationSnapshot(), snapshot != lastSavedState
        else { return }
        lastSavedState = snapshot
        SceneRestorationStorage.write(snapshot, identifier: identifier)
    }
}

private enum SceneRestorationStorage {
    static let maxSize = 32 * 1024
    static let activityType = "dev.impasto.scene-restoration"
    static let writes = DispatchQueue(label: "dev.impasto.scene-restoration")

    static func restore(_ activity: NSUserActivity?, identifier: String) -> String? {
        guard let activity,
              activity.activityType == activityType,
              activity.userInfo?["scene"] as? String == identifier,
              let snapshot = activity.userInfo?["snapshot"] as? String
        else { return nil }
        return accepted(snapshot)
    }

    static func activity(for snapshot: String, identifier: String) -> NSUserActivity {
        let activity = NSUserActivity(activityType: activityType)
        activity.addUserInfoEntries(from: ["scene": identifier, "snapshot": snapshot])
        return activity
    }

    static func read(_ identifier: String) -> String? {
        guard let location = try? location(identifier, create: false),
              let handle = try? FileHandle(forReadingFrom: location)
        else { return nil }
        defer { try? handle.close() }
        guard let data = try? handle.read(upToCount: maxSize + 1),
              let snapshot = String(data: data, encoding: .utf8) else { return nil }
        return accepted(snapshot)
    }

    static func write(_ snapshot: String, identifier: String) {
        writes.async {
            guard let location = try? location(identifier, create: true) else { return }
            try? Data(snapshot.utf8).write(to: location, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        }
    }

    static func discard(_ identifier: String) {
        writes.async {
            guard let location = try? location(identifier, create: false) else { return }
            try? FileManager.default.removeItem(at: location)
        }
    }

    private static func accepted(_ snapshot: String) -> String? {
        guard snapshot.utf8.count <= maxSize, snapshot.hasPrefix("1\n") else { return nil }
        return snapshot
    }

    private static func location(_ identifier: String, create: Bool) throws -> URL {
        let manager = FileManager.default
        let base = try manager.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: create)
        var directory = base.appendingPathComponent("ImpastoSceneRestoration", isDirectory: true)
        if create {
            try manager.createDirectory(at: directory, withIntermediateDirectories: true)
            var metadata = URLResourceValues()
            metadata.isExcludedFromBackup = true
            try? directory.setResourceValues(metadata)
        }
        let name = SHA256.hash(data: Data(identifier.utf8)).map { String(format: "%02x", $0) }.joined()
        return directory.appendingPathComponent(name)
    }
}
