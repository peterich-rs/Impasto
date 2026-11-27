import Foundation
import ImpastoHost

public enum PermissionStatus: String {
    case granted, denied, restricted, limited, permanentlyDenied, provisional
}

public enum PermissionServiceStatus: String {
    case enabled, disabled, notApplicable
}

public struct NotificationOptions: Decodable {
    public let alert: Bool?
    public let badge: Bool?
    public let sound: Bool?
    public let provisional: Bool?
    public let criticalAlert: Bool?
}

@MainActor
public struct PermissionStrategy {
    public let status: () async throws -> PermissionStatus
    public let request: (NotificationOptions?) async throws -> PermissionStatus
    public let service: () async throws -> PermissionServiceStatus
    public let picker: ((ImpastoHost) async throws -> Bool)?

    public init(
        status: @escaping () async throws -> PermissionStatus,
        request: ((NotificationOptions?) async throws -> PermissionStatus)? = nil,
        service: @escaping () async throws -> PermissionServiceStatus = { .notApplicable },
        picker: ((ImpastoHost) async throws -> Bool)? = nil
    ) {
        self.status = status
        self.request = request ?? { _ in try await status() }
        self.service = service
        self.picker = picker
    }
}

@MainActor
public protocol PermissionRegistrar {
    static func strategies() -> [String: PermissionStrategy]
}

public struct PermissionFailure: Error, Encodable {
    public let code: String
    public let message: String

    public init(_ code: String, _ message: String) {
        self.code = code
        self.message = message
    }

    public static func requireUsage(_ key: String) throws {
        guard let value = Bundle.main.object(forInfoDictionaryKey: key) as? String,
              !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw Self("missing-config", "Configure a non-empty \(key) before requesting this permission")
        }
    }

    static func encode(_ error: Error) -> String {
        let failure = (error as? Self) ?? (error is CancellationError
            ? Self("cancelled", "Permission request cancelled with host")
            : Self("platform", error.localizedDescription))
        guard let bytes = try? JSONEncoder().encode(failure) else { return failure.message }
        return String(decoding: bytes, as: UTF8.self)
    }
}
