import AVFoundation
import CoreMedia
import CoreVideo
import Foundation
import Metal

@_silgen_name("impasto_camera_ios_frame")
private func receiveFrame(
    _ id: UInt64, _ width: UInt32, _ height: UInt32, _ stride: Int, _ pixels: UnsafeRawPointer, _ count: Int,
    _ texture: UnsafeMutableRawPointer?, _ owner: UInt, _ release: @convention(c) (UInt) -> Void
)

@_silgen_name("impasto_camera_ios_failed")
private func cameraFailed(_ id: UInt64)

private final class FrameLease {
    let buffer: CVPixelBuffer
    let view: CVMetalTexture
    let texture: any MTLTexture
    let end: () -> Void

    init(buffer: CVPixelBuffer, view: CVMetalTexture, texture: any MTLTexture, end: @escaping () -> Void) {
        self.buffer = buffer
        self.view = view
        self.texture = texture
        self.end = end
    }

    deinit { end() }
}

private func releaseFrame(_ owner: UInt) {
    guard let pointer = UnsafeMutableRawPointer(bitPattern: owner) else { return }
    Unmanaged<FrameLease>.fromOpaque(pointer).release()
}

private final class Capture: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    let id: UInt64
    let session = AVCaptureSession()
    private let credits = DispatchSemaphore(value: 3)
    private var cache: CVMetalTextureCache?

    init(id: UInt64) {
        self.id = id
    }

    func start(front: Bool, queue: DispatchQueue) -> Bool {
        guard AVCaptureDevice.authorizationStatus(for: .video) == .authorized,
              let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: front ? .front : .back),
              let input = try? AVCaptureDeviceInput(device: device),
              session.canAddInput(input)
        else { return false }

        if let device = MTLCreateSystemDefaultDevice() {
            CVMetalTextureCacheCreate(nil, nil, device, nil, &cache)
        }
        session.beginConfiguration()
        session.sessionPreset = .vga640x480
        session.addInput(input)
        let output = AVCaptureVideoDataOutput()
        output.alwaysDiscardsLateVideoFrames = true
        output.videoSettings = [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferMetalCompatibilityKey as String: true
        ]
        output.setSampleBufferDelegate(self, queue: queue)
        guard session.canAddOutput(output) else {
            session.commitConfiguration()
            return false
        }
        session.addOutput(output)
        session.commitConfiguration()
        session.startRunning()
        return session.isRunning
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sample: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard let buffer = CMSampleBufferGetImageBuffer(sample) else { return }
        guard CVPixelBufferLockBaseAddress(buffer, .readOnly) == kCVReturnSuccess else { return }
        defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
        guard let pixels = CVPixelBufferGetBaseAddress(buffer),
              let width = UInt32(exactly: CVPixelBufferGetWidth(buffer)),
              let height = UInt32(exactly: CVPixelBufferGetHeight(buffer))
        else { return }
        let stride = CVPixelBufferGetBytesPerRow(buffer)
        let lease = metalFrame(buffer, width: Int(width), height: Int(height))
        let texture = lease.map { Unmanaged.passUnretained($0.texture).toOpaque() }
        let owner = lease.map { UInt(bitPattern: Unmanaged.passRetained($0).toOpaque()) } ?? 0
        receiveFrame(id, width, height, stride, pixels, stride * Int(height), texture, owner, releaseFrame)
    }

    private func metalFrame(_ buffer: CVPixelBuffer, width: Int, height: Int) -> FrameLease? {
        guard let cache, credits.wait(timeout: .now()) == .success else { return nil }
        var view: CVMetalTexture?
        let attributes = [kCVMetalTextureUsage: NSNumber(value: MTLTextureUsage.shaderRead.rawValue)] as CFDictionary
        guard CVMetalTextureCacheCreateTextureFromImage(nil, cache, buffer, attributes, .bgra8Unorm, width, height, 0, &view) == kCVReturnSuccess,
              let view, let texture = CVMetalTextureGetTexture(view)
        else {
            credits.signal()
            return nil
        }
        return FrameLease(buffer: buffer, view: view, texture: texture) { [credits] in credits.signal() }
    }
}

private final class Cameras: @unchecked Sendable {
    static let shared = Cameras()
    private let queue = DispatchQueue(label: "dev.impasto.camera.capture", qos: .userInitiated)
    private var active: [UInt64: Capture] = [:]

    func start(id: UInt64, front: Bool) {
        queue.async {
            guard self.active[id] == nil else { return }
            let capture = Capture(id: id)
            if capture.start(front: front, queue: self.queue) {
                self.active[id] = capture
            } else {
                cameraFailed(id)
            }
        }
    }

    func stop(id: UInt64) {
        queue.async {
            self.active.removeValue(forKey: id)?.session.stopRunning()
        }
    }
}

/// Starts capture for a session owned by the Rust camera plugin.
@_cdecl("impasto_camera_start")
public func startImpastoCamera(_ id: UInt64, _ front: Bool) {
    Cameras.shared.start(id: id, front: front)
}

/// Stops capture and releases the platform session.
@_cdecl("impasto_camera_stop")
public func stopImpastoCamera(_ id: UInt64) {
    Cameras.shared.stop(id: id)
}
