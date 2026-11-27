#if SWIFT_PACKAGE
import CImpastoIOS
#endif
import QuartzCore
import UIKit
import os.signpost

private let renderLog = OSLog(subsystem: "dev.impasto", category: .pointsOfInterest)

/// One script session and the UIKit view currently attached to it.
/// Keep the host to move its script session between view controllers.
@MainActor
public final class ImpastoHost {
    public enum State: Equatable {
        case idle
        case preparing
        case ready
        case failed(String)
        case closing
        case closed
    }

    public enum FirstFrame {
        case presented
        case submitted
    }

    public enum SnapshotError: Error {
        case unavailable
        case invalidSize
        case captureFailed
    }

    public enum PreparationError: Error {
        case failed(String)
        case closed
    }

    public private(set) var state: State = .idle
    public var onStateChanged: ((State) -> Void)?
    public var onFirstFrame: ((FirstFrame) -> Void)?
    public var onRestorationChanged: (() -> Void)?

    private let resources: URL
    private let initialURL: URL?
    private let restoredState: String?
    private let windowGroup: ImpastoWindowGroup?
    private var plugins: [any ImpastoPlugin] = []
    private var installedPlugins: [any ImpastoPlugin] = []
    private var attachedPlugins: [any ImpastoPlugin] = []
    private var pluginController: ObjectIdentifier?
    private lazy var externalTextures = ExternalTextureRegistry { [weak self] in self?.connection.handle }
    private let connection = SessionConnection()
    private var preparationWaiters: [UUID: CheckedContinuation<Void, any Error>] = [:]
    private var sessionGeneration: UInt64 = 0
    private weak var controller: ImpastoViewController?
    public var viewController: ImpastoViewController? { controller }
    private weak var display: UIScreen?
    private var displayID: Int32 = 0
    private var refreshRate: Int?
    private var surface: Surface?
    private var retiring: Surface?
    private var retirementPollScheduled = false
    private var lifecycle: Lifecycle?
    private var focused: Bool?
    private(set) var backEnabled = false
    private let platformViews = ImpastoPlatformViews()

    private lazy var frameClock = FrameClock { [weak self] token, timestamp, target in
        guard let self, let handle = connection.handle, surface != nil else { return }
        impasto_ios_frame(handle, token, timestamp, target)
    }

    public init(
        resources: URL, initialURL: URL? = nil,
        restoredState: String? = nil, windowGroup: ImpastoWindowGroup? = nil
    ) {
        self.resources = resources
        self.initialURL = initialURL
        self.restoredState = restoredState
        self.windowGroup = windowGroup
    }

    public func restorationSnapshot() -> String? {
        guard let handle = connection.handle else { return nil }
        var bytes = [UInt8](repeating: 0, count: 32 * 1024)
        let count = bytes.withUnsafeMutableBufferPointer {
            impasto_ios_restoration_copy(handle, $0.baseAddress, $0.count)
        }
        guard count > 0 && count <= bytes.count else { return nil }
        return String(bytes: bytes.prefix(count), encoding: .utf8)
    }

    /// Captures the latest committed foreground frame as an sRGB image at drawable size.
    /// Includes composited platform views and video, within this controller's bounds.
    public func captureSnapshot() throws -> UIImage {
        guard state == .ready, let surface, !surface.waitingForFrame,
              let controller, let content = controller.viewIfLoaded,
              content.window != nil, !controller.isShieldingSnapshot,
              controller.presentation == .resumed || controller.presentation == .inactive
        else { throw SnapshotError.unavailable }

        let bounds = content.bounds
        let size = surface.layer.drawableSize
        guard bounds.width > 0, bounds.height > 0,
              size.width > 0, size.height > 0, size.width * size.height <= 16 * 1024 * 1024
        else { throw SnapshotError.invalidSize }
        guard let srgb = CGColorSpace(name: CGColorSpace.sRGB),
              let bitmap = CGContext(data: nil, width: Int(size.width), height: Int(size.height),
                                     bitsPerComponent: 8, bytesPerRow: 0, space: srgb,
                                     bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { throw SnapshotError.captureFailed }

        bitmap.translateBy(x: 0, y: size.height)
        bitmap.scaleBy(x: size.width / bounds.width, y: -size.height / bounds.height)
        UIGraphicsPushContext(bitmap)
        let captured = content.drawHierarchy(in: CGRect(origin: .zero, size: bounds.size), afterScreenUpdates: true)
        UIGraphicsPopContext()
        guard self.surface?.id == surface.id, controller === self.controller,
              surface.layer.drawableSize == size, content.bounds == bounds else { throw SnapshotError.unavailable }
        guard captured, let image = bitmap.makeImage() else { throw SnapshotError.captureFailed }
        return UIImage(cgImage: image, scale: CGFloat(surface.metrics.scale), orientation: .up)
    }

    /// Registers a producer to match the name of an ExternalTexture node.
    public func makeExternalTexture(named name: String) -> ImpastoExternalTexture? {
        guard state != .closed, state != .closing else { return nil }
        return externalTextures.register(name)
    }

    public func unregisterExternalTexture(named name: String) {
        externalTextures.unregister(name)
    }

    /// Presents an app-owned file through the current scene's system share sheet.
    /// Registers a native plugin before starting the host. Each host assigns
    /// one plugin per identifier, in registration order.
    @discardableResult
    public func registerPlugin(_ plugin: any ImpastoPlugin) -> Bool {
        guard state == .idle, !plugin.identifier.isEmpty,
              !plugins.contains(where: { $0.identifier == plugin.identifier })
        else { return false }
        plugins.append(plugin)
        return true
    }

    public func makeViewController() -> ImpastoViewController {
        ImpastoViewController(host: self)
    }

    /// Registers the native view factory for a kind used by the script's PlatformView.
    /// The closure runs on the main actor and returns a fresh unattached UIView for each node ID.
    @discardableResult
    public func registerPlatformView(_ kind: String, factory: @escaping (UInt64) -> UIView) -> Bool {
        guard state != .closing, state != .closed else { return false }
        guard platformViews.register(kind, factory: factory) else { return false }
        if let handle = connection.handle { impasto_ios_invalidate(handle) }
        return true
    }

    public func unregisterPlatformView(_ kind: String) {
        platformViews.unregister(kind)
        if let handle = connection.handle { impasto_ios_invalidate(handle) }
    }

    func accessibilityView(for id: UInt64) -> UIView? {
        platformViews.accessibilityView(for: id)
    }

    /// Starts preparing the session. UIKit stays responsive during preparation.
    public func prepare() {
        startPreparation()
    }

    /// Waits for the session to become ready. Cancelling the task stops this wait.
    /// Call close() to end the session and all other waits.
    public func prepare() async throws {
        let id = UUID()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
                if Task.isCancelled {
                    continuation.resume(throwing: CancellationError())
                } else if let result = preparationResult(state) {
                    continuation.resume(with: result)
                } else {
                    preparationWaiters[id] = continuation
                    startPreparation()
                }
            }
        } onCancel: {
            Task { @MainActor [weak self] in
                self?.preparationWaiters.removeValue(forKey: id)?.resume(throwing: CancellationError())
            }
        }
    }

    private func startPreparation() {
        guard state == .idle else { return }
        guard resources.isFileURL else {
            transition(.failed("Invalid local resource directory"))
            return
        }

        transition(.preparing)
        guard state == .preparing else { return }
        for plugin in plugins {
            guard state == .preparing else { return }
            installedPlugins.append(plugin)
            plugin.installed(in: self)
        }
        guard state == .preparing else { return }
        if let controller { attachPlugins(to: controller) }
        let acceptedURL = initialURL.flatMap { url in
            routeToPlugin(url) ? nil : url
        }
        guard state == .preparing else { return }
        sessionGeneration &+= 1
        let context = Unmanaged.passRetained(CallbackContext(self, generation: sessionGeneration)).toOpaque()
        connection.handle = resources.path.withCString { root in
            withOptionalCString(acceptedURL?.absoluteString) { initial in
                withOptionalCString(restoredState) { restored in
                    impasto_ios_start_restored(
                        root, initial, restored, windowGroup?.handle,
                        context, receiveImpastoEvent, releaseImpastoContext
                    )
                }
            }
        }
        guard connection.handle != nil else {
            finishPlugins()
            guard state == .preparing else { return }
            transition(.failed("Unable to start the Impasto session"))
            return
        }
    }

    /// Starts a new session after a startup or rendering failure.
    public func retry() {
        guard case .failed = state, connection.handle == nil else { return }
        retiring = nil
        lifecycle = nil
        focused = nil
        backEnabled = false
        transition(.idle)
        if let controller {
            inspect(controller)
        } else {
            prepare()
        }
    }

    public func open(_ url: URL) {
        guard let handle = connection.handle, !routeToPlugin(url),
              state == .ready || state == .preparing, connection.handle == handle else { return }
        url.absoluteString.withCString { impasto_ios_open_url(handle, $0) }
    }

    public func close() {
        guard state != .closed, state != .closing else { return }
        externalTextures.close()
        controller?.cancelPointers()
        controller?.clearPlatformInput()
        platformViews.detach()
        frameClock.disconnect()
        let closesAsynchronously = connection.handle != nil
        transition(closesAsynchronously ? .closing : .closed)
        finishPlugins(permanently: true)
        connection.close()
    }

    /// Separates the view from its script session so another view can attach.
    public func detach() {
        guard let controller else { return }
        controller.stopAttachment()
        dropAttachment()
    }

    /// Makes this controller eligible to take over the session as soon as it is visible.
    public func attach(to candidate: ImpastoViewController) {
        guard candidate.host === self else { return }
        if controller !== candidate {
            detach()
        }
        candidate.allowAttachment()
        inspect(candidate)
    }

    private func dropAttachment() {
        controller?.cancelPointers()
        setLifecycle(.detached, focused: false)
        retireSurface()
        controller = nil
        detachPlugins()
    }

    func inspect(_ candidate: ImpastoViewController) {
        guard state != .closed, state != .closing, candidate.allowsAttachment else { return }
        if controller !== candidate {
            // The visible controller retains its session until it goes away or is detached.
            guard candidate.isVisible, candidate.drawableView.window != nil,
                  controller?.isVisible != true
            else { return }
            dropAttachment()
            controller = candidate
            candidate.showState(state)
            if state == .ready { attachPlugins(to: candidate) }
        }

        guard controller === candidate, state != .closed, state != .closing else { return }
        let view = candidate.drawableView
        let presentation = candidate.presentation
        if presentation == .detached || presentation == .hidden || presentation == .paused {
            candidate.cancelPointers()
        }

        if !candidate.hasWindowFocus { candidate.cancelBackGesture() }
        setLifecycle(presentation, focused: candidate.hasWindowFocus)
        guard controller === candidate, state != .closed, state != .closing else { return }
        candidate.textInput.setFocusAllowed(candidate.hasWindowFocus)
        guard presentation == .resumed || presentation == .inactive,
              let scene = view.window?.windowScene,
              let metrics = ViewMetrics(view)
        else {
            candidate.cancelPointers()
            retireSurface()
            if presentation == .detached {
                controller = nil
                detachPlugins()
            }
            return
        }

        prepare()
        guard state == .ready, let handle = connection.handle else { return }
        guard retiring == nil else { return }
        if display !== scene.screen {
            retireSurface()
            guard retiring == nil else { return }
            display = scene.screen
            displayID = DisplayIdentifiers.identifier(for: scene.screen)
            frameClock.bind(to: scene)
            refreshRate = nil
        }
        let rate = scene.screen.maximumFramesPerSecond
        if refreshRate != rate {
            refreshRate = rate
            frameClock.updateMaximumRate(rate)
            impasto_ios_display(handle, displayID, Float(rate))
        }

        if let surface {
            if surface.metrics != metrics {
                metrics.apply(to: view.metalLayer)
                platformViews.updateDrawableSizes()
                impasto_ios_resize(
                    handle, surface.id, metrics.width, metrics.height, metrics.scale,
                    metrics.left, metrics.top, metrics.right, metrics.bottom
                )
                self.surface?.metrics = metrics
            }
            return
        }

        guard view.metalLayer.device != nil else {
            transition(.failed("Metal is unavailable on this device"))
            connection.close()
            return
        }
        metrics.apply(to: view.metalLayer)
        let layerPointer = Unmanaged.passUnretained(view.metalLayer).toOpaque()
        let id = impasto_ios_attach(
            handle, layerPointer, metrics.width, metrics.height, metrics.scale,
            metrics.left, metrics.top, metrics.right, metrics.bottom
        )
        guard id != 0 else {
            transition(.failed("Unable to attach the Metal surface"))
            connection.close()
            return
        }

        surface = Surface(id: id, layer: view.metalLayer, metrics: metrics)
        externalTextures.setActive(true)
        platformViews.attach(over: view)
        candidate.showWaitingForFrame()
        impasto_ios_semantics_enabled(handle, true, true)
        candidate.updateKeyboardGeometry()
        updateSettings(candidate)
        impasto_ios_invalidate(handle)
    }

    func updateSettings(_ candidate: ImpastoViewController) {
        guard controller === candidate, let handle = connection.handle else { return }
        let traits = candidate.traitCollection
        let font = UIFont.preferredFont(forTextStyle: .body, compatibleWith: traits)
        let standard = UIFont.preferredFont(
            forTextStyle: .body,
            compatibleWith: UITraitCollection(preferredContentSizeCategory: .large)
        )
        (Locale.preferredLanguages.first ?? Locale.current.identifier).withCString { locale in
            impasto_ios_settings(
                handle, locale, Float(font.pointSize / standard.pointSize),
                traits.userInterfaceStyle == .dark, UIAccessibility.isReduceMotionEnabled,
                UIAccessibility.isBoldTextEnabled, traits.accessibilityContrast == .high
            )
        }
    }

    func handleMemoryWarning() {
        guard let handle = connection.handle else { return }
        impasto_ios_memory_warning(handle)
        for plugin in installedPlugins { plugin.handleMemoryWarning() }
    }

    fileprivate func receive(_ kind: UInt32, value: UInt64, message: String, generation: UInt64) {
        guard sessionGeneration == generation else { return }
        switch Int(kind) {
        case IMPASTO_IOS_EVENT_READY:
            guard state == .preparing else { return }
            transition(.ready)
#if DEBUG
            ImpastoDevelopmentSession.current?.attach(owner: self) { [weak self] revision, source in
                guard let self, state == .ready, let handle = connection.handle else { return false }
                return source.withUnsafeBufferPointer {
                    impasto_ios_update_source(handle, revision, $0.baseAddress, $0.count)
                }
            }
#endif
            if let controller {
                attachPlugins(to: controller)
                inspect(controller)
            }
        case IMPASTO_IOS_EVENT_FAILED:
            if state != .closing && state != .closed {
                controller?.cancelPointers()
                retireSurface()
                transition(.failed(message))
                finishPlugins()
                connection.close()
            }
        case IMPASTO_IOS_EVENT_IME:
            guard state == .ready, surface?.id == value, let controller,
                  let update = try? JSONDecoder().decode(ImpastoEditingState.self, from: Data(message.utf8))
            else { return }
            controller.textInput.apply(update)
            controller.accessibilityBridge.refresh()
        case IMPASTO_IOS_EVENT_SEMANTICS:
            guard state == .ready, surface?.id == value,
                  let snapshot = try? JSONDecoder().decode([ImpastoSemanticsNode].self, from: Data(message.utf8))
            else { return }
            controller?.accessibilityBridge.apply(snapshot)
        case IMPASTO_IOS_EVENT_PLATFORM_VIEWS:
            guard state == .ready, surface?.id == value,
                  let placements = try? JSONDecoder().decode([ImpastoPlatformViewPlacement].self, from: Data(message.utf8))
            else { return }
            platformViews.commit(placements)
            controller?.accessibilityBridge.refresh()
        case IMPASTO_IOS_EVENT_COMPOSITOR_LAYERS:
            guard state == .ready, surface?.id == value, let handle = connection.handle,
                  let request = try? JSONDecoder().decode(ImpastoCompositorLayers.self, from: Data(message.utf8))
            else { return }
            let addresses = platformViews.setLayerCount(request.count)
            _ = addresses.withUnsafeBufferPointer { layers in
                impasto_ios_compositor_layers(handle, value, layers.baseAddress, layers.count)
            }
        case IMPASTO_IOS_EVENT_COMPOSED_FRAME:
            guard let handle = connection.handle,
                  let frame = try? JSONDecoder().decode(ImpastoComposedFrame.self, from: Data(message.utf8))
            else { return }
            let submitted = state == .ready && surface?.id == value && platformViews.commit(frame)
            if submitted {
                controller?.accessibilityBridge.refresh()
                os_signpost(.event, log: renderLog, name: "Composed Frame", "surface=%llu frame=%llu", value, frame.serial)
            }
            if connection.handle == handle {
                impasto_ios_compositor_finish(handle, value, frame.serial, submitted)
            }
        case IMPASTO_IOS_EVENT_BACK_ENABLED:
            backEnabled = value != 0
        case IMPASTO_IOS_EVENT_FRAME_REQUESTED:
            if state == .ready && surface != nil {
                frameClock.request(value, preferredRate: Float(message))
            }
        case IMPASTO_IOS_EVENT_FRAME_CANCELLED:
            frameClock.cancel(value)
        case IMPASTO_IOS_EVENT_FRAME_SUBMITTED:
            waitForPresentation(surfaceID: value)
        case IMPASTO_IOS_EVENT_FRAME_PRESENTED:
            finishFirstFrame(surfaceID: value, result: .presented)
        case IMPASTO_IOS_EVENT_SERVICE_REQUEST:
            receiveService(value, message: message)
        case IMPASTO_IOS_EVENT_RESTORATION_CHANGED:
            onRestorationChanged?()
        case IMPASTO_IOS_EVENT_BACK_UNHANDLED:
            if state == .ready, surface?.id == value {
                controller?.returnToNativeContainer()
            }
#if DEBUG
        case IMPASTO_IOS_EVENT_SOURCE_UPDATED:
            ImpastoDevelopmentSession.current?.complete(owner: self, revision: value, error: message)
#endif
        case IMPASTO_IOS_EVENT_CLOSED:
#if DEBUG
            ImpastoDevelopmentSession.current?.detach(owner: self)
#endif
            externalTextures.close()
            controller?.clearPlatformInput()
            platformViews.detach()
            frameClock.disconnect()
            surface = nil
            retiring = nil
            if state == .closing {
                transition(.closed)
            } else if state == .ready || state == .preparing {
                transition(.failed("The Impasto session ended unexpectedly"))
                finishPlugins()
                connection.close()
            }
        default:
            break
        }
    }

    private func routeToPlugin(_ url: URL) -> Bool {
        for plugin in installedPlugins {
            guard state == .ready || state == .preparing else { break }
            if plugin.open(url) { return true }
        }
        return false
    }

    private func attachPlugins(to controller: ImpastoViewController) {
        let owner = ObjectIdentifier(controller)
        guard pluginController != owner else { return }
        detachPlugins()
        guard self.controller === controller, state == .ready || state == .preparing else { return }
        pluginController = owner
        for plugin in installedPlugins {
            guard pluginController == owner else { return }
            attachedPlugins.append(plugin)
            plugin.attached(to: controller)
        }
    }

    private func detachPlugins() {
        pluginController = nil
        let attached = attachedPlugins
        attachedPlugins.removeAll()
        for plugin in attached { plugin.detached() }
    }

    private func finishPlugins(permanently: Bool = false) {
        let installed = installedPlugins
        installedPlugins.removeAll()
        if permanently { plugins.removeAll() }
        detachPlugins()
        for plugin in installed { plugin.uninstalled() }
    }

    @discardableResult
    public func emitPluginEvent(topic: String, payload: String) -> Bool {
        guard let handle = connection.handle else { return false }
        return topic.withCString { topic in
            payload.withCString { impasto_ios_service_event(handle, topic, $0) }
        }
    }

    private func receiveService(_ id: UInt64, message: String) {
        struct Payload: Decodable {
            let operation: String
            let payload: String
        }
        guard let handle = connection.handle,
              let payload = try? JSONDecoder().decode(Payload.self, from: Data(message.utf8))
        else { return }
        let request = ImpastoNativeRequest(
            id: id, operation: payload.operation, payload: payload.payload,
            pending: { [weak self] in
                self?.connection.handle == handle && impasto_ios_service_pending(handle, id)
            },
            finish: { [weak self] status, payload in
                guard self?.connection.handle == handle else { return false }
                return withOptionalCString(payload) {
                    impasto_ios_service_result(handle, id, status, $0)
                }
            }
        )
        guard request.isPending else { return }
        if !installedPlugins.contains(where: { $0.handle(request) }) {
            request.fail("No platform plugin handles \(payload.operation)")
        }
    }

    func pointer(
        _ candidate: ImpastoViewController, _ sample: PointerSample,
        history: [ImpastoIOSPointerPoint] = []
    ) -> Bool {
        guard controller === candidate, let handle = connection.handle, let surface else { return false }
        if sample.phase != .cancel && lifecycle != .resumed { return false }
        if !history.isEmpty {
            let point = ImpastoIOSPointerPoint(
                x: Float(sample.point.x), y: Float(sample.point.y), seconds: sample.time
            )
            history.withUnsafeBufferPointer { positions in
                impasto_ios_pointer_move(
                    handle, surface.id, sample.id, sample.kind, point, sample.buttons, sample.modifiers,
                    sample.pencil?.force ?? 0, sample.pencil?.radius ?? 0,
                    sample.pencil?.altitude ?? 0, sample.pencil?.azimuth ?? 0,
                    positions.baseAddress, positions.count
                )
            }
        } else if let pencil = sample.pencil {
            impasto_ios_pen(
                handle, surface.id, sample.id, sample.phase.rawValue,
                Float(sample.point.x), Float(sample.point.y), sample.time,
                pencil.force, pencil.radius, pencil.altitude, pencil.azimuth, sample.modifiers
            )
        } else {
            impasto_ios_pointer(
                handle, sample.id, sample.kind, sample.phase.rawValue,
                Float(sample.point.x), Float(sample.point.y), sample.time,
                sample.button, sample.buttons, sample.modifiers,
                Float(sample.delta.x), Float(sample.delta.y), sample.zoom
            )
        }
        return true
    }

    func key(_ candidate: ImpastoViewController, name: String, modifiers: UInt32) -> Bool {
        guard controller === candidate, candidate.hasWindowFocus,
              let handle = connection.handle, let surface else { return false }
        name.withCString { impasto_ios_key(handle, surface.id, $0, modifiers) }
        return true
    }

    func replaceEditing(
        _ candidate: ImpastoViewController, client: UInt64, sequence: UInt64,
        snapshot: ImpastoTextSnapshot
    ) {
        guard controller === candidate, let handle = connection.handle, let surface else { return }
        let bytes = Data(snapshot.text.utf8)
        bytes.withUnsafeBytes { text in
            impasto_ios_ime_replace(
                handle, surface.id, client, sequence,
                text.bindMemory(to: UInt8.self).baseAddress, text.count,
                snapshot.start, snapshot.end, snapshot.composeStart, snapshot.composeEnd
            )
        }
    }

    func dismissEditing(_ candidate: ImpastoViewController, client: UInt64) {
        guard controller === candidate, let handle = connection.handle, let surface else { return }
        impasto_ios_ime_dismiss(handle, surface.id, client)
    }

    func updateKeyboard(_ candidate: ImpastoViewController, bottom: CGFloat, occlusion: CGRect) {
        guard controller === candidate, let handle = connection.handle, let surface,
              surface.keyboardBottom != bottom || surface.keyboardOcclusion != occlusion
        else { return }
        self.surface?.keyboardBottom = bottom
        self.surface?.keyboardOcclusion = occlusion
        impasto_ios_keyboard_inset(handle, surface.id, Float(bottom))
        impasto_ios_keyboard_occlusion(
            handle, surface.id, Float(occlusion.minX), Float(occlusion.minY),
            Float(occlusion.width), Float(occlusion.height)
        )
    }

    func semanticsAction(
        _ candidate: ImpastoViewController, node: UInt64, action: ImpastoSemanticsAction,
        dx: CGFloat = 0, dy: CGFloat = 0, start: UInt32 = 0, end: UInt32 = 0, text: String = ""
    ) -> Bool {
        guard controller === candidate, candidate.hasWindowFocus,
              let handle = connection.handle, let surface else { return false }
        let bytes = Data(text.utf8)
        bytes.withUnsafeBytes { text in
            impasto_ios_semantics_action(
                handle, surface.id, node, action.rawValue, Float(dx), Float(dy), start, end,
                text.bindMemory(to: UInt8.self).baseAddress, text.count
            )
        }
        return true
    }

    func back(_ candidate: ImpastoViewController, phase: UInt32, progress: Float, fromRight: Bool) -> Bool {
        guard controller === candidate, phase == 2 || candidate.hasWindowFocus,
              let handle = connection.handle, let surface else { return false }
        impasto_ios_back(handle, surface.id, phase, progress, fromRight)
        return true
    }

    private func transition(_ state: State) {
        self.state = state
        if let result = preparationResult(state) {
            let waiters = preparationWaiters.values
            preparationWaiters.removeAll()
            for waiter in waiters { waiter.resume(with: result) }
        }
        controller?.showState(state)
        onStateChanged?(state)
    }

    private func preparationResult(_ state: State) -> Result<Void, any Error>? {
        switch state {
        case .idle, .preparing: nil
        case .ready: .success(())
        case .failed(let message): .failure(PreparationError.failed(message))
        case .closing, .closed: .failure(PreparationError.closed)
        }
    }

    private func setLifecycle(_ state: Lifecycle, focused: Bool) {
        // Remember state only once the running session has received it.
        guard let handle = connection.handle else { return }
        let changed = lifecycle != state
        if changed {
            lifecycle = state
            if state == .detached || state == .hidden || state == .paused {
            }
            impasto_ios_lifecycle(handle, state.rawValue)
        }
        if self.focused != focused {
            self.focused = focused
            impasto_ios_focus(handle, focused)
        }
        if changed {
            let activity: ImpastoSceneActivity
            switch state {
            case .detached: activity = .detached
            case .resumed: activity = .foreground
            case .inactive: activity = .inactive
            case .hidden: activity = .hidden
            case .paused: activity = .background
            }
            for plugin in installedPlugins { plugin.sceneActivityChanged(activity) }
        }
    }

    private func retireSurface() {
        externalTextures.setActive(false)
        frameClock.disconnect()
        display = nil
        guard let surface, let handle = connection.handle else { return }
        self.surface = nil
        platformViews.detach()
        controller?.clearPlatformInput()
        retiring = surface
        impasto_ios_detach(handle, surface.id)
        scheduleRetirementPoll()
    }

    private func pollRetirement() {
        guard let handle = connection.handle, let retiring else { return }
        if impasto_ios_surface_dropped(handle, retiring.id) {
            self.retiring = nil
            if let controller {
                inspect(controller)
            }
            return
        }
        scheduleRetirementPoll()
    }

    private func scheduleRetirementPoll() {
        guard !retirementPollScheduled else { return }
        retirementPollScheduled = true
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(16)) { [weak self] in
            guard let self else { return }
            retirementPollScheduled = false
            pollRetirement()
        }
    }

    private func waitForPresentation(surfaceID: UInt64) {
        guard surface?.id == surfaceID, surface?.waitingForFrame == true,
              surface?.submitted == false
        else { return }
        surface?.submitted = true
        // The simulator can accept a frame without reporting a presentation timestamp.
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(300)) { [weak self] in
            self?.finishFirstFrame(surfaceID: surfaceID, result: .submitted)
        }
    }

    private func finishFirstFrame(surfaceID: UInt64, result: FirstFrame) {
        guard state == .ready, surface?.id == surfaceID, surface?.waitingForFrame == true else { return }
        surface?.waitingForFrame = false
        os_signpost(.event, log: renderLog, name: "First Frame", "surface=%llu presented=%d", surfaceID, result == .presented ? 1 : 0)
        controller?.revealFrame()
        onFirstFrame?(result)
    }
}

@MainActor
private enum DisplayIdentifiers {
    static let identifiers = NSMapTable<UIScreen, NSNumber>.weakToStrongObjects()
    static var next: Int32 = 0

    static func identifier(for display: UIScreen) -> Int32 {
        if let existing = identifiers.object(forKey: display) { return existing.int32Value }
        next &+= 1
        identifiers.setObject(NSNumber(value: next), forKey: display)
        return next
    }
}

private struct Surface {
    let id: UInt64
    let layer: CAMetalLayer
    var metrics: ViewMetrics
    var keyboardBottom: CGFloat = 0
    var keyboardOcclusion = CGRect.zero
    var waitingForFrame = true
    var submitted = false
}

enum Lifecycle: UInt32 {
    case detached, resumed, inactive, hidden, paused
}

private struct ViewMetrics: Equatable {
    let width: Float
    let height: Float
    let scale: Float
    let left: Float
    let top: Float
    let right: Float
    let bottom: Float

    @MainActor init?(_ view: MetalView) {
        let scale = Float(view.traitCollection.displayScale)
        let width = Float(view.bounds.width)
        let height = Float(view.bounds.height)
        guard width.isFinite, height.isFinite, scale.isFinite, scale > 0,
              (width * scale).isFinite, (height * scale).isFinite,
              width * scale > 1, height * scale > 1
        else { return nil }
        self.width = width
        self.height = height
        self.scale = scale
        let insets = view.safeAreaInsets
        left = Float(insets.left)
        top = Float(insets.top)
        right = Float(insets.right)
        bottom = Float(insets.bottom)
    }

    @MainActor func apply(to layer: CAMetalLayer) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        layer.contentsScale = CGFloat(scale)
        layer.drawableSize = CGSize(
            width: CGFloat((width * scale).rounded()),
            height: CGFloat((height * scale).rounded())
        )
        CATransaction.commit()
    }
}

private final class SessionConnection {
    var handle: OpaquePointer?

    func close() {
        guard let handle else { return }
        self.handle = nil
        impasto_ios_close(handle)
    }

    deinit {
        // The final reference can leave scope outside the main actor.
        guard let handle else { return }
        let address = UInt(bitPattern: handle)
        DispatchQueue.main.async {
            impasto_ios_close(OpaquePointer(bitPattern: address))
        }
    }
}

private final class CallbackContext: @unchecked Sendable {
    @MainActor weak var host: ImpastoHost?
    let generation: UInt64

    @MainActor init(_ host: ImpastoHost, generation: UInt64) {
        self.host = host
        self.generation = generation
    }
}

private func receiveImpastoEvent(
    _ rawContext: UnsafeMutableRawPointer?,
    _ kind: UInt32,
    _ value: UInt64,
    _ bytes: UnsafePointer<UInt8>?,
    _ length: Int
) {
    guard let rawContext else { return }
    let context = Unmanaged<CallbackContext>.fromOpaque(rawContext).takeUnretainedValue()
    let message = bytes.map {
        String(decoding: UnsafeBufferPointer(start: $0, count: length), as: UTF8.self)
    } ?? ""
    // These updates only touch the current UIKit view. The session may
    // already be on the main actor when the UI and platform queues are shared.
    if Thread.isMainThread && (8...11).contains(kind) {
        MainActor.assumeIsolated {
            context.host?.receive(kind, value: value, message: message, generation: context.generation)
        }
        return
    }
    DispatchQueue.main.async {
        context.host?.receive(kind, value: value, message: message, generation: context.generation)
    }
}

private func releaseImpastoContext(_ rawContext: UnsafeMutableRawPointer?) {
    guard let rawContext else { return }
    Unmanaged<CallbackContext>.fromOpaque(rawContext).release()
}

@MainActor
private final class FrameClock: NSObject {
    private var lifetime: DisplayLinkLifetime?
    private var token: UInt64?
    private var maximumRate: Float = 60
    private var preferredRate: Float?
    private let pulse: (UInt64, CFTimeInterval, CFTimeInterval) -> Void

    init(pulse: @escaping (UInt64, CFTimeInterval, CFTimeInterval) -> Void) {
        self.pulse = pulse
    }

    func bind(to scene: UIWindowScene) {
        disconnect()
        let target = FrameClockTarget(self)
        let selector = #selector(FrameClockTarget.tick(_:))
        let link: CADisplayLink
        #if compiler(>=6.4)
        if #available(iOS 27.0, *) {
            link = scene.displayLink(target: target, selector: selector)
                ?? CADisplayLink(target: target, selector: selector)
        } else {
            link = scene.screen.displayLink(withTarget: target, selector: selector)
                ?? CADisplayLink(target: target, selector: selector)
        }
        #else
        link = scene.screen.displayLink(withTarget: target, selector: selector)
            ?? CADisplayLink(target: target, selector: selector)
        #endif
        link.isPaused = true
        link.add(to: .main, forMode: .common)
        lifetime = DisplayLinkLifetime(link)
    }

    func updateMaximumRate(_ rate: Int) {
        maximumRate = Float(rate)
        configureRate()
    }

    private func configureRate() {
        let rate = min(preferredRate ?? maximumRate, maximumRate)
        lifetime?.link?.preferredFrameRateRange = CAFrameRateRange(
            minimum: min(30, rate), maximum: rate, preferred: rate)
    }

    func request(_ token: UInt64, preferredRate: Float?) {
        self.token = token
        if self.preferredRate != preferredRate {
            self.preferredRate = preferredRate
            configureRate()
        }
        lifetime?.link?.isPaused = false
    }

    func cancel(_ request: UInt64) {
        guard token == request else { return }
        token = nil
        lifetime?.link?.isPaused = true
    }

    func disconnect() {
        token = nil
        lifetime?.invalidate()
        lifetime = nil
    }

    fileprivate func tick(_ link: CADisplayLink) {
        guard lifetime?.link === link else { return }
        link.isPaused = true
        guard let token else { return }
        self.token = nil
        os_signpost(.event, log: renderLog, name: "Display Link", "token=%llu period_ms=%.2f", token, (link.targetTimestamp - link.timestamp) * 1000)
        pulse(token, link.timestamp, link.targetTimestamp)
    }
}

@MainActor
private final class FrameClockTarget: NSObject {
    weak var clock: FrameClock?

    init(_ clock: FrameClock) {
        self.clock = clock
    }

    @objc func tick(_ link: CADisplayLink) {
        clock?.tick(link)
    }
}

/// Invalidates a display link even when the application releases the host without detaching it.
private final class DisplayLinkLifetime {
    private(set) var link: CADisplayLink?

    init(_ link: CADisplayLink) {
        self.link = link
    }

    @MainActor func invalidate() {
        link?.invalidate()
        link = nil
    }

    deinit {
        // Core Animation allows invalidation from any thread.
        link?.invalidate()
    }
}

private func withOptionalCString<T>(_ value: String?, _ body: (UnsafePointer<CChar>?) -> T) -> T {
    guard let value else { return body(nil) }
    return value.withCString(body)
}
