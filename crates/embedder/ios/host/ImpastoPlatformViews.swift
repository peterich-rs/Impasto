import Metal
import QuartzCore
import UIKit

struct ImpastoPlatformRect: Decodable, Equatable {
    let x: CGFloat
    let y: CGFloat
    let w: CGFloat
    let h: CGFloat

    var rect: CGRect { CGRect(x: x, y: y, width: max(0, w), height: max(0, h)) }
}

struct ImpastoPlatformViewPlacement: Decodable, Equatable {
    let id: UInt64
    let kind: String
    let rect: ImpastoPlatformRect
    let clip: ImpastoPlatformRect
    let geometry: ImpastoPlatformGeometry?
    let occluders: [ImpastoPlatformRect]?
}

struct ImpastoPlatformGeometry: Decodable, Equatable {
    let width: CGFloat
    let height: CGFloat
    let transform: [CGFloat]
    let opacity: CGFloat
    let clips: [ImpastoPlatformClip]

    func localTransform(in frame: CGRect) -> CGAffineTransform? {
        impastoLocalTransform(transform, in: frame)
    }
}

struct ImpastoPlatformClip: Decodable, Equatable {
    let width: CGFloat
    let height: CGFloat
    let radius: CGFloat
    let transform: [CGFloat]
    let path: [[CGFloat]]?

    func clipPath(in frame: CGRect) -> CGPath? {
        guard width > 0, height > 0, var matrix = impastoLocalTransform(transform, in: frame) else {
            return nil
        }
        if let path {
            let result = CGMutablePath()
            for command in path {
                guard command.allSatisfy(\.isFinite) else { return nil }
                switch (command.first, command.count) {
                case (0, 3): result.move(to: CGPoint(x: command[1], y: command[2]))
                case (1, 3): result.addLine(to: CGPoint(x: command[1], y: command[2]))
                case (2, 5):
                    result.addQuadCurve(to: CGPoint(x: command[3], y: command[4]),
                                        control: CGPoint(x: command[1], y: command[2]))
                case (3, 7):
                    result.addCurve(to: CGPoint(x: command[5], y: command[6]),
                                    control1: CGPoint(x: command[1], y: command[2]),
                                    control2: CGPoint(x: command[3], y: command[4]))
                case (4, 1): result.closeSubpath()
                default: return nil
                }
            }
            return result.copy(using: &matrix)
        }
        let radius = min(max(radius, 0), min(width, height) / 2)
        let bounds = CGRect(x: 0, y: 0, width: width, height: height)
        return CGPath(roundedRect: bounds, cornerWidth: radius, cornerHeight: radius, transform: &matrix)
    }
}

/// The graphics protocol describes a local-to-scene affine. UIKit's clip frame
/// starts at a scene-relative origin, so only translation changes in the wrapper.
private func impastoLocalTransform(_ values: [CGFloat], in frame: CGRect) -> CGAffineTransform? {
    guard values.count == 6, values.allSatisfy(\.isFinite) else { return nil }
    return CGAffineTransform(
        a: values[0], b: values[1], c: values[2], d: values[3],
        tx: values[4] - frame.minX, ty: values[5] - frame.minY
    )
}

struct ImpastoCompositorLayers: Decodable {
    let count: Int
}

struct ImpastoComposedFrame: Decodable {
    let serial: UInt64
    let drawables: [UInt64]
    let views: [ImpastoPlatformViewPlacement]
    let resets: Bool
}

@MainActor
final class ImpastoPlatformViews {
    private let container = TransparentContainer()
    private weak var drawable: MetalView?
    private var overlays: [CAMetalLayer] = []
    private var lastSerial: UInt64?
    private var applying = false
    private var factories: [String: Factory] = [:]
    private var nextFactoryID: UInt64 = 0
    private var installed: [UInt64: Entry] = [:]
    private var placements: [ImpastoPlatformViewPlacement] = []

    private struct Factory {
        let id: UInt64
        let create: (UInt64) -> UIView
    }

    @MainActor
    private struct Entry {
        let factoryID: UInt64
        let kind: String
        let wrapper: ClipContainer
        let content: UIView
        let view: UIView
        var placement: ImpastoPlatformViewPlacement?

        func updateGeometry(_ placement: ImpastoPlatformViewPlacement) {
            let full = placement.rect.rect
            let clip = placement.clip.rect.intersection(full)
            wrapper.isHidden = clip.isEmpty
            if clip.isEmpty { return }
            wrapper.frame = clip
            wrapper.occluders = placement.occluders?.map(\.rect) ?? []

            if let geometry = placement.geometry {
                guard geometry.width > 0, geometry.height > 0,
                      let matrix = geometry.localTransform(in: clip) else {
                    wrapper.isHidden = true
                    return
                }
                let paths = geometry.clips.compactMap { $0.clipPath(in: clip) }
                guard paths.count == geometry.clips.count else {
                    wrapper.isHidden = true
                    return
                }
                wrapper.setClipPaths(paths)
                content.bounds = CGRect(x: 0, y: 0, width: geometry.width, height: geometry.height)
                content.transform = matrix
                content.alpha = min(max(geometry.opacity, 0), 1)
            } else {
                wrapper.setClipPaths([])
                content.bounds = CGRect(origin: .zero, size: full.size)
                content.transform = CGAffineTransform(
                    translationX: full.minX - clip.minX, y: full.minY - clip.minY
                )
                content.alpha = 1
            }

            view.bounds.size = content.bounds.size
            view.center = CGPoint(x: content.bounds.midX, y: content.bounds.midY)
        }
    }

    func attach(over drawable: MetalView) {
        guard let parent = drawable.superview else { return }
        self.drawable = drawable
        container.clipsToBounds = true
        container.translatesAutoresizingMaskIntoConstraints = false
        parent.insertSubview(container, aboveSubview: drawable)
        NSLayoutConstraint.activate([
            container.leadingAnchor.constraint(equalTo: drawable.leadingAnchor),
            container.trailingAnchor.constraint(equalTo: drawable.trailingAnchor),
            container.topAnchor.constraint(equalTo: drawable.topAnchor),
            container.bottomAnchor.constraint(equalTo: drawable.bottomAnchor),
        ])
        commit(placements)
    }

    func register(_ kind: String, factory: @escaping (UInt64) -> UIView) -> Bool {
        guard !kind.isEmpty, kind.utf8.count <= 128, factories[kind] == nil else { return false }
        nextFactoryID &+= 1
        factories[kind] = Factory(id: nextFactoryID, create: factory)
        if overlays.isEmpty { commit(placements) }
        return true
    }

    func unregister(_ kind: String) {
        guard factories.removeValue(forKey: kind) != nil else { return }
        if overlays.isEmpty { dispose(installed.compactMap { $0.value.kind == kind ? $0.key : nil }) }
    }

    func accessibilityView(for id: UInt64) -> UIView? {
        guard let entry = installed[id], !entry.wrapper.isHidden, entry.wrapper.superview != nil else { return nil }
        return entry.wrapper
    }

    func commit(_ placements: [ImpastoPlatformViewPlacement]) {
        self.placements = placements
        guard container.superview != nil, overlays.isEmpty else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        apply(placements)
        CATransaction.commit()
    }

    /// Applies placements to the transaction already opened by the compositor.
    private func apply(_ placements: [ImpastoPlatformViewPlacement]) {
        guard !applying else { return }
        applying = true
        defer { applying = false }
        self.placements = placements
        let next = Set(placements.map(\.id))
        dispose(installed.keys.filter { !next.contains($0) })
        for (index, placement) in placements.enumerated() {
            let factory = factories[placement.kind]
            if let installed = installed[placement.id], installed.factoryID != factory?.id {
                dispose([placement.id])
            }
            guard let factory else { continue }
            var entry: Entry
            if let current = installed[placement.id] {
                entry = current
            } else {
                let view = factory.create(placement.id)
                // A factory owns one view per slot. Reparenting would detach another live surface.
                guard container.superview != nil else { return }
                guard view.superview == nil, factories[placement.kind]?.id == factory.id else { continue }
                let wrapper = ClipContainer()
                wrapper.clipsToBounds = true
                let content = UIView()
                content.layer.anchorPoint = .zero
                content.layer.position = .zero
                content.addSubview(view)
                wrapper.addSubview(content)
                container.addSubview(wrapper)
                entry = Entry(factoryID: factory.id, kind: placement.kind, wrapper: wrapper, content: content, view: view)
            }
            if entry.placement != placement {
                entry.updateGeometry(placement)
                entry.placement = placement
            }
            installed[placement.id] = entry
            entry.wrapper.layer.zPosition = CGFloat(index * 2 + 1)
            if container.subviews.last !== entry.wrapper {
                container.bringSubviewToFront(entry.wrapper)
            }
        }
    }

    /// Render worker acquires these layers only after the bridge retains them.
    func setLayerCount(_ count: Int) -> [UnsafeRawPointer?] {
        guard count >= 0, let root = drawable?.metalLayer else { return [] }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        root.presentsWithTransaction = true
        while overlays.count < count {
            let overlay = CAMetalLayer()
            overlay.framebufferOnly = root.framebufferOnly
            overlay.isOpaque = false
            overlay.allowsNextDrawableTimeout = true
            overlay.presentsWithTransaction = true
            overlays.append(overlay)
        }
        // Retain existing layers while an older composition is still visible.
        // Unused layers are removed when the new frame replaces it.
        updateDrawableSizes()
        CATransaction.commit()
        return overlays.prefix(count).map { UnsafeRawPointer(Unmanaged.passUnretained($0).toOpaque()) }
    }

    func updateDrawableSizes() {
        guard let drawable else { return }
        let root = drawable.metalLayer
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for overlay in overlays {
            overlay.device = root.device
            overlay.pixelFormat = root.pixelFormat
            overlay.colorspace = root.colorspace
            if #available(iOS 16.0, *) {
                overlay.wantsExtendedDynamicRangeContent = root.wantsExtendedDynamicRangeContent
            }
            overlay.contentsScale = root.contentsScale
            overlay.drawableSize = root.drawableSize
            overlay.frame = CGRect(origin: .zero, size: drawable.bounds.size)
        }
        CATransaction.commit()
    }

    /// The drawables remain owned by the render worker until the caller acknowledges this frame.
    func commit(_ frame: ImpastoComposedFrame) -> Bool {
        guard let root = drawable?.metalLayer, (lastSerial ?? 0) < frame.serial,
              frame.drawables.count == frame.views.count + 1,
              frame.views.count <= overlays.count,
              !frame.resets || frame.views.isEmpty
        else { return false }

        let displayLayers = [root] + overlays.prefix(frame.views.count)
        var drawables: [any CAMetalDrawable] = []
        for (address, layer) in zip(frame.drawables, displayLayers) {
            guard let raw = UnsafeRawPointer(bitPattern: UInt(address)),
                  let drawable = Unmanaged<AnyObject>.fromOpaque(raw).takeUnretainedValue() as? any CAMetalDrawable,
                  drawable.layer === layer,
                  CGFloat(drawable.texture.width) == root.drawableSize.width,
                  CGFloat(drawable.texture.height) == root.drawableSize.height
            else { return false }
            drawables.append(drawable)
        }

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        apply(frame.views)
        guard drawable?.metalLayer === root else {
            // A factory may close its host while constructing a view.
            CATransaction.commit()
            return false
        }
        for (index, overlay) in overlays.enumerated() {
            if index < frame.views.count {
                overlay.zPosition = CGFloat(index * 2 + 2)
                if overlay.superlayer !== container.layer { container.layer.addSublayer(overlay) }
            } else {
                overlay.removeFromSuperlayer()
            }
        }
        for drawable in drawables { drawable.present() }
        CATransaction.commit()
        lastSerial = frame.serial
        if frame.resets {
            overlays.removeAll()
            root.presentsWithTransaction = false
        }
        return true
    }

    func detach() {
        clear()
        overlays.forEach { $0.removeFromSuperlayer() }
        overlays.removeAll()
        drawable?.metalLayer.presentsWithTransaction = false
        drawable = nil
        lastSerial = nil
        container.removeFromSuperview()
    }

    func clear() {
        placements = []
        dispose(Array(installed.keys))
    }

    private func dispose(_ ids: [UInt64]) {
        for id in ids {
            guard let entry = installed.removeValue(forKey: id) else { continue }
            entry.view.endEditing(true)
            entry.view.removeFromSuperview()
            entry.wrapper.removeFromSuperview()
        }
    }
}

@MainActor
private class TransparentContainer: UIView {
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        guard let hit = super.hitTest(point, with: event), hit !== self else { return nil }
        return hit
    }
}

@MainActor
private final class ClipContainer: TransparentContainer {
    private var paths: [CGPath] = []
    var occluders: [CGRect] = []

    func setClipPaths(_ paths: [CGPath]) {
        self.paths = paths
        // Linking masks compounds coverage, so overlapping round or transformed
        // ancestor clips are an intersection for both pixels and touch hits.
        var innerMask: CALayer?
        for path in paths.reversed() {
            let mask = CAShapeLayer()
            mask.frame = bounds
            mask.path = path
            mask.fillColor = UIColor.black.cgColor
            mask.mask = innerMask
            innerMask = mask
        }
        layer.mask = innerMask
    }

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        guard super.point(inside: point, with: event) else { return false }
        let scenePoint = CGPoint(x: frame.minX + point.x, y: frame.minY + point.y)
        return !occluders.contains { $0.contains(scenePoint) } && paths.allSatisfy { $0.contains(point) }
    }
}
