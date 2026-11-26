// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ImpastoHost",
    platforms: [.macOS(.v13)],
    products: [.library(name: "ImpastoHost", targets: ["ImpastoHost"])],
    targets: [
        .target(name: "CImpastoMacOS", path: "include", publicHeadersPath: "."),
        .target(name: "ImpastoHost", dependencies: ["CImpastoMacOS"], path: "host"),
    ]
)
