import Foundation
import ImpastoHost
import LocalAuthentication

private struct Prompt: Decodable {
    let id: UInt64
    let reason: String
    let cancelLabel: String?
    let fallbackLabel: String?
    let allowDeviceCredential: Bool?
    let allowWatch: Bool?
}

private struct Capabilities: Encodable {
    let isAvailable: Bool
    let isEnrolled: Bool
    let hasDeviceCredential: Bool
    let biometryTypes: [String]
    let securityLevel: String
}
private struct Authentication: Encodable { let authenticated: Bool; let reason: String?; let message: String? }

@MainActor
public final class ImpastoBiometricsPlugin: ImpastoPlugin {
    public let identifier = "@impasto/biometrics"
    private var active: (id: UInt64, request: ImpastoNativeRequest, context: LAContext)?

    public static func register(with host: ImpastoHost) { host.registerPlugin(Self()) }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        do {
            switch request.operation {
            case "\(identifier)/capabilities": request.resolveJSON(capabilities())
            case "\(identifier)/authenticate": authenticate(request, try request.decodeJSON(Prompt.self))
            case "\(identifier)/cancel":
                let id = try request.decodeJSON(UInt64.self)
                if active?.id == id { finish(false, "cancelled", "Authentication was cancelled") }
                request.resolve()
            default: return false
            }
        } catch { request.fail(code: "invalid-argument", message: "Invalid authentication request") }
        return true
    }

    private func capabilities() -> Capabilities {
        let context = LAContext()
        var failure: NSError?
        let enrolled = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &failure)
        let hasHardware = context.biometryType != .none
        let credential = context.canEvaluatePolicy(.deviceOwnerAuthentication, error: nil)
        var types = [String]()
        if context.biometryType == .touchID { types = ["fingerprint"] }
        else if context.biometryType == .faceID { types = ["face"] }
        #if os(iOS)
        if #available(iOS 17.0, *), context.biometryType == .opticID { types = ["opticId"] }
        #endif
        return Capabilities(isAvailable: hasHardware || credential, isEnrolled: enrolled || failure?.code == LAError.biometryLockout.rawValue,
                            hasDeviceCredential: credential, biometryTypes: types, securityLevel: hasHardware ? "strong" : "none")
    }

    private func authenticate(_ request: ImpastoNativeRequest, _ prompt: Prompt) {
        guard active == nil else { request.fail(code: "busy", message: "Authentication is already in progress"); return }
        let context = LAContext()
        context.localizedCancelTitle = prompt.cancelLabel
        context.localizedFallbackTitle = prompt.fallbackLabel
        var policy: LAPolicy = prompt.allowDeviceCredential == true ? .deviceOwnerAuthentication : .deviceOwnerAuthenticationWithBiometrics
        #if os(macOS)
        if prompt.allowWatch == true, prompt.allowDeviceCredential != true { policy = .deviceOwnerAuthenticationWithBiometricsOrWatch }
        #elseif os(iOS)
        if Bundle.main.object(forInfoDictionaryKey: "NSFaceIDUsageDescription") == nil {
            request.fail(code: "missing-config", message: "Missing Face ID usage description")
            return
        }
        #endif
        active = (prompt.id, request, context)
        context.evaluatePolicy(policy, localizedReason: prompt.reason) { [weak self] success, error in
            let reason = (error as? LAError).map(Self.reason) ?? "failed"
            Task { @MainActor in
                guard let self, self.active?.id == prompt.id else { return }
                self.finish(success, reason, success ? nil : "Authentication did not complete")
            }
        }
    }

    nonisolated private static func reason(_ error: LAError) -> String {
        switch error.code {
        case .userCancel: "cancelled"
        case .userFallback: "user-fallback"
        case .biometryLockout: "lockout"
        case .biometryNotEnrolled: "not-enrolled"
        case .biometryNotAvailable: "not-available"
        case .passcodeNotSet: "passcode-not-set"
        case .systemCancel, .appCancel: "system-cancelled"
        default: "failed"
        }
    }

    private func finish(_ success: Bool, _ reason: String, _ message: String?) {
        guard let current = active else { return }
        active = nil
        current.context.invalidate()
        current.request.resolveJSON(Authentication(authenticated: success, reason: success ? nil : reason, message: message))
    }

    public func detached() { finish(false, "system-cancelled", "The authentication window closed") }
    public func uninstalled() { detached() }
}
