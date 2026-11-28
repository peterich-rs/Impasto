import Foundation
import ImpastoHost
#if os(iOS)
import UIKit
#endif

@MainActor
public final class ImpastoSystemInfoPlugin: ImpastoPlugin {
    public let identifier = "@impasto/system-info"

    public static func register(with host: ImpastoHost) { host.registerPlugin(Self()) }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        switch request.operation {
        case "\(identifier)/appInfo": request.resolveJSON(Self.app())
        case "\(identifier)/deviceInfo": request.resolveJSON(Self.device())
        case "\(identifier)/runtimeInfo":
            Task { @MainActor in
                do {
                    let info = try await Task.detached { try Self.runtime() }.value
                    request.resolveJSON(info)
                } catch { request.fail(code: "platform", message: error.localizedDescription) }
            }
        default: return false
        }
        return true
    }

    private struct App: Encodable {
        let appName: String
        let packageName: String
        let version: String
        let buildNumber: String
        let installerStore: String
        let isDebug: Bool
    }

    private static func app() -> App {
        let bundle = Bundle.main
        func info(_ key: String) -> String? { bundle.object(forInfoDictionaryKey: key) as? String }
        var isDebug = false
        var store = "sideload"
        #if DEBUG
        isDebug = true
        #endif
        #if targetEnvironment(simulator)
        store = "simulator"
        #else
        if let receipt = bundle.appStoreReceiptURL, FileManager.default.fileExists(atPath: receipt.path) {
            store = receipt.lastPathComponent == "sandboxReceipt" ? "testFlight" : "appStore"
        }
        #endif
        return App(appName: info("CFBundleDisplayName") ?? info("CFBundleName") ?? "",
                   packageName: bundle.bundleIdentifier ?? "", version: info("CFBundleShortVersionString") ?? "",
                   buildNumber: info("CFBundleVersion") ?? "", installerStore: store, isDebug: isDebug)
    }

    private struct Device: Encodable {
        let platform: String
        let osVersion: String
        let osBuild: String
        let sdkInt: Int? = nil
        let model: String
        let brand = "Apple"
        let manufacturer = "Apple"
        let architecture: String
        let isPhysicalDevice: Bool
        let processorCount: Int
        let totalMemoryBytes: UInt64
        let locale: String
        let timeZone: String
    }

    private static func device() -> Device {
        let process = ProcessInfo.processInfo
        let version = process.operatingSystemVersion
        #if os(macOS)
        let platform = "macos"
        let model = system("hw.model")
        #else
        let platform = "ios"
        let model = ProcessInfo.processInfo.environment["SIMULATOR_MODEL_IDENTIFIER"] ?? system("hw.machine")
        #endif
        #if arch(arm64)
        let architecture = "arm64"
        #else
        let architecture = "x86_64"
        #endif
        #if targetEnvironment(simulator)
        let physical = false
        #else
        let physical = true
        #endif
        return Device(platform: platform, osVersion: "\(version.majorVersion).\(version.minorVersion).\(version.patchVersion)",
                      osBuild: system("kern.osversion"), model: model, architecture: architecture,
                      isPhysicalDevice: physical, processorCount: process.processorCount,
                      totalMemoryBytes: process.physicalMemory, locale: Locale.current.identifier,
                      timeZone: TimeZone.current.identifier)
    }

    private static func system(_ name: String) -> String {
        var count = 0
        guard sysctlbyname(name, nil, &count, nil, 0) == 0, count > 0 else { return "" }
        var bytes = [UInt8](repeating: 0, count: count)
        let result = bytes.withUnsafeMutableBytes { sysctlbyname(name, $0.baseAddress, &count, nil, 0) }
        guard result == 0 else { return "" }
        return String(decoding: bytes.prefix { $0 != 0 }, as: UTF8.self)
    }

    private struct Runtime: Encodable {
        let freeDiskBytes: Int64
        let totalDiskBytes: Int
        let isLowPowerMode: Bool
    }

    nonisolated private static func runtime() throws -> Runtime {
        let values = try FileManager.default.temporaryDirectory.resourceValues(forKeys: [
            .volumeAvailableCapacityForImportantUsageKey, .volumeAvailableCapacityKey, .volumeTotalCapacityKey
        ])
        return Runtime(freeDiskBytes: values.volumeAvailableCapacityForImportantUsage ?? Int64(values.volumeAvailableCapacity ?? 0),
                       totalDiskBytes: values.volumeTotalCapacity ?? 0,
                       isLowPowerMode: ProcessInfo.processInfo.isLowPowerModeEnabled)
    }
}
