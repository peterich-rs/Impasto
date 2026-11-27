import CoreLocation
import Foundation
import ImpastoPermissionsCore
#if os(iOS)
import UIKit
#else
import AppKit
#endif

public enum LocationPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        let foreground = strategy(always: false)
        return ["location": foreground, "locationWhenInUse": foreground, "locationAlways": strategy(always: true)]
    }

    private static func strategy(always: Bool) -> PermissionStrategy {
        PermissionStrategy(status: {
            status(CLLocationManager().authorizationStatus, always: always)
        }, request: { _ in
            let prompt = LocationPrompt()
            #if os(iOS)
            try PermissionFailure.requireUsage("NSLocationWhenInUseUsageDescription")
            if always { try PermissionFailure.requireUsage("NSLocationAlwaysAndWhenInUseUsageDescription") }
            #else
            try PermissionFailure.requireUsage("NSLocationUsageDescription")
            #endif
            if prompt.authorization == .notDetermined {
                _ = try await prompt.request(always: false)
            }
            #if os(iOS)
            if always, prompt.authorization == .authorizedWhenInUse {
                // The authorization delegate can fire before the app regains focus.
                for _ in 0..<20 where !LocationPrompt.isActive {
                    try await Task.sleep(nanoseconds: 50_000_000)
                }
                _ = try await prompt.request(always: true)
            }
            #endif
            return status(prompt.authorization, always: always)
        }, service: {
            await withCheckedContinuation { continuation in
                DispatchQueue.global(qos: .userInitiated).async {
                    let state: PermissionServiceStatus = CLLocationManager.locationServicesEnabled() ? .enabled : .disabled
                    continuation.resume(returning: state)
                }
            }
        })
    }

    private static func status(_ authorization: CLAuthorizationStatus, always: Bool) -> PermissionStatus {
        switch authorization {
        case .notDetermined: return .denied
        case .denied: return .permanentlyDenied
        case .restricted: return .restricted
        case .authorizedAlways: return .granted
        case .authorizedWhenInUse:
            #if os(iOS)
            return always ? .limited : .granted
            #else
            return .granted
            #endif
        @unknown default: return .restricted
        }
    }
}

@MainActor
private final class LocationPrompt: NSObject, CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    private var completion: CheckedContinuation<CLAuthorizationStatus, any Error>?
    private var previous: CLAuthorizationStatus?
    var authorization: CLAuthorizationStatus { manager.authorizationStatus }

    static var isActive: Bool {
        #if os(iOS)
        UIApplication.shared.applicationState == .active
        #else
        NSApplication.shared.isActive
        #endif
    }

    override init() {
        super.init()
        manager.delegate = self
    }

    func request(always: Bool) async throws -> CLAuthorizationStatus {
        try Task.checkCancellation()
        guard Self.isActive else {
            throw PermissionFailure("host-unavailable", "Location authorization requires an active application")
        }
        previous = authorization
        #if os(iOS)
        let activation = NotificationCenter.default.addObserver(
            forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            guard always else { return }
            Task { @MainActor in
                // Let Core Location publish any changed authorization before reading it.
                try await Task.sleep(nanoseconds: 150_000_000)
                if let self { self.finish(.success(self.authorization)) }
            }
        }
        defer { NotificationCenter.default.removeObserver(activation) }
        #endif
        // Core Location omits a delegate callback if authorization is unchanged.
        // While a system prompt interrupts the app, keep this queue slot reserved.
        let fallback = Task { @MainActor in
            try await Task.sleep(nanoseconds: 60_000_000_000)
            while !Self.isActive { try await Task.sleep(nanoseconds: 500_000_000) }
            self.finish(.success(self.authorization))
        }
        defer { fallback.cancel() }

        let current = try await withCheckedThrowingContinuation { continuation in
            completion = continuation
            if always { manager.requestAlwaysAuthorization() }
            else { manager.requestWhenInUseAuthorization() }
        }
        try Task.checkCancellation()
        return current
    }

    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        Task { @MainActor in
            let current = self.authorization
            if current != self.previous {
                self.finish(.success(current))
            }
        }
    }

    private func finish(_ result: Result<CLAuthorizationStatus, any Error>) {
        let pending = completion
        completion = nil
        pending?.resume(with: result)
    }
}
