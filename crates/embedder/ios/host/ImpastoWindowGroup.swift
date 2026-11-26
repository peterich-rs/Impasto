#if SWIFT_PACKAGE
import CImpastoIOS
#endif
import Foundation

/// Shares WindowState between hosts that explicitly retain the same group.
public final class ImpastoWindowGroup: Sendable {
    private let address: UInt

    public init() {
        address = UInt(bitPattern: impasto_ios_window_group_new())
    }

    var handle: OpaquePointer? { OpaquePointer(bitPattern: address) }

    deinit {
        impasto_ios_window_group_free(handle)
    }
}
