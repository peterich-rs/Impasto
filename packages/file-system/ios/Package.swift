// swift-tools-version: 5.9
import Foundation
import PackageDescription

let host = ProcessInfo.processInfo.environment["IMPASTO_ROOT"].map {
    "\($0)/crates/embedder/ios/ImpastoHost"
} ?? "../../../crates/embedder/ios/ImpastoHost"

let package = Package(
    name: "ImpastoFileSystem",
    platforms: [.iOS(.v15)],
    products: [.library(name: "ImpastoFileSystem", targets: ["ImpastoFileSystem"])],
    dependencies: [.package(name: "ImpastoHost", path: host)],
    targets: [
        .target(name: "ImpastoFileSystem", dependencies: [.product(name: "ImpastoHost", package: "ImpastoHost")]),
    ]
)
