// swift-tools-version: 5.9
import Foundation
import PackageDescription

let host = ProcessInfo.processInfo.environment["IMPASTO_ROOT"].map {
    "\($0)/crates/embedder/ios/ImpastoHost"
} ?? "../../../crates/embedder/ios/ImpastoHost"

let package = Package(
    name: "ImpastoCamera",
    platforms: [.iOS(.v15)],
    products: [.library(name: "ImpastoCamera", targets: ["ImpastoCamera"])],
    dependencies: [.package(name: "ImpastoHost", path: host)],
    targets: [
        .target(
            name: "ImpastoCamera",
            dependencies: [.product(name: "ImpastoHost", package: "ImpastoHost")],
            resources: [.process("Resources")],
            linkerSettings: [
                .linkedFramework("AVFoundation"),
                .linkedFramework("CoreMedia"),
                .linkedFramework("CoreVideo"),
                .linkedFramework("Metal"),
            ]
        ),
    ]
)
