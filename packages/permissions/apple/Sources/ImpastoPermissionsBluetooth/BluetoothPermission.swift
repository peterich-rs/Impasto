import CoreBluetooth
import Foundation
import ImpastoPermissionsCore

public enum BluetoothPermissionRegistrar: PermissionRegistrar {
    public static func strategies() -> [String: PermissionStrategy] {
        let strategy = PermissionStrategy(status: { authorization() }, request: { _ in
            guard authorization() == .denied else { return authorization() }
            try PermissionFailure.requireUsage("NSBluetoothAlwaysUsageDescription")
            _ = try await BluetoothProbe().state()
            return authorization()
        }, service: {
            // Creating a manager while authorization is undecided can trigger the privacy prompt.
            guard CBManager.authorization == .allowedAlways else { return .notApplicable }
            switch try await BluetoothProbe().state() {
            case .poweredOn: return .enabled
            case .poweredOff: return .disabled
            default: return .notApplicable
            }
        })
        return [
            "bluetooth": strategy, "bluetoothConnect": strategy,
            "bluetoothScan": strategy, "bluetoothAdvertise": strategy,
        ]
    }

    private static func authorization() -> PermissionStatus {
        switch CBManager.authorization {
        case .notDetermined: return .denied
        case .allowedAlways: return .granted
        case .restricted: return .restricted
        case .denied: return .permanentlyDenied
        @unknown default: return .restricted
        }
    }
}

@MainActor
private final class BluetoothProbe: NSObject, CBCentralManagerDelegate {
    private var manager: CBCentralManager?
    private var completion: CheckedContinuation<CBManagerState, Never>?

    func state() async throws -> CBManagerState {
        try Task.checkCancellation()
        let state = await withCheckedContinuation { (continuation: CheckedContinuation<CBManagerState, Never>) in
            completion = continuation
            manager = CBCentralManager(delegate: self, queue: nil, options: [CBCentralManagerOptionShowPowerAlertKey: false])
        }
        try Task.checkCancellation()
        return state
    }

    nonisolated func centralManagerDidUpdateState(_ central: CBCentralManager) {
        let state = central.state
        guard state == .unsupported || (state != .unknown && state != .resetting && CBManager.authorization != .notDetermined) else { return }
        Task { @MainActor in self.finish(state) }
    }

    private func finish(_ result: CBManagerState) {
        let pending = completion
        completion = nil
        manager = nil
        pending?.resume(returning: result)
    }
}
