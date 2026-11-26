import ImpastoGeneratedPlugins
import CoreGraphics
import CoreVideo
import QuartzCore
import UIKit

@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(
        _ application: UIApplication,
        configurationForConnecting session: UISceneSession,
        options: UIScene.ConnectionOptions
    ) -> UISceneConfiguration {
        let configuration = UISceneConfiguration(name: "Impasto", sessionRole: session.role)
        configuration.sceneClass = UIWindowScene.self
        configuration.delegateClass = SceneDelegate.self
        return configuration
    }
}

final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    private var host: ImpastoHost?

    func scene(
        _ scene: UIScene,
        willConnectTo session: UISceneSession,
        options: UIScene.ConnectionOptions
    ) {
        guard let scene = scene as? UIWindowScene, let resources = Bundle.main.resourceURL else { return }
        let host = ImpastoHost(resources: resources)
        let content = host.makeViewController()
        host.registerPlatformView("green") { _ in
            Self.nativeView(label: "原生绿", red: 0, green: 204, blue: 0)
        }
        host.registerPlatformView("orange") { _ in
            Self.nativeView(label: "原生橙", red: 238, green: 136, blue: 0)
        }
        host.onFirstFrame = { [weak self, weak content] _ in
            guard let self, let content else { return }
            Task { await self.verify(content) }
        }

        let parent = UIViewController()
        parent.view.backgroundColor = .purple
        parent.addChild(content)
        content.view.translatesAutoresizingMaskIntoConstraints = false
        parent.view.addSubview(content.view)
        NSLayoutConstraint.activate([
            content.view.leadingAnchor.constraint(equalTo: parent.view.leadingAnchor, constant: 20),
            content.view.topAnchor.constraint(equalTo: parent.view.topAnchor, constant: 20),
            content.view.trailingAnchor.constraint(equalTo: parent.view.trailingAnchor, constant: -20),
            content.view.bottomAnchor.constraint(equalTo: parent.view.bottomAnchor, constant: -20),
        ])
        content.didMove(toParent: parent)

        let window = UIWindow(windowScene: scene)
        window.rootViewController = parent
        self.window = window
        self.host = host
        Task {
            do {
                try await host.prepare()
            } catch {
                Self.report("无法预热宿主：\(error)")
            }
        }
        window.makeKeyAndVisible()
    }

    func sceneDidDisconnect(_ scene: UIScene) {
        host?.close()
    }

    private func verify(_ content: ImpastoViewController) async {
        do {
            guard let host,
                  let bgra = host.makeExternalTexture(named: "bgra"),
                  let nv12 = host.makeExternalTexture(named: "nv12")
            else { throw FixtureFailure("无法创建媒体纹理") }
            bgra.update(try Self.buffer(kCVPixelFormatType_32BGRA))
            let video = try Self.buffer(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange)
            nv12.update(video)

            _ = try await verifiedImage(from: host, content: content, flipped: false)
            nv12.update(video, orientation: .down)
            let image = try await verifiedImage(from: host, content: content, flipped: true)
            guard let png = image.pngData() else { throw FixtureFailure("无法编码截图 PNG") }
            try png.write(to: Self.documents.appendingPathComponent("mixed.png"), options: .atomic)

            host.detach()
            do {
                _ = try host.captureSnapshot()
                throw FixtureFailure("解绑后仍然返回截图")
            } catch ImpastoHost.SnapshotError.unavailable {
                Self.report("ok")
            }
        } catch {
            Self.report(String(describing: error))
        }
    }

    private func verifiedImage(
        from host: ImpastoHost, content: ImpastoViewController, flipped: Bool
    ) async throws -> UIImage {
        let deadline = CACurrentMediaTime() + 6
        var failure: Error = FixtureFailure("当前还没有可用的截图")
        while CACurrentMediaTime() < deadline {
            do {
                let image = try host.captureSnapshot()
                try checkPixels(image, in: content.view, flipped: flipped)
                try checkAccessibility(content.view)
                try checkClipHits(content)
                return image
            } catch {
                failure = error
                try await Task.sleep(nanoseconds: 200_000_000)
            }
        }
        throw failure
    }

    private func checkPixels(_ snapshot: UIImage, in view: UIView, flipped: Bool) throws {
        guard let image = snapshot.cgImage, let parent = view.superview,
              let srgb = CGColorSpace(name: CGColorSpace.sRGB),
              image.colorSpace?.name == srgb.name,
              image.width == Int((view.bounds.width * snapshot.scale).rounded()),
              image.height == Int((view.bounds.height * snapshot.scale).rounded()),
              image.width < Int((parent.bounds.width * snapshot.scale).rounded()),
              image.height < Int((parent.bounds.height * snapshot.scale).rounded())
        else { throw FixtureFailure("截图应为宿主子视图尺寸的 sRGB 图像") }

        let light = RGB(203, 203, 203)
        let dark = RGB(75, 75, 75)
        let samples: [(String, CGFloat, CGFloat, RGB)] = [
            ("底层 raster", 20, 45, RGB(208, 0, 0)),
            ("第一个原生视图", 50, 64, RGB(0, 204, 0)),
            ("两个原生视图之间的 raster", 105, 60, RGB(0, 0, 221)),
            ("第二个原生视图", 150, 70, RGB(238, 136, 0)),
            ("上层 raster", 200, 100, RGB(255, 0, 255)),
            ("原生视图裁去的区域", 38, 43, RGB(208, 0, 0)),
            ("BGRA", 45, 184, RGB(0, 221, 221)),
            ("NV12 上半部", 149, 184, flipped ? dark : light),
            ("NV12 下半部", 149, 210, flipped ? light : dark),
        ]
        for (label, x, y, expected) in samples {
            let pixelX = ((x + view.safeAreaInsets.left) * snapshot.scale).rounded()
            let pixelY = ((y + view.safeAreaInsets.top) * snapshot.scale).rounded()
            let region = CGRect(x: pixelX, y: pixelY, width: 1, height: 1)
            guard let sample = image.cropping(to: region), let context = CGContext(
                data: nil, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                space: srgb,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ), let data = context.data else {
                throw FixtureFailure("无法读取 \(label) 的像素")
            }
            context.draw(sample, in: CGRect(x: 0, y: 0, width: 1, height: 1))
            let bytes = data.assumingMemoryBound(to: UInt8.self)
            let actual = RGB(Int(bytes[0]), Int(bytes[1]), Int(bytes[2]))
            guard actual.matches(expected) else {
                throw FixtureFailure("\(label) 的像素为 \(actual)，预期 \(expected)")
            }
        }
    }

    private func checkClipHits(_ content: ImpastoViewController) throws {
        guard let view = content.viewIfLoaded else { throw FixtureFailure("宿主视图未加载") }
        let offset = view.safeAreaInsets
        let outside = CGPoint(x: 38 + offset.left, y: 43 + offset.top)
        let inside = CGPoint(x: 54 + offset.left, y: 43 + offset.top)
        guard view.hitTest(outside, with: nil) === content.drawableView,
              view.hitTest(inside, with: nil)?.accessibilityLabel == "原生绿"
        else { throw FixtureFailure("原生视图的不规则裁剪区域命中错误") }
    }

    private func checkAccessibility(_ view: UIView) throws {
        func labels(in element: Any) -> [String] {
            if let leaf = element as? UIAccessibilityElement {
                return leaf.accessibilityLabel.map { [$0] } ?? []
            }
            guard let native = element as? UIView, !native.isHidden, !native.accessibilityElementsHidden else {
                return []
            }
            if native.isAccessibilityElement { return native.accessibilityLabel.map { [$0] } ?? [] }
            return (native.accessibilityElements ?? native.subviews).flatMap { labels(in: $0) }
        }
        let labels = (view.accessibilityElements ?? []).flatMap { labels(in: $0) }
        let expected = ["框架前项", "原生绿", "框架后项", "原生橙"]
        guard labels == expected else { throw FixtureFailure("无障碍阅读顺序为 \(labels)，预期 \(expected)") }
    }

    private static func nativeView(label: String, red: CGFloat, green: CGFloat, blue: CGFloat) -> UIView {
        let view = UIView()
        view.backgroundColor = UIColor(red: red / 255, green: green / 255, blue: blue / 255, alpha: 1)
        view.isAccessibilityElement = true
        view.accessibilityLabel = label
        return view
    }

    private static func buffer(_ format: OSType) throws -> CVPixelBuffer {
        let attributes: [String: Any] = [
            kCVPixelBufferMetalCompatibilityKey as String: true,
            kCVPixelBufferIOSurfacePropertiesKey as String: [:] as [String: Any],
        ]
        var output: CVPixelBuffer?
        guard CVPixelBufferCreate(nil, 64, 64, format, attributes as CFDictionary, &output) == kCVReturnSuccess,
              let output, CVPixelBufferLockBaseAddress(output, []) == kCVReturnSuccess
        else { throw FixtureFailure("无法分配媒体像素缓冲区") }
        defer { CVPixelBufferUnlockBaseAddress(output, []) }
        if format == kCVPixelFormatType_32BGRA {
            guard let data = CVPixelBufferGetBaseAddress(output) else { throw FixtureFailure("BGRA 缓冲区无数据") }
            let stride = CVPixelBufferGetBytesPerRow(output)
            for y in 0..<64 {
                for x in 0..<64 {
                    data.storeBytes(of: UInt32(0xff00dddd), toByteOffset: y * stride + x * 4, as: UInt32.self)
                }
            }
        } else {
            for plane in 0..<2 {
                guard let data = CVPixelBufferGetBaseAddressOfPlane(output, plane) else {
                    throw FixtureFailure("NV12 缓冲区缺少平面")
                }
                let stride = CVPixelBufferGetBytesPerRowOfPlane(output, plane)
                let height = CVPixelBufferGetHeightOfPlane(output, plane)
                for y in 0..<height {
                    let value: Int32 = plane == 1 ? 128 : (y < height / 2 ? 190 : 80)
                    memset(data.advanced(by: y * stride), value, stride)
                }
            }
            CVBufferSetAttachment(output, kCVImageBufferYCbCrMatrixKey,
                                  kCVImageBufferYCbCrMatrix_ITU_R_709_2, .shouldPropagate)
        }
        return output
    }

    private static var documents: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
    }

    private static func report(_ text: String) {
        do {
            try text.write(to: documents.appendingPathComponent("report.txt"), atomically: true, encoding: .utf8)
        } catch {
            NSLog("无法写入验收报告: %@", String(describing: error))
        }
    }
}

private struct FixtureFailure: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

private struct RGB: CustomStringConvertible {
    let red: Int
    let green: Int
    let blue: Int

    init(_ red: Int, _ green: Int, _ blue: Int) {
        self.red = red
        self.green = green
        self.blue = blue
    }

    func matches(_ other: RGB) -> Bool {
        abs(red - other.red) <= 6 && abs(green - other.green) <= 6 && abs(blue - other.blue) <= 6
    }

    var description: String { "(\(red), \(green), \(blue))" }
}
