import UIKit

struct ImpastoSemanticsNode: Decodable {
    let id: UInt64
    let parent: UInt64?
    let x: CGFloat
    let y: CGFloat
    let w: CGFloat
    let h: CGFloat
    let label: String
    let value: String
    let help: String
    let role: String
    let actions: UInt32
    let flags: UInt32
    let selectionStart: UInt32?
    let selectionEnd: UInt32?
    let scrollX: CGFloat
    let scrollY: CGFloat
    let contentWidth: CGFloat
    let contentHeight: CGFloat
    let autofill: String?
    let live: String?
    let platformView: Bool?

    var rect: CGRect { CGRect(x: x, y: y, width: max(0, w), height: max(0, h)) }
    var checked: Bool { flags & 2 != 0 }
    var disabled: Bool { flags & 4 != 0 }
    var editable: Bool { flags & 8 != 0 }
    var focused: Bool { flags & 32 != 0 }
    var password: Bool { flags & 64 != 0 }
    var canActivate: Bool { actions & 1 != 0 && !disabled }
    var canScroll: Bool { actions & 2 != 0 && !disabled }
}

enum ImpastoSemanticsAction: UInt32 {
    case activate, scroll, setText, setSelection, autofill, focus
}

@MainActor
final class ImpastoAccessibility {
    private weak var controller: ImpastoViewController?
    private var nodes: [UInt64: ImpastoSemanticsNode] = [:]
    private var order: [UInt64] = []
    private var elements: [UInt64: ImpastoAccessibilityElement] = [:]
    private var pendingScroll: PendingScroll?
    private var focusID: UInt64?

    private struct PendingScroll {
        let node: UInt64
        let oldOffset: CGPoint
    }

    init(_ controller: ImpastoViewController) {
        self.controller = controller
    }

    func apply(_ snapshot: [ImpastoSemanticsNode]) {
        guard let controller else { return }
        let previous = order
        let previousNodes = nodes
        order = snapshot.map(\.id)
        nodes = Dictionary(snapshot.map { ($0.id, $0) }, uniquingKeysWith: { _, newest in newest })
        elements = elements.filter { nodes[$0.key] != nil }
        for node in snapshot {
            let element = elements[node.id] ?? ImpastoAccessibilityElement(
                node: node.id, container: controller.view, bridge: self
            )
            element.update(node, viewport: controller.drawableView.bounds)
            elements[node.id] = element
            if controller.hasWindowFocus, UIAccessibility.isVoiceOverRunning, let live = node.live, live != "Off",
               !node.password, !node.label.isEmpty,
               previousNodes[node.id]?.label != node.label || previousNodes[node.id]?.value != node.value {
                let announcement = [node.label, node.value].filter { !$0.isEmpty }.joined(separator: ", ")
                UIAccessibility.post(notification: .announcement, argument: NSAttributedString(
                    string: announcement, attributes: [.accessibilitySpeechQueueAnnouncement: live == "Polite"]
                ))
            }
        }
        controller.textInput.useNodes(snapshot)
        refresh()
        if let pendingScroll, let node = nodes[pendingScroll.node],
           CGPoint(x: node.scrollX, y: node.scrollY) != pendingScroll.oldOffset {
            self.pendingScroll = nil
            UIAccessibility.post(notification: .pageScrolled, argument: node.label.isEmpty ? nil : node.label)
        }
        if let focusID, nodes[focusID] == nil {
            self.focusID = nil
            UIAccessibility.post(notification: .layoutChanged, argument: nil)
        } else if previous.isEmpty && !order.isEmpty {
            UIAccessibility.post(notification: .layoutChanged, argument: nil)
        }
    }

    func refresh() {
        guard let controller else { return }
        let proxy = controller.textInput.activeElement
        let focused = controller.textInput.focusedNode
        var ordered: [Any] = []
        var insertedProxy = false
        for id in order {
            if nodes[id]?.platformView == true {
                if let native = controller.host.accessibilityView(for: id) { ordered.append(native) }
            } else if id == focused, let proxy {
                ordered.append(proxy)
                insertedProxy = true
            } else if let element = elements[id], element.isAccessibilityElement {
                ordered.append(element)
            }
        }
        if !insertedProxy, let proxy { ordered.append(proxy) }
        controller.viewIfLoaded?.accessibilityElements = ordered
    }

    func clear() {
        pendingScroll = nil
        focusID = nil
        nodes.removeAll()
        order.removeAll()
        elements.removeAll()
        controller?.textInput.useNodes([])
        controller?.viewIfLoaded?.accessibilityElements = nil
    }

    fileprivate func activate(_ id: UInt64) -> Bool {
        guard let controller, let node = nodes[id], !node.disabled else { return false }
        if node.editable {
            return controller.host.semanticsAction(controller, node: id, action: .focus)
        }
        if node.canActivate {
            return controller.host.semanticsAction(controller, node: id, action: .activate)
        }
        return false
    }

    fileprivate func edit(_ id: UInt64, command: ImpastoSemanticsAction) -> Bool {
        guard let controller, let node = nodes[id], node.editable, !node.disabled else { return false }
        switch command {
        case .setText:
            guard node.actions & 4 != 0, let text = UIPasteboard.general.string else { return false }
            return controller.host.semanticsAction(controller, node: id, action: command, text: text)
        case .setSelection:
            guard node.actions & 8 != 0, !node.password else { return false }
            return controller.host.semanticsAction(
                controller, node: id, action: command, end: UInt32(clamping: node.value.utf16.count)
            )
        default:
            return false
        }
    }

    fileprivate func didFocus(_ id: UInt64) {
        if nodes[id] != nil { focusID = id }
    }

    func scroll(_ direction: UIAccessibilityScrollDirection, from id: UInt64? = nil) -> Bool {
        guard let controller else { return false }
        var ancestor = id ?? focusID
        var visited = Set<UInt64>()
        while let next = ancestor, let node = nodes[next], visited.insert(next).inserted {
            if node.canScroll && sendScroll(direction, on: node, controller: controller) { return true }
            ancestor = node.parent
        }
        // UIKit can call the container directly when no virtual node has accessibility focus.
        if id == nil && focusID == nil {
            for id in order {
                guard let node = nodes[id], node.canScroll else { continue }
                if sendScroll(direction, on: node, controller: controller) { return true }
            }
        }
        return false
    }

    private func sendScroll(
        _ direction: UIAccessibilityScrollDirection, on node: ImpastoSemanticsNode,
        controller: ImpastoViewController
    ) -> Bool {
        let horizontal = node.contentWidth > node.w + 0.5
        let vertical = node.contentHeight > node.h + 0.5
        let rtl = controller.drawableView.effectiveUserInterfaceLayoutDirection == .rightToLeft
        let step: CGPoint = switch direction {
        case .up: CGPoint(x: 0, y: node.h)
        case .down: CGPoint(x: 0, y: -node.h)
        case .left: CGPoint(x: node.w, y: 0)
        case .right: CGPoint(x: -node.w, y: 0)
        case .next: vertical ? CGPoint(x: 0, y: node.h) : CGPoint(x: rtl ? -node.w : node.w, y: 0)
        case .previous: vertical ? CGPoint(x: 0, y: -node.h) : CGPoint(x: rtl ? node.w : -node.w, y: 0)
        @unknown default: .zero
        }
        let x = horizontal ? min(max(node.scrollX + step.x, 0), node.contentWidth - node.w) - node.scrollX : 0
        let y = vertical ? min(max(node.scrollY + step.y, 0), node.contentHeight - node.h) - node.scrollY : 0
        if abs(x) <= 0.5 && abs(y) <= 0.5 { return false }
        let accepted = controller.host.semanticsAction(controller, node: node.id, action: .scroll, dx: x, dy: y)
        if accepted {
            pendingScroll = PendingScroll(node: node.id, oldOffset: CGPoint(x: node.scrollX, y: node.scrollY))
        }
        return accepted
    }
}

@MainActor
private final class ImpastoAccessibilityElement: UIAccessibilityElement {
    private let node: UInt64
    private weak var bridge: ImpastoAccessibility?

    init(node: UInt64, container: UIView, bridge: ImpastoAccessibility) {
        self.node = node
        self.bridge = bridge
        super.init(accessibilityContainer: container)
    }

    func update(_ node: ImpastoSemanticsNode, viewport: CGRect) {
        let rect = node.rect.intersection(viewport)
        accessibilityFrameInContainerSpace = rect.isNull ? .zero : rect
        accessibilityIdentifier = "impasto-node-\(node.id)"
        accessibilityLabel = node.label.isEmpty ? nil : node.label
        accessibilityHint = node.help.isEmpty ? nil : node.help
        accessibilityValue = !node.password && !node.value.isEmpty ? node.value : nil
        isAccessibilityElement = !rect.isEmpty && (
            !node.label.isEmpty || !node.value.isEmpty || node.role != "Scroll"
        )

        var traits: UIAccessibilityTraits = []
        switch node.role {
        case "Button": traits.insert(.button)
        case "Image": traits.insert(.image)
        case "Text": traits.insert(.staticText)
        case "SearchField": traits.insert(.searchField)
        case "Switch":
            if #available(iOS 17.0, *) { traits.insert(.toggleButton) }
            else { traits.insert(.button) }
            accessibilityValue = node.checked ? String(localized: "On") : String(localized: "Off")
        default: break
        }
        if node.disabled { traits.insert(.notEnabled) }
        accessibilityTraits = traits

        var actions: [UIAccessibilityCustomAction] = []
        if node.editable && !node.disabled && !node.password {
            if node.actions & 4 != 0 {
                actions.append(UIAccessibilityCustomAction(name: String(localized: "Replace with clipboard text")) { [weak self] _ in
                    guard let self else { return false }
                    return self.bridge?.edit(self.node, command: .setText) ?? false
                })
            }
            if node.actions & 8 != 0 {
                actions.append(UIAccessibilityCustomAction(name: String(localized: "Select All")) { [weak self] _ in
                    guard let self else { return false }
                    return self.bridge?.edit(self.node, command: .setSelection) ?? false
                })
            }
        }
        accessibilityCustomActions = actions
    }

    override func accessibilityActivate() -> Bool { bridge?.activate(node) ?? false }

    override func accessibilityScroll(_ direction: UIAccessibilityScrollDirection) -> Bool {
        bridge?.scroll(direction, from: node) ?? false
    }

    override func accessibilityElementDidBecomeFocused() {
        super.accessibilityElementDidBecomeFocused()
        bridge?.didFocus(node)
    }
}
