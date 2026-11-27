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
let core = "ImpastoPermissionsCore"
let common = ["Camera", "Microphone", "Speech", "Photos", "Location", "Notification",
              "Contacts", "Calendar", "Reminders", "Bluetooth"]
let strategies = (common + (isMacOS ? ["Desktop"] : ["Assistant", "Motion", "MediaLibrary", "Tracking"]))
    .map { "ImpastoPermissions\($0)" }

let package = Package(
    name: "ImpastoPermissions",
    platforms: isMacOS ? [.macOS(.v13)] : [.iOS(.v15)],
    products: [.library(name: "ImpastoPermissions", targets: [core])] +
        strategies.map { .library(name: $0, targets: [$0]) },
    dependencies: [.package(name: "ImpastoHost", path: host)],
    targets: [.target(name: core, dependencies: [.product(name: "ImpastoHost", package: "ImpastoHost")])] +
        strategies.map { .target(name: $0, dependencies: [.target(name: core)]) }
)
