import UIKit

struct ImpastoEditingState: Decodable {
    let shown: Bool
    let client: UInt64?
    let ack: UInt64?
    let node: UInt64?
    let text: String?
    let start: UInt32?
    let end: UInt32?
    let composeStart: Int32?
    let composeEnd: Int32?
    let x: CGFloat?
    let y: CGFloat?
    let w: CGFloat?
    let h: CGFloat?
    let action: String?
    let keyboard: String?
    let autofill: String?
    let fieldX: CGFloat?
    let fieldY: CGFloat?
    let fieldWidth: CGFloat?
    let fieldHeight: CGFloat?

    var caret: CGRect {
        CGRect(x: x ?? 0, y: y ?? 0, width: max(w ?? 1, 1), height: max(h ?? 1, 1))
    }

    var frame: CGRect? {
        guard let fieldX, let fieldY, let fieldWidth, let fieldHeight,
              fieldWidth > 0, fieldHeight > 0 else { return nil }
        return CGRect(x: fieldX, y: fieldY, width: fieldWidth, height: fieldHeight)
    }
}

struct ImpastoTextSnapshot: Equatable {
    let text: String
    let start: UInt32
    let end: UInt32
    let composeStart: Int32
    let composeEnd: Int32

    init(text: String, selection: NSRange, marked: NSRange?) {
        self.text = text
        start = UInt32(clamping: selection.location)
        end = UInt32(clamping: selection.location + selection.length)
        composeStart = marked.map { Int32(clamping: $0.location) } ?? -1
        composeEnd = marked.map { Int32(clamping: $0.location + $0.length) } ?? -1
    }
}

@MainActor
final class ImpastoTextInput: NSObject, UITextViewDelegate {
    private weak var controller: ImpastoViewController?
    private var view: EditingProxy?
    private var state: ImpastoEditingState?
    private var nodes: [ImpastoSemanticsNode] = []
    private var autofillFields: [UInt64: AutofillCompanion] = [:]
    private var sent: ImpastoTextSnapshot?
    private var sequence: UInt64 = 0
    private var applying = false
    private weak var queuedProxy: EditingProxy?
    private var focusAllowed = false
    private var scribbleFocus: ScribbleFocus?
    private var scribbleTimeout: Task<Void, Never>?

    private struct ScribbleFocus {
        let node: UInt64
        let completion: ((any UIResponder & UITextInput)?) -> Void
    }

    init(_ controller: ImpastoViewController) {
        self.controller = controller
        super.init()
    }

    var hasClient: Bool { state != nil }

    var activeElement: UIView? { view.flatMap { $0.isFirstResponder ? $0 : nil } }

    var focusedNode: UInt64? {
        if let node = state?.node, nodes.contains(where: { $0.id == node }) { return node }
        return nodes.first(where: { $0.focused && $0.editable })?.id
    }

    func attach(to container: UIView) {
        container.addInteraction(UIIndirectScribbleInteraction(delegate: self))
    }

    func apply(_ state: ImpastoEditingState) {
        guard state.shown, let client = state.client else {
            let ownedFocus = view?.isFirstResponder == true
            clear()
            if ownedFocus { controller?.focusDefaultInput() }
            return
        }
        if let view = queuedProxy, self.state?.client == client {
            sendEdit(from: view)
        }
        if self.state?.client != client {
            // Disassociate UIKit's undo stack, composition and pending delegate calls from the old client.
            removeProxy()
            sequence = 0
            sent = nil
        }
        self.state = state
        guard let proxy = getProxy() else { return }
        updateGeometry()
        configure(proxy, state: state)
        updateAutofillContext()
        if (state.ack ?? 0) >= sequence {
            applySnapshot(to: proxy, state: state)
        }
        updateFirstResponder()
        resolveScribble()
    }

    func useNodes(_ nodes: [ImpastoSemanticsNode]) {
        self.nodes = nodes
        updateGeometry()
        if let view, let state {
            configure(view, state: state)
        }
        updateAutofillContext()
        resolveScribble()
    }

    func setFocusAllowed(_ allowed: Bool) {
        guard allowed != focusAllowed else { return }
        focusAllowed = allowed
        updateFirstResponder()
        controller?.accessibilityBridge.refresh()
    }

    func canShowEditMenu(at point: CGPoint) -> Bool {
        guard let view, view.isFirstResponder, view.markedTextRange == nil else { return false }
        return view.frame.contains(point)
    }

    func showEditMenu(at point: CGPoint) {
        guard canShowEditMenu(at: point), let view, let container = controller?.drawableView else { return }
        if view.selectedRange.length == 0 && !view.isSecureTextEntry {
            view.select(nil)
        }
        view.presentEditMenu(at: view.convert(point, from: container))
    }

    func dismissFromPlatform() -> Bool {
        guard let controller, let view, view.isFirstResponder, let client = state?.client else { return false }
        sendEdit(from: view)
        controller.host.dismissEditing(controller, client: client)
        clear()
        return true
    }

    func clear() {
        state = nil
        sequence = 0
        sent = nil
        removeProxy()
        clearAutofillContext()
        finishScribble(nil)
    }

    private func removeProxy() {
        guard let view else { return }
        self.view = nil
        queuedProxy = nil
        view.delegate = nil
        view.resignFirstResponder()
        view.removeFromSuperview()
    }

    private func getProxy() -> EditingProxy? {
        if let view { return view }
        guard let container = controller?.drawableView else { return nil }
        let proxy = EditingProxy()
        proxy.delegate = self
        proxy.onChange = { [weak self, weak proxy] in
            guard let proxy else { return }
            self?.scheduleEdit(for: proxy)
        }
        proxy.onHardwarePress = { [weak self] press in
            self?.handlePress(press) ?? false
        }
        container.addSubview(proxy)
        view = proxy
        return proxy
    }

    private func updateGeometry() {
        guard let state, let view, let container = controller?.drawableView else { return }
        let node = focusedNode.flatMap { id in nodes.first { $0.id == id } }
        let frame = state.frame ?? node?.rect ?? state.caret.insetBy(dx: -22, dy: -8)
        let visible = frame.intersection(container.bounds)
        view.frame = visible.isEmpty
            ? CGRect(origin: container.bounds.origin, size: CGSize(width: 1, height: 1))
            : visible
        view.sceneCaret = state.caret.offsetBy(dx: -view.frame.minX, dy: -view.frame.minY)
        view.accessibilityLabel = node?.label
        view.accessibilityHint = node?.help
    }

    private func configure(_ view: EditingProxy, state: ImpastoEditingState) {
        let keyboard: UIKeyboardType = switch state.keyboard {
        case "number": .numberPad
        case "phone": .phonePad
        case "email": .emailAddress
        case "url": .URL
        case "name": .namePhonePad
        default: .default
        }
        let action: UIReturnKeyType = switch state.action {
        case "done": .done
        case "go": .go
        case "next": .next
        case "search": .search
        case "send": .send
        default: .default
        }
        let secure = state.keyboard == "password"
        let hint = state.autofill ?? focusedNode.flatMap { id in nodes.first { $0.id == id }?.autofill }
        let contentType = ImpastoTextInput.contentType(hint)
        let changed = view.keyboardType != keyboard || view.returnKeyType != action ||
            view.isSecureTextEntry != secure || view.textContentType != contentType
        guard changed else { return }
        applying = true
        defer { applying = false }
        view.keyboardType = keyboard
        view.returnKeyType = action
        view.textContentType = contentType
        view.isSecureTextEntry = secure
        view.autocorrectionType = secure ? .no : .default
        view.spellCheckingType = secure ? .no : .default
        view.autocapitalizationType = secure ? .none : .sentences
        if view.isFirstResponder { view.reloadInputViews() }
    }

    static func contentType(_ hint: String?) -> UITextContentType? {
        switch hint {
        case "username", "newUsername": .username
        case "password", "current-password": .password
        case "newPassword", "new-password": .newPassword
        case "oneTimeCode", "one-time-code": .oneTimeCode
        case "email", "emailAddress": .emailAddress
        case "name": .name
        case "tel", "telephoneNumber": .telephoneNumber
        default: nil
        }
    }

    private func updateAutofillContext() {
        guard let controller, let focusedNode, let active = nodes.first(where: { $0.id == focusedNode }),
              let hint = Self.contentType(state?.autofill ?? active.autofill),
              hint == .username || hint == .password || hint == .newPassword
        else {
            clearAutofillContext()
            return
        }

        // UIKit can fill the paired field while the active proxy receives the other value.
        // Choose the nearest opt-in counterpart in the same semantic container.
        let counterpartTypes: [UITextContentType] = hint == .username ? [.password, .newPassword] : [.username]
        let candidates = nodes.filter {
            $0.id != focusedNode && $0.parent == active.parent && $0.editable && !$0.disabled &&
                controller.drawableView.bounds.intersects($0.rect)
        }
        let counterpart = candidates.filter { node in
            guard let type = Self.contentType(node.autofill) else { return false }
            return counterpartTypes.contains(type)
        }.min { lhs, rhs in
            abs(lhs.rect.midY - active.rect.midY) < abs(rhs.rect.midY - active.rect.midY)
        }

        for id in autofillFields.keys.filter({ $0 != counterpart?.id }) {
            autofillFields.removeValue(forKey: id)?.removeFromSuperview()
        }
        guard let counterpart else { return }
        let field: AutofillCompanion
        if let existing = autofillFields[counterpart.id] {
            field = existing
        } else {
            field = AutofillCompanion()
            let id = counterpart.id
            field.completed = { [weak self, weak field] text in
                guard let self, let controller = self.controller, let field,
                      self.autofillFields[id] === field else { return }
                _ = controller.host.semanticsAction(controller, node: id, action: .autofill, text: text)
            }
            controller.drawableView.addSubview(field)
            autofillFields[id] = field
        }
        let counterpartType = Self.contentType(counterpart.autofill)
        if field.textContentType != counterpartType { field.textContentType = counterpartType }
        if field.isSecureTextEntry != counterpart.password { field.isSecureTextEntry = counterpart.password }
        // AX snapshots redact passwords. Keep the system-filled value only for the life of the proxy.
        if !counterpart.password && field.text != counterpart.value { field.text = counterpart.value }
        field.frame = counterpart.rect.intersection(controller.drawableView.bounds)
    }

    private func clearAutofillContext() {
        for field in autofillFields.values { field.removeFromSuperview() }
        autofillFields.removeAll()
    }

    private func applySnapshot(to view: EditingProxy, state: ImpastoEditingState) {
        let text = state.text ?? ""
        let length = text.utf16.count
        let start = min(length, Int(state.start ?? 0))
        let end = min(length, Int(state.end ?? state.start ?? 0))
        let selection = NSRange(location: min(start, end), length: abs(end - start))
        let composeStart = Int(state.composeStart ?? -1)
        let composeEnd = Int(state.composeEnd ?? -1)
        let composition: NSRange? = composeStart >= 0 && composeEnd > composeStart && composeEnd <= length
            ? NSRange(location: composeStart, length: composeEnd - composeStart) : nil
        let next = ImpastoTextSnapshot(text: text, selection: selection, marked: composition)
        sent = next
        if view.snapshot == next { return }

        applying = true
        defer { applying = false }
        if view.markedTextRange != nil {
            view.unmarkText()
        }
        if view.text != text {
            view.text = text
        }
        if let composition, let range = Range(composition, in: text),
           selection.location >= composition.location,
           NSMaxRange(selection) <= NSMaxRange(composition) {
            view.selectedRange = composition
            view.setMarkedText(String(text[range]), selectedRange: NSRange(
                location: selection.location - composition.location,
                length: selection.length
            ))
        } else {
            view.selectedRange = selection
        }
    }

    private func updateFirstResponder() {
        guard let view else { return }
        if focusAllowed && view.window?.isKeyWindow == true {
            if !view.isFirstResponder { view.becomeFirstResponder() }
        } else if view.isFirstResponder {
            applying = true
            view.resignFirstResponder()
            applying = false
        }
    }

    private func scheduleEdit(for view: EditingProxy) {
        guard self.view === view, !applying, queuedProxy !== view, focusAllowed else { return }
        queuedProxy = view
        // A compound UIKit edit reports text and selection as separate delegate callbacks.
        DispatchQueue.main.async { [weak self, weak view] in
            guard let self, let view, queuedProxy === view else { return }
            queuedProxy = nil
            sendEdit(from: view)
        }
    }

    private func sendEdit(from view: EditingProxy) {
        guard self.view === view, !applying, focusAllowed,
              let controller, let client = state?.client else { return }
        let snapshot = view.snapshot
        guard snapshot != sent else { return }
        sent = snapshot
        sequence &+= 1
        controller.host.replaceEditing(controller, client: client, sequence: sequence, snapshot: snapshot)
    }

    func textViewDidChange(_ textView: UITextView) {
        guard let view, textView === view else { return }
        scheduleEdit(for: view)
    }

    func textViewDidChangeSelection(_ textView: UITextView) {
        guard let view, textView === view else { return }
        scheduleEdit(for: view)
    }

    func textViewDidEndEditing(_ textView: UITextView) {
        guard let view, textView === view, !applying,
              focusAllowed, let controller, let client = state?.client else { return }
        // A native control taking first responder is a genuine departure from this editor.
        sendEdit(from: view)
        controller.host.dismissEditing(controller, client: client)
    }

    func textView(_ textView: UITextView, shouldChangeTextIn range: NSRange, replacementText text: String) -> Bool {
        guard let view, view === textView, let state else { return false }
        if applying || text != "\n" || view.markedTextRange != nil { return true }
        if state.action == "newline" || (state.keyboard == "multiline" && state.action == "none") { return true }
        if let controller {
            sendEdit(from: view)
            _ = controller.host.key(controller, name: "Enter", modifiers: 0)
        }
        return false
    }

    private func handlePress(_ press: UIPress) -> Bool {
        guard let controller, let key = press.key,
              let routed = ImpastoHardwareKey(key, editing: true) else { return false }
        if routed.name == "Escape" && dismissFromPlatform() {
            controller.focusDefaultInput()
            return true
        }
        if let view { sendEdit(from: view) }
        return controller.host.key(controller, name: routed.name, modifiers: routed.modifiers)
    }

    private func resolveScribble() {
        if let request = scribbleFocus, request.node == focusedNode, let view, view.isFirstResponder {
            finishScribble(view)
        }
    }

    private func finishScribble(_ view: (any UIResponder & UITextInput)?) {
        let request = scribbleFocus
        scribbleFocus = nil
        scribbleTimeout?.cancel()
        scribbleTimeout = nil
        request?.completion(view)
    }
}

extension ImpastoTextInput: @preconcurrency UIIndirectScribbleInteractionDelegate {
    func indirectScribbleInteraction(
        _ interaction: any UIInteraction, requestElementsIn rect: CGRect,
        completion: @escaping ([UInt64]) -> Void
    ) {
        completion(nodes.filter { $0.editable && !$0.disabled && $0.rect.intersects(rect) }.map(\.id))
    }

    func indirectScribbleInteraction(_ interaction: any UIInteraction, isElementFocused node: UInt64) -> Bool {
        focusedNode == node && view?.isFirstResponder == true
    }

    func indirectScribbleInteraction(_ interaction: any UIInteraction, frameForElement node: UInt64) -> CGRect {
        nodes.first { $0.id == node }?.rect ?? .null
    }

    func indirectScribbleInteraction(
        _ interaction: any UIInteraction, focusElementIfNeeded node: UInt64,
        referencePoint: CGPoint, completion: @escaping ((any UIResponder & UITextInput)?) -> Void
    ) {
        if focusedNode == node, let view, view.isFirstResponder {
            completion(view)
            return
        }
        finishScribble(nil)
        guard let controller,
              controller.host.semanticsAction(controller, node: node, action: .focus)
        else {
            completion(nil)
            return
        }
        scribbleFocus = ScribbleFocus(node: node, completion: completion)
        scribbleTimeout = Task { [weak self] in
            // UIKit's completion belongs to this request even if the route changed.
            do {
                try await Task.sleep(nanoseconds: 1_000_000_000)
                self?.finishScribble(nil)
            } catch {
                // A replacement request or a successful focus cancelled this task.
            }
        }
    }
}

@MainActor
private final class AutofillCompanion: UITextField {
    var completed: ((String) -> Void)?

    init() {
        super.init(frame: .zero)
        backgroundColor = .clear
        tintColor = .clear
        textColor = .clear
        accessibilityElementsHidden = true
        isAccessibilityElement = false
        addTarget(self, action: #selector(didFill), for: .editingChanged)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("Use init()") }

    @objc private func didFill() {
        if let text { completed?(text) }
    }

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool { false }
}

@MainActor
private final class EditingProxy: UITextView {
    var sceneCaret = CGRect.zero
    var onChange: (() -> Void)?
    var onHardwarePress: ((UIPress) -> Bool)?
    private var routedPresses: Set<ObjectIdentifier> = []
    private var standaloneMenu: NSObject?

    init() {
        super.init(frame: .zero, textContainer: nil)
        font = UIFont.systemFont(ofSize: 16)
        backgroundColor = .clear
        textColor = .clear
        tintColor = .clear
        textContainerInset = .zero
        textContainer.lineFragmentPadding = 0
        isScrollEnabled = false
        isHidden = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("Use init()") }

    private var caretPosition: UITextPosition? { selectedTextRange?.end }

    var snapshot: ImpastoTextSnapshot {
        let selection = selectedTextRange.map(range) ?? NSRange(location: 0, length: 0)
        return ImpastoTextSnapshot(text: text ?? "", selection: selection, marked: markedTextRange.map(range))
    }

    private func range(_ range: UITextRange) -> NSRange {
        let start = max(0, offset(from: beginningOfDocument, to: range.start))
        let end = max(start, offset(from: beginningOfDocument, to: range.end))
        return NSRange(location: start, length: end - start)
    }

    override func caretRect(for position: UITextPosition) -> CGRect {
        guard let caretPosition, !sceneCaret.isEmpty,
              compare(caretPosition, to: position) == .orderedSame else {
            return super.caretRect(for: position)
        }
        return sceneCaret
    }

    override func firstRect(for range: UITextRange) -> CGRect {
        let native = super.firstRect(for: range)
        guard let caretPosition, !sceneCaret.isEmpty, !native.isNull else { return native }
        let anchor = super.caretRect(for: caretPosition)
        return native.offsetBy(dx: sceneCaret.minX - anchor.minX, dy: sceneCaret.minY - anchor.minY)
    }

    override func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        super.setMarkedText(markedText, selectedRange: selectedRange)
        onChange?()
    }

    override func unmarkText() {
        super.unmarkText()
        onChange?()
    }

    func presentEditMenu(at point: CGPoint) {
        if #available(iOS 16.0, *) {
            let menu: UIEditMenuInteraction
            if let existing = interactions.compactMap({ $0 as? UIEditMenuInteraction }).first {
                menu = existing
            } else {
                let delegate = ImpastoStandaloneEditMenu(view: self)
                standaloneMenu = delegate
                menu = UIEditMenuInteraction(delegate: delegate)
                addInteraction(menu)
            }
            menu.presentEditMenu(with: UIEditMenuConfiguration(identifier: nil, sourcePoint: point))
        } else {
            UIMenuController.shared.showMenu(from: self, rect: CGRect(origin: point, size: .zero))
        }
    }

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        // Rendered field gestures stay in the Impasto scene. UIKit retains the text input
        // and accessibility responder, including its selection commands and dictation.
        false
    }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let forwarded = presses.filter { press in
            if onHardwarePress?(press) == true {
                routedPresses.insert(ObjectIdentifier(press))
                return false
            }
            return true
        }
        if !forwarded.isEmpty { super.pressesBegan(forwarded, with: event) }
    }

    override func pressesChanged(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let forwarded = presses.filter { !routedPresses.contains(ObjectIdentifier($0)) }
        if !forwarded.isEmpty { super.pressesChanged(forwarded, with: event) }
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let forwarded = ending(presses)
        if !forwarded.isEmpty { super.pressesEnded(forwarded, with: event) }
    }

    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let forwarded = ending(presses)
        if !forwarded.isEmpty { super.pressesCancelled(forwarded, with: event) }
    }

    private func ending(_ presses: Set<UIPress>) -> Set<UIPress> {
        presses.filter { routedPresses.remove(ObjectIdentifier($0)) == nil }
    }
}

@available(iOS 16.0, *)
@MainActor
private final class ImpastoStandaloneEditMenu: NSObject, @preconcurrency UIEditMenuInteractionDelegate {
    weak var view: UITextView?

    init(view: UITextView) {
        self.view = view
    }

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction, menuFor configuration: UIEditMenuConfiguration,
        suggestedActions: [UIMenuElement]
    ) -> UIMenu? {
        guard let view else { return nil }
        if !suggestedActions.isEmpty { return UIMenu(children: suggestedActions) }
        let edits: [(Selector, String, (UITextView) -> Void)] = [
            (#selector(UIResponderStandardEditActions.cut(_:)), String(localized: "Cut"), { $0.cut(nil) }),
            (#selector(UIResponderStandardEditActions.copy(_:)), String(localized: "Copy"), { $0.copy(nil) }),
            (#selector(UIResponderStandardEditActions.paste(_:)), String(localized: "Paste"), { $0.paste(nil) }),
            (#selector(UIResponderStandardEditActions.selectAll(_:)), String(localized: "Select All"), { $0.selectAll(nil) }),
        ]
        return UIMenu(children: edits.compactMap { selector, title, edit in
            guard view.canPerformAction(selector, withSender: nil) else { return nil }
            return UIAction(title: title) { [weak view] _ in
                if let view { edit(view) }
            }
        })
    }
}

struct ImpastoHardwareKey {
    let name: String
    let modifiers: UInt32

    @MainActor init?(_ key: UIKey, editing: Bool) {
        modifiers = impastoModifiers(key.modifierFlags)
        let special: String? = switch key.keyCode {
        case .keyboardTab: "Tab"
        case .keyboardEscape: "Escape"
        case .keyboardDeleteOrBackspace: "Backspace"
        case .keyboardDeleteForward: "Delete"
        case .keyboardReturnOrEnter, .keypadEnter: "Enter"
        case .keyboardLeftArrow: modifiers & 8 != 0 ? "Home" : modifiers & 4 != 0 ? "WordLeft" : "ArrowLeft"
        case .keyboardRightArrow: modifiers & 8 != 0 ? "End" : modifiers & 4 != 0 ? "WordRight" : "ArrowRight"
        case .keyboardUpArrow: modifiers & 8 != 0 ? "DocumentStart" : "ArrowUp"
        case .keyboardDownArrow: modifiers & 8 != 0 ? "DocumentEnd" : "ArrowDown"
        case .keyboardHome: "Home"
        case .keyboardEnd: "End"
        default: nil
        }
        if let special {
            if editing && special != "Escape" && special != "Tab" { return nil }
            name = special
            return
        }
        let character = key.charactersIgnoringModifiers
        let shortcutModifiers: UInt32 = editing ? 2 | 8 : 2 | 4 | 8
        guard modifiers & shortcutModifiers != 0, character.unicodeScalars.count == 1,
              !character.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) })
        else { return nil }
        if editing && ["a", "c", "v", "x", "z"].contains(character.lowercased()) {
            // UIKit owns editing, paste consent and undo for the active text proxy.
            return nil
        }
        name = character
    }
}

func impastoModifiers(_ flags: UIKeyModifierFlags) -> UInt32 {
    var bits: UInt32 = 0
    if flags.contains(.shift) { bits |= 1 }
    if flags.contains(.control) { bits |= 2 }
    if flags.contains(.alternate) { bits |= 4 }
    if flags.contains(.command) { bits |= 8 }
    return bits
}
