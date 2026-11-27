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
let module = "ImpastoConnectivity"

let package = Package(
    name: module,
    platforms: isMacOS ? [.macOS(.v13)] : [.iOS(.v15)],
    products: [.library(name: module, targets: [module])],
    dependencies: [.package(name: "ImpastoHost", path: host)],
    targets: [.target(name: module, dependencies: [.product(name: "ImpastoHost", package: "ImpastoHost")])]
)
