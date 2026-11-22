import AVFoundation
import CoreGraphics
import CoreImage
import CoreMedia
import CoreVideo
import Foundation
import ImageIO
import Metal

/// Publishes the latest immutable pixel buffer for an Impasto ExternalTexture.
/// The producer may release its buffer after update returns.
public final class ImpastoExternalTexture: @unchecked Sendable {
    private struct State {
        var latest: PixelBuffer?
        var generation: UInt64 = 0
        var inFlight = 0
        var scheduled = false
        var suspended = true
        var frozen = false
        var closed = false

        var canPublish: Bool { !closed && !suspended && !frozen }
    }

    private final class PixelBuffer: @unchecked Sendable {
        let value: CVPixelBuffer
        let orientation: CGImagePropertyOrientation

        init(_ value: CVPixelBuffer, orientation: CGImagePropertyOrientation) {
            self.value = value
            self.orientation = orientation
        }
    }

    private let lock = NSLock()
    private var state = State()
    private let queue = DispatchQueue(label: "dev.impasto.media-conversion", qos: .userInitiated)
    private let converter: MediaFrameConverter
    private let publish: @MainActor (MediaFrame, @escaping @Sendable () -> Void) -> Void

    fileprivate init(
        device: any MTLDevice,
        publish: @escaping @MainActor (MediaFrame, @escaping @Sendable () -> Void) -> Void
    ) {
        converter = MediaFrameConverter(device: device)
        self.publish = publish
    }

    public func update(_ pixelBuffer: CVPixelBuffer, orientation: CGImagePropertyOrientation = .up) {
        mutate { state in
            guard state.canPublish else { return }
            state.latest = PixelBuffer(pixelBuffer, orientation: orientation)
        }
    }

    /// Leaves the published image on screen. Resume by publishing another frame.
    public func setFrozen(_ frozen: Bool) {
        mutate { state in
            guard state.frozen != frozen else { return }
            state.frozen = frozen
            if frozen { clearPending(&state) }
        }
    }

    /// Reads the next available decoded video frame. Supply a display-link targetTimestamp.
    @discardableResult
    public func update(
        from output: AVPlayerItemVideoOutput, at hostTime: CFTimeInterval,
        orientation: CGImagePropertyOrientation = .up
    ) -> Bool {
        let itemTime = output.itemTime(forHostTime: hostTime)
        guard output.hasNewPixelBuffer(forItemTime: itemTime),
              let buffer = output.copyPixelBuffer(forItemTime: itemTime, itemTimeForDisplay: nil)
        else { return false }
        update(buffer, orientation: orientation)
        return true
    }

    fileprivate func setSuspended(_ suspended: Bool) {
        mutate { state in
            guard state.suspended != suspended else { return }
            state.suspended = suspended
            if suspended { clearPending(&state) }
        }
    }

    fileprivate func close() {
        mutate { state in
            state.closed = true
            clearPending(&state)
        }
    }

    private func clearPending(_ state: inout State) {
        state.generation &+= 1
        state.latest = nil
    }

    private func mutate(_ body: (inout State) -> Void) {
        // The mailbox holds one pending buffer. GPU owners hold at most three
        // prepared buffers until the Rust/Impeller release callback.
        let schedule = lock.withLock {
            body(&state)
            guard state.canPublish, state.latest != nil, state.inFlight < 3, !state.scheduled else { return false }
            state.scheduled = true
            return true
        }
        if schedule { queue.async { [weak self] in self?.prepareNext() } }
    }

    private func prepareNext() {
        let next: (PixelBuffer, UInt64)? = lock.withLock {
            guard state.canPublish, state.inFlight < 3, let latest = state.latest else {
                state.scheduled = false
                return nil
            }
            state.latest = nil
            state.inFlight += 1
            return (latest, state.generation)
        }
        guard let (buffer, generation) = next else { return }
        let frame = autoreleasepool { converter.frame(from: buffer.value, orientation: buffer.orientation) }
        mutate { $0.scheduled = false }
        guard let frame else {
            frameReleased()
            return
        }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            let current = lock.withLock { self.state.canPublish && self.state.generation == generation }
            guard current else {
                frameReleased()
                return
            }
            publish(frame, { [weak self] in self?.frameReleased() })
        }
    }

    private func frameReleased() {
        mutate { $0.inFlight -= 1 }
    }
}

/// A camera producer using the app's configured capture session and permissions.
public final class ImpastoCameraTexture: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate, @unchecked Sendable {
    public let output = AVCaptureVideoDataOutput()
    private weak var texture: ImpastoExternalTexture?

    public init(texture: ImpastoExternalTexture) {
        self.texture = texture
        super.init()
        output.alwaysDiscardsLateVideoFrames = true
        output.videoSettings = [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
            kCVPixelBufferMetalCompatibilityKey as String: true,
        ]
        output.setSampleBufferDelegate(self, queue: DispatchQueue(label: "dev.impasto.camera-texture"))
    }

    public func captureOutput(
        _ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection
    ) {
        if let buffer = CMSampleBufferGetImageBuffer(sampleBuffer) { texture?.update(buffer) }
    }
}

@MainActor
final class ExternalTextureRegistry {
    private struct Slot {
        let identifier: UUID
        let texture: ImpastoExternalTexture
    }

    private let host: @MainActor () -> OpaquePointer?
    private var slots: [String: Slot] = [:]
    private var active = false

    init(host: @escaping @MainActor () -> OpaquePointer?) {
        self.host = host
    }

    func register(_ name: String) -> ImpastoExternalTexture? {
        guard !name.isEmpty, name.utf8.count <= 128, !name.utf8.contains(0), slots[name] == nil,
              let device = MTLCreateSystemDefaultDevice()
        else { return nil }
        let identifier = UUID()
        let texture = ImpastoExternalTexture(device: device) { [weak self] frame, release in
            guard let self, slots[name]?.identifier == identifier, let handle = host(), active else {
                release()
                return
            }
            let context = Unmanaged.passRetained(MediaFrameLease(frame: frame, release: release)).toOpaque()
            let native = Unmanaged.passUnretained(frame.nativeTexture).toOpaque()
            _ = name.withCString {
                impasto_ios_texture_frame(
                    handle, $0, native, UInt32(frame.texture.width), UInt32(frame.texture.height),
                    context, releaseMediaFrame
                )
            }
        }
        slots[name] = Slot(identifier: identifier, texture: texture)
        texture.setSuspended(!active)
        return texture
    }

    func unregister(_ name: String) {
        guard let slot = slots.removeValue(forKey: name) else { return }
        slot.texture.close()
        if let handle = host() {
            name.withCString { impasto_ios_texture_unregister(handle, $0) }
        }
    }

    func setActive(_ active: Bool) {
        guard self.active != active else { return }
        self.active = active
        for slot in slots.values { slot.texture.setSuspended(!active) }
    }

    func close() {
        setActive(false)
        for name in Array(slots.keys) { unregister(name) }
    }
}

private final class MediaFrame: @unchecked Sendable {
    let texture: any MTLTexture
    let nativeTexture: AnyObject
    // Both Core Video objects must survive until the GPU finishes sampling.
    private let backing: (CVPixelBuffer, CVMetalTexture)

    init(_ texture: any MTLTexture, backing: (CVPixelBuffer, CVMetalTexture)) {
        self.texture = texture
        nativeTexture = texture as AnyObject
        self.backing = backing
    }
}

private final class MediaFrameLease {
    private var frame: MediaFrame?
    private let release: @Sendable () -> Void

    init(frame: MediaFrame, release: @escaping @Sendable () -> Void) {
        self.frame = frame
        self.release = release
    }

    deinit {
        frame = nil
        release()
    }
}

private func releaseMediaFrame(_ context: UnsafeMutableRawPointer?) {
    guard let context else { return }
    Unmanaged<MediaFrameLease>.fromOpaque(context).release()
}

// Created once per producer and used only on its serial conversion queue.
private final class MediaFrameConverter {
    private let device: any MTLDevice
    private lazy var cache: CVMetalTextureCache? = {
        var result: CVMetalTextureCache?
        guard CVMetalTextureCacheCreate(nil, nil, device, nil, &result) == kCVReturnSuccess else { return nil }
        return result
    }()
    private lazy var context = CIContext(mtlDevice: device, options: [.cacheIntermediates: false])
    private lazy var commands = device.makeCommandQueue()
    private var conversionPool: (width: Int, height: Int, pool: CVPixelBufferPool)?

    init(device: any MTLDevice) { self.device = device }

    func frame(from buffer: CVPixelBuffer, orientation: CGImagePropertyOrientation) -> MediaFrame? {
        let width = CVPixelBufferGetWidth(buffer)
        let height = CVPixelBufferGetHeight(buffer)
        guard width > 0, height > 0, width <= 8192, height <= 8192,
              width <= (16 * 1024 * 1024) / height
        else { return nil }

        switch CVPixelBufferGetPixelFormatType(buffer) {
        case kCVPixelFormatType_32BGRA:
            let colorSpace = CVImageBufferGetColorSpace(buffer)?.takeUnretainedValue()
            if orientation == .up, (colorSpace == nil || colorSpace?.name == CGColorSpace.sRGB),
               let cache,
               let frame = importBGRA(buffer, width: width, height: height, cache: cache) {
                return frame
            }
            return convert(buffer, orientation: orientation)
        case kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange:
            return convert(buffer, orientation: orientation)
        default:
            return nil
        }
    }

    private func importBGRA(
        _ buffer: CVPixelBuffer, width: Int, height: Int, cache: CVMetalTextureCache,
        attributes: CFDictionary? = nil
    ) -> MediaFrame? {
        var imported: CVMetalTexture?
        guard CVMetalTextureCacheCreateTextureFromImage(
            nil, cache, buffer, attributes, .bgra8Unorm, width, height, 0, &imported
        ) == kCVReturnSuccess, let imported, let texture = CVMetalTextureGetTexture(imported)
        else { return nil }
        return MediaFrame(texture, backing: (buffer, imported))
    }

    private func conversionBuffer(width: Int, height: Int) -> CVPixelBuffer? {
        if conversionPool?.width != width || conversionPool?.height != height {
            let attributes: [String: Any] = [
                kCVPixelBufferWidthKey as String: width,
                kCVPixelBufferHeightKey as String: height,
                kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
                kCVPixelBufferMetalCompatibilityKey as String: true,
                kCVPixelBufferIOSurfacePropertiesKey as String: [:] as [String: Any],
            ]
            var pool: CVPixelBufferPool?
            guard CVPixelBufferPoolCreate(nil, nil, attributes as CFDictionary, &pool) == kCVReturnSuccess,
                  let pool
            else { return nil }
            conversionPool = (width, height, pool)
        }
        guard let pool = conversionPool?.pool else { return nil }
        var buffer: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &buffer) == kCVReturnSuccess else { return nil }
        return buffer
    }

    private func convert(_ buffer: CVPixelBuffer, orientation: CGImagePropertyOrientation) -> MediaFrame? {
        // Core Image uses the source primaries, range and YCbCr matrix attached
        // to the CVPixelBuffer, and transforms the converted image into sRGB.
        let source = CIImage(cvPixelBuffer: buffer).oriented(orientation)
        let extent = source.extent
        let width = Int(extent.width)
        let height = Int(extent.height)
        // Core Image textures are bottom-up. Match the top-down Core Video
        // rows used by the direct BGRA import before Impeller samples them.
        let image = source.transformed(by: CGAffineTransform(
            a: 1, b: 0, c: 0, d: -1, tx: -extent.minX, ty: extent.maxY
        ))
        // Use a shareable IOSurface target for Core Image and Impeller's separate
        // Metal command queues. The producer's in-flight limit bounds pool usage.
        let usage: MTLTextureUsage = [.shaderRead, .shaderWrite, .renderTarget]
        let attributes = [kCVMetalTextureUsage: NSNumber(value: usage.rawValue)] as CFDictionary
        guard let cache, let destination = conversionBuffer(width: width, height: height),
              let frame = importBGRA(destination, width: width, height: height, cache: cache, attributes: attributes),
              let command = commands?.makeCommandBuffer(),
              let srgb = CGColorSpace(name: CGColorSpace.sRGB)
        else { return nil }
        context.render(image, to: frame.texture, commandBuffer: command, bounds: image.extent, colorSpace: srgb)
        command.commit()
        // Only the private producer queue waits. Impeller samples after this
        // fence, with a different command queue on the same device.
        command.waitUntilCompleted()
        return command.status == .completed ? frame : nil
    }
}
