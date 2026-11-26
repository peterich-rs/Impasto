// swift-tools-version: 5.9
import Foundation
import PackageDescription

let host = ProcessInfo.processInfo.environment["IMPASTO_ROOT"].map {
    "\($0)/crates/embedder/ios/ImpastoHost"
} ?? "../../../crates/embedder/ios/ImpastoHost"

let package = Package(
    name: "ImpastoLinking",
    platforms: [.iOS(.v15)],
    products: [.library(name: "ImpastoLinking", targets: ["ImpastoLinking"])],
    dependencies: [.package(name: "ImpastoHost", path: host)],
    targets: [
        .target(name: "ImpastoLinking", dependencies: [.product(name: "ImpastoHost", package: "ImpastoHost")]),
    ]
)
