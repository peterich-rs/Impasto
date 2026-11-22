import AudioToolbox
import UIKit

/// UIKit policy for the view controller that presents an Impasto session.
public struct ImpastoSystemUI {
    public var statusBarStyle: UIStatusBarStyle?
    public var statusBarHidden: Bool?
    public var homeIndicatorAutoHidden: Bool?
    public var deferredSystemGestureEdges: UIRectEdge?
    public var supportedOrientations: UIInterfaceOrientationMask?

    public init(
        statusBarStyle: UIStatusBarStyle? = nil,
        statusBarHidden: Bool? = nil,
        homeIndicatorAutoHidden: Bool? = nil,
        deferredSystemGestureEdges: UIRectEdge? = nil,
        supportedOrientations: UIInterfaceOrientationMask? = nil
    ) {
        self.statusBarStyle = statusBarStyle
        self.statusBarHidden = statusBarHidden
        self.homeIndicatorAutoHidden = homeIndicatorAutoHidden
        self.deferredSystemGestureEdges = deferredSystemGestureEdges
        self.supportedOrientations = supportedOrientations
    }
}

@MainActor
public enum ImpastoFeedback {
    public static func selection() {
        UISelectionFeedbackGenerator().selectionChanged()
    }

    public static func impact(_ style: UIImpactFeedbackGenerator.FeedbackStyle) {
        UIImpactFeedbackGenerator(style: style).impactOccurred()
    }

    public static func notification(_ type: UINotificationFeedbackGenerator.FeedbackType) {
        UINotificationFeedbackGenerator().notificationOccurred(type)
    }

    public static func playSound(_ sound: SystemSoundID) {
        AudioServicesPlaySystemSound(sound)
    }
}

enum ImpastoGeometryError: LocalizedError {
    case unsupported
    case unattached

    var errorDescription: String? {
        switch self {
        case .unsupported: return "Scene geometry requests require iOS 16 or later."
        case .unattached: return "The Impasto controller has no window scene."
        }
    }
}
