import Metal
import QuartzCore
import UIKit

/// Embeds one Impasto session in a window, navigation stack, or parent controller.
@MainActor
public final class ImpastoViewController: UIViewController {
    let host: ImpastoHost
    let drawableView = MetalView()
    lazy var textInput = ImpastoTextInput(self)
    lazy var accessibilityBridge = ImpastoAccessibility(self)
    private var backGesture: (fromRight: Bool, progress: Float)?

    public var systemUI = ImpastoSystemUI() {
        didSet {
            setNeedsStatusBarAppearanceUpdate()
            setNeedsUpdateOfHomeIndicatorAutoHidden()
            setNeedsUpdateOfScreenEdgesDeferringSystemGestures()
            if #available(iOS 16.0, *) { setNeedsUpdateOfSupportedInterfaceOrientations() }
        }
    }

    public override var preferredStatusBarStyle: UIStatusBarStyle {
        systemUI.statusBarStyle ?? super.preferredStatusBarStyle
    }

    public override var prefersStatusBarHidden: Bool {
        systemUI.statusBarHidden ?? super.prefersStatusBarHidden
    }

    public override var prefersHomeIndicatorAutoHidden: Bool {
        systemUI.homeIndicatorAutoHidden ?? super.prefersHomeIndicatorAutoHidden
    }

    public override var preferredScreenEdgesDeferringSystemGestures: UIRectEdge {
        systemUI.deferredSystemGestureEdges ?? super.preferredScreenEdgesDeferringSystemGestures
    }

    public override var supportedInterfaceOrientations: UIInterfaceOrientationMask {
        systemUI.supportedOrientations ?? super.supportedInterfaceOrientations
    }

    /// Requests a rotation on the scene containing this view controller.
    public func requestInterfaceOrientations(
        _ orientations: UIInterfaceOrientationMask,
        onError: @escaping @Sendable (Error) -> Void
    ) {
        guard let scene = viewIfLoaded?.window?.windowScene else {
            onError(ImpastoGeometryError.unattached)
            return
        }
        if #available(iOS 16.0, *) {
            scene.requestGeometryUpdate(.iOS(interfaceOrientations: orientations), errorHandler: onError)
        } else {
            onError(ImpastoGeometryError.unsupported)
        }
    }
    private let statusView = UIView()
    private let statusLabel = UILabel()
    private let retryButton = UIButton(type: .system)
    private let appSwitcherShield = UIView()
    private weak var observedScene: UIWindowScene?
    private var sceneLifecycle: Lifecycle?
    private var hasRevealedFrame = false
    private var appearance: Bool?
    private(set) var allowsAttachment = true

    /// Covers this controller's content while its scene is inactive or in the background.
    public var hidesContentInAppSwitcher = false {
        didSet { updateAppSwitcherShield() }
    }

    var isShieldingSnapshot: Bool { !appSwitcherShield.isHidden }

    var isVisible: Bool {
        appearance ?? (drawableView.window != nil)
    }

    public init(host: ImpastoHost) {
        self.host = host
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    public required init?(coder: NSCoder) {
        fatalError("Use init(host:)")
    }

    public override func loadView() {
        view = UIView()
        view.backgroundColor = .systemBackground
        drawableView.owner = self
        textInput.attach(to: drawableView)
        let editMenu = UILongPressGestureRecognizer(target: self, action: #selector(handleEditMenu(_:)))
        editMenu.delegate = self
        editMenu.cancelsTouchesInView = false
        drawableView.addGestureRecognizer(editMenu)
        for edge in [UIRectEdge.left, .right] {
            let recognizer = UIScreenEdgePanGestureRecognizer(target: self, action: #selector(handleBackEdge(_:)))
            recognizer.edges = edge
            recognizer.delegate = self
            drawableView.addGestureRecognizer(recognizer)
        }
        drawableView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(drawableView)

        statusView.backgroundColor = .systemBackground
        statusView.translatesAutoresizingMaskIntoConstraints = false
        // Leave hit testing to the drawable view even while a frame is arriving.
        statusView.isUserInteractionEnabled = false
        view.addSubview(statusView)

        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        statusLabel.font = .preferredFont(forTextStyle: .body)
        statusLabel.adjustsFontForContentSizeCategory = true
        statusLabel.textAlignment = .center
        statusLabel.numberOfLines = 0
        statusLabel.textColor = .label
        statusLabel.isHidden = true
        statusView.addSubview(statusLabel)
        retryButton.translatesAutoresizingMaskIntoConstraints = false
        retryButton.setTitle(NSLocalizedString("Retry", comment: "Try starting the app again"), for: .normal)
        retryButton.addTarget(self, action: #selector(retry), for: .touchUpInside)
        retryButton.isHidden = true
        statusView.addSubview(retryButton)

        appSwitcherShield.backgroundColor = .systemBackground
        appSwitcherShield.translatesAutoresizingMaskIntoConstraints = false
        appSwitcherShield.isHidden = true
        appSwitcherShield.accessibilityElementsHidden = true
        view.addSubview(appSwitcherShield)

        NSLayoutConstraint.activate([
            drawableView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            drawableView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            drawableView.topAnchor.constraint(equalTo: view.topAnchor),
            drawableView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            statusView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            statusView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            statusView.topAnchor.constraint(equalTo: view.topAnchor),
            statusView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            statusLabel.leadingAnchor.constraint(equalTo: statusView.layoutMarginsGuide.leadingAnchor),
            statusLabel.trailingAnchor.constraint(equalTo: statusView.layoutMarginsGuide.trailingAnchor),
            statusLabel.centerYAnchor.constraint(equalTo: statusView.safeAreaLayoutGuide.centerYAnchor),
            retryButton.centerXAnchor.constraint(equalTo: statusView.centerXAnchor),
            retryButton.topAnchor.constraint(equalTo: statusLabel.bottomAnchor, constant: 16),
            appSwitcherShield.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            appSwitcherShield.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            appSwitcherShield.topAnchor.constraint(equalTo: view.topAnchor),
            appSwitcherShield.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        showState(host.state)
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        let notifications = NotificationCenter.default
        for name in [
            UIScene.didActivateNotification,
            UIScene.willDeactivateNotification,
            UIScene.willEnterForegroundNotification,
            UIScene.didEnterBackgroundNotification,
            UIScene.didDisconnectNotification,
        ] {
            notifications.addObserver(self, selector: #selector(sceneChanged(_:)), name: name, object: nil)
        }
        for name in [
            UIWindow.didBecomeKeyNotification, UIWindow.didResignKeyNotification,
            UIWindow.didBecomeVisibleNotification, UIWindow.didBecomeHiddenNotification,
        ] {
            notifications.addObserver(self, selector: #selector(windowFocusChanged(_:)), name: name, object: nil)
        }
        notifications.addObserver(
            self, selector: #selector(screenModeChanged(_:)), name: UIScreen.modeDidChangeNotification, object: nil
        )
        for name in [UIResponder.keyboardWillChangeFrameNotification, UIResponder.keyboardDidChangeFrameNotification] {
            notifications.addObserver(self, selector: #selector(keyboardChanged), name: name, object: nil)
        }
        for name in [
            UIAccessibility.reduceMotionStatusDidChangeNotification,
            UIAccessibility.boldTextStatusDidChangeNotification,
            UIContentSizeCategory.didChangeNotification,
            NSLocale.currentLocaleDidChangeNotification,
        ] {
            notifications.addObserver(self, selector: #selector(settingsChanged), name: name, object: nil)
        }
        if #available(iOS 17.0, *) {
            registerForTraitChanges([
                UITraitUserInterfaceStyle.self, UITraitAccessibilityContrast.self,
                UITraitDisplayScale.self, UITraitPreferredContentSizeCategory.self,
            ]) { (self: ImpastoViewController, _) in
                self.host.updateSettings(self)
                self.host.inspect(self)
            }
        }
    }

    public override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        allowsAttachment = true
        appearance = true
        host.inspect(self)
    }

    public override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        host.inspect(self)
        focusDefaultInput()
    }

    public override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        appearance = false
        host.inspect(self)
    }

    public override func traitCollectionDidChange(_ previousTraitCollection: UITraitCollection?) {
        super.traitCollectionDidChange(previousTraitCollection)
        if #unavailable(iOS 17.0) {
            host.updateSettings(self)
            host.inspect(self)
        }
    }

    public override func didReceiveMemoryWarning() {
        super.didReceiveMemoryWarning()
        host.handleMemoryWarning()
    }

    var presentation: Lifecycle {
        guard let scene = drawableView.window?.windowScene else { return .detached }
        if !isVisible || drawableView.window?.isHidden == true {
            return .hidden
        }
        if observedScene === scene, let sceneLifecycle {
            return sceneLifecycle
        }
        switch scene.activationState {
        case .foregroundActive: return .resumed
        case .foregroundInactive: return .inactive
        case .background: return .paused
        case .unattached: return .detached
        @unknown default: return .hidden
        }
    }

    var hasWindowFocus: Bool {
        presentation == .resumed && drawableView.window?.isKeyWindow == true
    }

    func surfaceWindowChanged() {
        let scene = drawableView.window?.windowScene
        if observedScene !== scene {
            observedScene = scene
            sceneLifecycle = nil
        }
        updateAppSwitcherShield()
        host.inspect(self)
    }

    func surfaceGeometryChanged() {
        host.inspect(self)
        updateKeyboardGeometry()
    }

    @objc private func keyboardChanged() {
        drawableView.setNeedsLayout()
        updateKeyboardGeometry()
    }

    func updateKeyboardGeometry() {
        guard drawableView.window != nil else { return }
        let frame = drawableView.keyboardLayoutGuide.layoutFrame.intersection(drawableView.bounds)
        let showing = !frame.isEmpty && frame.height > drawableView.safeAreaInsets.bottom + 1
        let docked = showing &&
            frame.maxY >= drawableView.bounds.maxY - 1 &&
            frame.width >= drawableView.bounds.width - 1
        host.updateKeyboard(self, bottom: docked ? frame.height : 0, occlusion: showing ? frame : .zero)
    }

    func clearPlatformInput() {
        cancelBackGesture()
        textInput.setFocusAllowed(false)
        textInput.clear()
        accessibilityBridge.clear()
        drawableView.resignFirstResponder()
    }

    func focusDefaultInput() {
        guard host.state == .ready, hasWindowFocus, !textInput.hasClient, !drawableView.isFirstResponder else { return }
        drawableView.becomeFirstResponder()
    }

    override public func accessibilityPerformEscape() -> Bool {
        if textInput.dismissFromPlatform() { return true }
        if host.backEnabled && host.back(self, phase: 3, progress: 1, fromRight: false) { return true }
        return super.accessibilityPerformEscape()
    }

    func cancelBackGesture() {
        guard let backGesture else { return }
        self.backGesture = nil
        _ = host.back(self, phase: 2, progress: backGesture.progress, fromRight: backGesture.fromRight)
    }

    @objc private func handleEditMenu(_ recognizer: UILongPressGestureRecognizer) {
        if recognizer.state == .began {
            textInput.showEditMenu(at: recognizer.location(in: drawableView))
        }
    }

    @objc private func handleBackEdge(_ recognizer: UIScreenEdgePanGestureRecognizer) {
        let fromRight = recognizer.edges == .right
        let sign: CGFloat = fromRight ? -1 : 1
        switch recognizer.state {
        case .began:
            if host.back(self, phase: 0, progress: 0, fromRight: fromRight) { backGesture = (fromRight, 0) }
        case .changed:
            guard backGesture?.fromRight == fromRight else { return }
            let width = max(drawableView.bounds.width, 1)
            let progress = Float(min(max(sign * recognizer.translation(in: drawableView).x / width, 0), 1))
            backGesture?.progress = progress
            _ = host.back(self, phase: 1, progress: progress, fromRight: fromRight)
        case .ended, .cancelled, .failed:
            guard let gesture = backGesture, gesture.fromRight == fromRight else { return }
            let progress = gesture.progress
            backGesture = nil
            let commit = recognizer.state == .ended &&
                (progress >= 0.33 || sign * recognizer.velocity(in: drawableView).x >= 550)
            _ = host.back(self, phase: commit ? 3 : 2, progress: progress, fromRight: fromRight)
        default:
            break
        }
    }

    func pointer(_ sample: PointerSample) -> Bool {
        host.pointer(self, sample)
    }

    func cancelPointers() {
        drawableView.cancelPointers()
    }

    func returnToNativeContainer() {
        if let navigationController, navigationController.topViewController === self,
           navigationController.viewControllers.count > 1 {
            navigationController.popViewController(animated: true)
        } else if presentingViewController != nil {
            dismiss(animated: true)
        }
    }

    func stopAttachment() {
        allowsAttachment = false
    }

    func allowAttachment() {
        allowsAttachment = true
    }

    func showState(_ state: ImpastoHost.State) {
        guard isViewLoaded else { return }
        if case .failed(let message) = state {
            statusLabel.text = message
            statusLabel.isHidden = false
            retryButton.isHidden = false
            statusView.isUserInteractionEnabled = true
            statusView.isHidden = false
        } else if state == .preparing {
            hasRevealedFrame = false
            showWaitingForFrame()
        }
    }

    @objc private func retry() {
        host.retry()
    }

    func showWaitingForFrame() {
        guard !hasRevealedFrame else { return }
        statusLabel.isHidden = true
        retryButton.isHidden = true
        statusView.isUserInteractionEnabled = false
        statusView.isHidden = false
    }

    func revealFrame() {
        hasRevealedFrame = true
        statusView.isHidden = true
        focusDefaultInput()
    }

    @objc private func sceneChanged(_ notification: Notification) {
        guard let scene = drawableView.window?.windowScene, notification.object as? UIScene === scene
        else { return }
        observedScene = scene
        switch notification.name {
        case UIScene.didActivateNotification:
            sceneLifecycle = .resumed
        case UIScene.willDeactivateNotification, UIScene.willEnterForegroundNotification:
            sceneLifecycle = .inactive
        case UIScene.didEnterBackgroundNotification:
            sceneLifecycle = .paused
        case UIScene.didDisconnectNotification:
            sceneLifecycle = .detached
        default:
            return
        }
        updateAppSwitcherShield()
        host.inspect(self)
    }

    private func updateAppSwitcherShield() {
        guard let view = viewIfLoaded else { return }
        let state = presentation
        let shields = hidesContentInAppSwitcher && (state == .inactive || state == .paused)
        appSwitcherShield.isHidden = !shields
        view.accessibilityElementsHidden = shields
    }

    @objc private func windowFocusChanged(_ notification: Notification) {
        guard let window = drawableView.window, notification.object as? UIWindow === window else { return }
        host.inspect(self)
    }

    @objc private func screenModeChanged(_ notification: Notification) {
        guard let screen = drawableView.window?.windowScene?.screen,
              notification.object as? UIScreen === screen
        else { return }
        host.inspect(self)
    }

    @objc private func settingsChanged() {
        host.updateSettings(self)
    }
}

extension ImpastoViewController: UIGestureRecognizerDelegate {
    public func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        if gestureRecognizer is UILongPressGestureRecognizer {
            return textInput.canShowEditMenu(at: gestureRecognizer.location(in: drawableView))
        }
        guard gestureRecognizer is UIScreenEdgePanGestureRecognizer else { return true }
        let usesUIKitBack = (navigationController?.viewControllers.count ?? 0) > 1 &&
            navigationController?.interactivePopGestureRecognizer?.isEnabled != false
        return host.backEnabled && hasWindowFocus && !usesUIKitBack && !textInput.hasClient
    }
}

final class ImpastoDrawableLayer: CAMetalLayer {
    private let observerLock = NSLock()
    private var drawableObserver: ((any CAMetalDrawable) -> Void)?

    @objc(impastoSetDrawableObserver:)
    func setDrawableObserver(_ observer: @escaping (any CAMetalDrawable) -> Void) {
        observerLock.withLock { drawableObserver = observer }
    }

    override func nextDrawable() -> (any CAMetalDrawable)? {
        guard let drawable = super.nextDrawable() else { return nil }
        let observer = observerLock.withLock { drawableObserver }
        observer?(drawable)
        return drawable
    }
}

@MainActor
final class MetalView: UIView {
    weak var owner: ImpastoViewController?
    private var pointers: [ObjectIdentifier: PointerSample] = [:]
    private var nextPointerID: UInt64 = 0
    private var routedPresses: Set<ObjectIdentifier> = []
    private let keyboardProbe = KeyboardLayoutProbe()
    private var lastHoverPoint: CGPoint?
    private var lastPinchScale: CGFloat = 1

    override class var layerClass: AnyClass { ImpastoDrawableLayer.self }
    override var canBecomeFirstResponder: Bool { true }

    var metalLayer: CAMetalLayer {
        // UIView creates the backing layer using layerClass above.
        layer as! CAMetalLayer
    }

    init() {
        super.init(frame: .zero)
        isOpaque = true
        isMultipleTouchEnabled = true
        metalLayer.device = MTLCreateSystemDefaultDevice()
        metalLayer.pixelFormat = .bgra8Unorm_srgb
        metalLayer.colorspace = CGColorSpace(name: CGColorSpace.sRGB)
        metalLayer.framebufferOnly = true
        metalLayer.allowsNextDrawableTimeout = true

        let hover = UIHoverGestureRecognizer(target: self, action: #selector(mouseHover(_:)))
        hover.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.indirectPointer.rawValue)]
        addGestureRecognizer(hover)
        if #available(iOS 16.4, *) {
            let pencil = UIHoverGestureRecognizer(target: self, action: #selector(pencilHover(_:)))
            pencil.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.pencil.rawValue)]
            addGestureRecognizer(pencil)
        }
        for (scrollTypes, kind) in [(UIScrollTypeMask.discrete, UInt32(1)), (.continuous, UInt32(3))] {
            let scroll = ImpastoScrollRecognizer(kind: kind, target: self, action: #selector(scrolled(_:)))
            scroll.allowedScrollTypesMask = scrollTypes
            scroll.allowedTouchTypes = []
            scroll.cancelsTouchesInView = false
            addGestureRecognizer(scroll)
        }
        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(pinched(_:)))
        addGestureRecognizer(pinch)

        keyboardLayoutGuide.followsUndockedKeyboard = true
        keyboardProbe.translatesAutoresizingMaskIntoConstraints = false
        keyboardProbe.isUserInteractionEnabled = false
        keyboardProbe.accessibilityElementsHidden = true
        keyboardProbe.onGeometryChanged = { [weak self] in self?.owner?.updateKeyboardGeometry() }
        addSubview(keyboardProbe)
        NSLayoutConstraint.activate([
            keyboardProbe.leadingAnchor.constraint(equalTo: keyboardLayoutGuide.leadingAnchor),
            keyboardProbe.trailingAnchor.constraint(equalTo: keyboardLayoutGuide.trailingAnchor),
            keyboardProbe.topAnchor.constraint(equalTo: keyboardLayoutGuide.topAnchor),
            keyboardProbe.bottomAnchor.constraint(equalTo: keyboardLayoutGuide.bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("Use init()")
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        owner?.surfaceWindowChanged()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        owner?.surfaceGeometryChanged()
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        owner?.surfaceGeometryChanged()
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        owner?.focusDefaultInput()
        sendTouches(touches, event: event, phase: .down)
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        sendTouches(touches, event: event, phase: .move)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        sendTouches(touches, event: event, phase: .up)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        sendTouches(touches, event: event, phase: .cancel)
    }

    override func touchesEstimatedPropertiesUpdated(_ touches: Set<UITouch>) {
        for touch in touches where touch.type == .pencil {
            let key = ObjectIdentifier(touch)
            guard let previous = pointers[key], previous.time <= touch.timestamp else { continue }
            let update = sample(touch, event: nil, id: previous.id, phase: .move, previous: previous)
            _ = owner?.pointer(update)
            pointers[key] = update
        }
    }

    override func accessibilityScroll(_ direction: UIAccessibilityScrollDirection) -> Bool {
        if owner?.accessibilityBridge.scroll(direction) == true { return true }
        return super.accessibilityScroll(direction)
    }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let forwarded = presses.filter { press in
            guard let owner, !owner.textInput.hasClient, let key = press.key,
                  let routed = ImpastoHardwareKey(key, editing: false),
                  owner.host.key(owner, name: routed.name, modifiers: routed.modifiers)
            else { return true }
            routedPresses.insert(ObjectIdentifier(press))
            return false
        }
        if !forwarded.isEmpty { super.pressesBegan(forwarded, with: event) }
    }

    override func pressesChanged(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let forwarded = presses.filter { !routedPresses.contains(ObjectIdentifier($0)) }
        if !forwarded.isEmpty { super.pressesChanged(forwarded, with: event) }
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let forwarded = presses.filter { routedPresses.remove(ObjectIdentifier($0)) == nil }
        if !forwarded.isEmpty { super.pressesEnded(forwarded, with: event) }
    }

    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let forwarded = presses.filter { routedPresses.remove(ObjectIdentifier($0)) == nil }
        if !forwarded.isEmpty { super.pressesCancelled(forwarded, with: event) }
    }

    @objc private func mouseHover(_ recognizer: UIHoverGestureRecognizer) {
        let point: CGPoint
        switch recognizer.state {
        case .began, .changed:
            point = recognizer.location(in: self)
            lastHoverPoint = point
        case .ended, .cancelled:
            point = CGPoint(x: bounds.minX - 1, y: bounds.minY - 1)
            lastHoverPoint = nil
        default: return
        }
        sendGesture(phase: .move, kind: 1, point: point, modifiers: recognizer.modifierFlags)
    }

    @available(iOS 16.4, *)
    @objc private func pencilHover(_ recognizer: UIHoverGestureRecognizer) {
        guard recognizer.state == .began || recognizer.state == .changed || recognizer.state == .ended else { return }
        let point = recognizer.state == .ended
            ? CGPoint(x: bounds.minX - 1, y: bounds.minY - 1) : recognizer.location(in: self)
        sendGesture(
            phase: .move, kind: 2, point: point, modifiers: recognizer.modifierFlags,
            pencil: PencilSample(
                force: 0, radius: 0, altitude: Float(recognizer.altitudeAngle),
                azimuth: Float(recognizer.azimuthAngle(in: self))
            )
        )
    }

    @objc private func scrolled(_ recognizer: ImpastoScrollRecognizer) {
        switch recognizer.state {
        case .began, .changed, .ended:
            let translation = recognizer.translation(in: self)
            recognizer.setTranslation(.zero, in: self)
            if translation != .zero {
                let point = recognizer.location(in: self)
                sendGesture(
                    phase: .wheel, kind: recognizer.kind,
                    point: bounds.contains(point) ? point : lastHoverPoint ?? point,
                    modifiers: recognizer.modifierFlags,
                    delta: CGPoint(x: -translation.x, y: -translation.y)
                )
            }
        default: break
        }
    }

    @objc private func pinched(_ recognizer: UIPinchGestureRecognizer) {
        if recognizer.state == .began { lastPinchScale = 1 }
        guard recognizer.state == .began || recognizer.state == .changed || recognizer.state == .ended,
              recognizer.scale.isFinite, recognizer.scale > 0 else { return }
        let factor = recognizer.scale / lastPinchScale
        lastPinchScale = recognizer.scale
        if factor != 1 {
            sendGesture(
                phase: .zoom, kind: recognizer.numberOfTouches > 0 ? 0 : 3,
                point: recognizer.location(in: self), modifiers: recognizer.modifierFlags, zoom: Float(factor)
            )
        }
    }

    private func sendGesture(
        phase: PointerPhase, kind: UInt32, point: CGPoint, modifiers: UIKeyModifierFlags,
        delta: CGPoint = .zero, zoom: Float = 0, pencil: PencilSample? = nil
    ) {
        _ = owner?.pointer(PointerSample(
            id: kind == 2 ? UInt64.max : 0, kind: kind, phase: phase, point: point, time: CACurrentMediaTime(),
            button: 0, buttons: 0, modifiers: impastoModifiers(modifiers), delta: delta, zoom: zoom, pencil: pencil
        ))
    }

    func cancelPointers() {
        routedPresses.removeAll()
        lastHoverPoint = nil
        for var sample in pointers.values {
            sample.phase = .cancel
            sample.button = 0
            sample.buttons = 0
            sample.time = CACurrentMediaTime()
            _ = owner?.pointer(sample)
        }
        pointers.removeAll(keepingCapacity: true)
    }

    private func sendTouches(_ touches: Set<UITouch>, event: UIEvent?, phase: PointerPhase) {
        for touch in touches {
            let key = ObjectIdentifier(touch)
            switch phase {
            case .down:
                guard pointers[key] == nil else { continue }
                nextPointerID += 1
                let sample = sample(touch, event: event, id: nextPointerID, phase: phase)
                if owner?.pointer(sample) == true {
                    pointers[key] = sample
                }
            case .move:
                guard let previous = pointers[key] else { continue }
                let samples = event?.coalescedTouches(for: touch) ?? [touch]
                var last = previous
                for touch in samples where touch.timestamp >= last.time {
                    last = sample(touch, event: event, id: previous.id, phase: phase, previous: last)
                    _ = owner?.pointer(last)
                }
                pointers[key] = last
            case .up, .cancel:
                guard let previous = pointers.removeValue(forKey: key) else { continue }
                let sample = sample(touch, event: event, id: previous.id, phase: phase, previous: previous)
                _ = owner?.pointer(sample)
            case .wheel, .zoom:
                break
            }
        }
    }

    private func sample(
        _ touch: UITouch,
        event: UIEvent?,
        id: UInt64,
        phase: PointerPhase,
        previous: PointerSample? = nil
    ) -> PointerSample {
        let kind: UInt32
        switch touch.type {
        case .pencil: kind = 2
        case .indirectPointer: kind = 1
        case .indirect: kind = 3
        case .direct: kind = 0
        @unknown default: kind = 0
        }

        let mask = event?.buttonMask ?? []
        let actual: UInt32 = mask.rawValue == 0 ? (previous?.buttons ?? 1) :
            (mask.contains(.primary) ? 1 : 0) |
            (mask.contains(.secondary) ? 2 : 0) |
            (mask.contains(.button(3)) ? 4 : 0)
        let button: UInt32
        let buttons: UInt32
        switch phase {
        case .down:
            buttons = actual
            button = actual & 1 != 0 ? 1 : actual & 2 != 0 ? 2 : actual & 4 != 0 ? 3 : 0
        case .move:
            (button, buttons) = (0, actual)
        case .up, .cancel:
            (button, buttons) = (previous?.buttons == 2 ? 2 : previous?.buttons == 4 ? 3 : 1, 0)
        case .wheel, .zoom:
            (button, buttons) = (0, 0)
        }

        let pencil = touch.type == .pencil ? PencilSample(
            force: touch.maximumPossibleForce > 0 ? Float(touch.force / touch.maximumPossibleForce) : 0,
            radius: Float(touch.majorRadius), altitude: Float(touch.altitudeAngle),
            azimuth: Float(touch.azimuthAngle(in: self))
        ) : nil
        return PointerSample(
            id: id, kind: kind, phase: phase, point: touch.location(in: self),
            time: touch.timestamp, button: button, buttons: buttons,
            modifiers: event.map { impastoModifiers($0.modifierFlags) } ?? previous?.modifiers ?? 0,
            pencil: pencil
        )
    }
}

@MainActor
private final class KeyboardLayoutProbe: UIView {
    var onGeometryChanged: (() -> Void)?

    override var frame: CGRect {
        didSet {
            if frame != oldValue { onGeometryChanged?() }
        }
    }
}

@MainActor
private final class ImpastoScrollRecognizer: UIPanGestureRecognizer {
    let kind: UInt32

    init(kind: UInt32, target: Any?, action: Selector?) {
        self.kind = kind
        super.init(target: target, action: action)
    }
}

enum PointerPhase: UInt32 {
    case down, move, up, cancel, wheel, zoom
}

struct PencilSample {
    let force: Float
    let radius: Float
    let altitude: Float
    let azimuth: Float
}

struct PointerSample {
    let id: UInt64
    let kind: UInt32
    var phase: PointerPhase
    let point: CGPoint
    var time: TimeInterval
    var button: UInt32
    var buttons: UInt32
    let modifiers: UInt32
    var delta: CGPoint = .zero
    var zoom: Float = 0
    var pencil: PencilSample? = nil
}
