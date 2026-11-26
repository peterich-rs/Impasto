import ImpastoHost

@MainActor
public final class CameraPlugin: ImpastoPlugin {
    public let identifier = "@impasto/camera"

    public static func register(with host: ImpastoHost) {
        host.registerPlugin(CameraPlugin())
    }
}
