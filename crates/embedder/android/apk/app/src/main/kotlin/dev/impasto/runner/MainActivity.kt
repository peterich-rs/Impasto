package dev.impasto.runner

import dev.impasto.host.ImpastoActivity
import dev.impasto.host.ImpastoHost
import dev.impasto.plugins.clipboard.ClipboardPlugin
import dev.impasto.plugins.connectivity.ConnectivityPlugin
import dev.impasto.plugins.filesystem.FileSystemPlugin
import dev.impasto.plugins.haptics.HapticsPlugin
import dev.impasto.plugins.linking.LinkingPlugin
import dev.impasto.plugins.permissions.PermissionsPlugin
import dev.impasto.plugins.share.SharePlugin
import dev.impasto.plugins.systeminfo.SystemInfoPlugin

class MainActivity : ImpastoActivity() {
    override fun registerPlugins(host: ImpastoHost) {
        ClipboardPlugin.register(host)
        ConnectivityPlugin.register(host)
        FileSystemPlugin.register(host)
        HapticsPlugin.register(host)
        LinkingPlugin.register(host)
        PermissionsPlugin.register(host)
        SharePlugin.register(host)
        SystemInfoPlugin.register(host)
    }
}
