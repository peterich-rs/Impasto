import ImpastoGeneratedPlugins
import UIKit

@main
final class AppDelegate: ImpastoAppDelegate {
    override func application(
        _ application: UIApplication,
        willFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
        GeneratedPluginRegistrant.prepare()
    }

    override func registerPlugins(with host: ImpastoHost) {
        GeneratedPluginRegistrant.register(with: host)
    }
}
