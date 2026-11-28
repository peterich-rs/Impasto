import Foundation
import ImpastoHost
#if os(iOS)
import AudioToolbox
import CoreHaptics
import UIKit
#else
import AppKit
#endif

@MainActor
public final class ImpastoHapticsPlugin: ImpastoPlugin {
    public let identifier = "@impasto/haptics"
    #if os(iOS)
    private var impacts: [String: UIImpactFeedbackGenerator] = [:]
    private let selection = UISelectionFeedbackGenerator()
    private let notification = UINotificationFeedbackGenerator()
    private var engine: CHHapticEngine?
    private var player: CHHapticPatternPlayer?
    #endif

    public static func register(with host: ImpastoHost) { host.registerPlugin(Self()) }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        guard request.operation.hasPrefix("\(identifier)/") else { return false }
        let action = String(request.operation.dropFirst(identifier.count + 1))
        do {
            #if os(iOS)
            let supported = CHHapticEngine.capabilitiesForHardware().supportsHaptics
            if action == "isSupported" { request.resolveJSON(supported); return true }
            guard supported else { request.resolve(); return true }
            switch action {
            case "impact": impact(try request.decodeJSON(String.self))
            case "selection": selection.prepare(); selection.selectionChanged()
            case "notification":
                let style = try request.decodeJSON(String.self)
                notification.prepare()
                notification.notificationOccurred(style == "error" ? .error : (style == "warning" ? .warning : .success))
            case "vibrate": AudioServicesPlaySystemSound(kSystemSoundID_Vibrate)
            case "vibratePattern": try vibrate(request.decodeJSON(Pattern.self))
            case "cancel": detached()
            default: return false
            }
            #else
            switch action {
            case "isSupported": request.resolveJSON(true); return true
            case "impact":
                let style = try request.decodeJSON(String.self)
                feedback(style == "light" || style == "soft" ? .alignment : .generic)
            case "selection": feedback(.alignment)
            case "notification": feedback(.levelChange)
            case "vibrate", "vibratePattern": feedback(.generic)
            case "cancel": break
            default: return false
            }
            #endif
            request.resolve()
        } catch {
            request.fail(code: "platform", message: error.localizedDescription)
        }
        return true
    }

    public func detached() {
        #if os(iOS)
        try? player?.stop(atTime: CHHapticTimeImmediate)
        player = nil
        engine?.stop(completionHandler: nil)
        engine = nil
        #endif
    }

    public func uninstalled() { detached() }

    #if os(iOS)
    private struct Pattern: Decodable { let timingsMs: [Int]; let amplitudes: [Int]? }

    private func impact(_ style: String) {
        let generator: UIImpactFeedbackGenerator
        if let existing = impacts[style] { generator = existing }
        else {
            let styles: [String: UIImpactFeedbackGenerator.FeedbackStyle] = [
                "light": .light, "medium": .medium, "heavy": .heavy, "soft": .soft, "rigid": .rigid,
            ]
            generator = UIImpactFeedbackGenerator(style: styles[style] ?? .medium)
            impacts[style] = generator
        }
        generator.prepare()
        generator.impactOccurred()
    }

    private func vibrate(_ pattern: Pattern) throws {
        var events: [CHHapticEvent] = []
        var time = 0.0
        for (index, duration) in pattern.timingsMs.enumerated() {
            let duration = Double(duration) / 1000
            let amplitude = pattern.amplitudes.flatMap { $0.indices.contains(index) ? $0[index] : nil }
                ?? (pattern.timingsMs.count == 1 || index % 2 == 1 ? 255 : 0)
            if amplitude > 0 && duration > 0 {
                // Core Haptics caps a single continuous event at 30 seconds.
                events.append(CHHapticEvent(eventType: .hapticContinuous,
                    parameters: [CHHapticEventParameter(parameterID: .hapticIntensity, value: Float(amplitude) / 255)],
                    relativeTime: time, duration: duration))
            }
            time += duration
        }
        guard !events.isEmpty else { return }
        let engine = try engine ?? CHHapticEngine()
        engine.isAutoShutdownEnabled = true
        try player?.stop(atTime: CHHapticTimeImmediate)
        try engine.start()
        let player = try engine.makePlayer(with: CHHapticPattern(events: events, parameters: []))
        try player.start(atTime: CHHapticTimeImmediate)
        self.engine = engine
        self.player = player
    }
    #else
    private func feedback(_ pattern: NSHapticFeedbackManager.FeedbackPattern) {
        NSHapticFeedbackManager.defaultPerformer.perform(pattern, performanceTime: .now)
    }
    #endif
}
