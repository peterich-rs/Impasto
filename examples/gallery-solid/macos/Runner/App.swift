import Darwin
import ImpastoGeneratedPlugins

@main
struct App {
    @MainActor
    static func main() {
        let host = ImpastoHost()
        GeneratedPluginRegistrant.register(with: host)
        exit(host.run())
    }
}
