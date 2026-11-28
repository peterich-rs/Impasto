// swift-tools-version: 5.9
import Foundation
import PackageDescription

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().resolvingSymlinksInPath()
let isMacOS = root.lastPathComponent == "macos"
let platform = isMacOS ? "macos" : "ios"
let framework = ProcessInfo.processInfo.environment["IMPASTO_ROOT"] ??
    root.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().path
let host = "\(framework)/crates/embedder/\(platform)/ImpastoHost"
let module = "ImpastoNotifications"

let feature = "ImpastoNotificationsCriticalAlerts"
let featureEnabled = true

let package = Package(
    name: module,
    platforms: isMacOS ? [.macOS(.v13)] : [.iOS(.v15)],
    products: [.library(name: module, targets: [module])] + (featureEnabled ? [.library(name: feature, targets: [feature])] : []),
    dependencies: [.package(name: "ImpastoHost", path: host)],
    targets: [.target(name: module, dependencies: [.product(name: "ImpastoHost", package: "ImpastoHost")], resources: [.process("Resources")])] + (featureEnabled ? [.target(name: feature, dependencies: [.target(name: module)])] : [])
)
