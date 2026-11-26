// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ImpastoHost",
    platforms: [.iOS(.v15)],
    products: [.library(name: "ImpastoHost", targets: ["ImpastoHost"])],
    targets: [
        .target(name: "CImpastoIOS", path: "include", publicHeadersPath: "."),
        .target(
            name: "ImpastoHost",
            dependencies: ["CImpastoIOS"],
            path: "host",
            exclude: ["LaunchScreen.storyboard"],
            swiftSettings: [.define("IMPASTO_IOS_DEVELOPMENT", .when(configuration: .debug))]
        ),
    ]
)
