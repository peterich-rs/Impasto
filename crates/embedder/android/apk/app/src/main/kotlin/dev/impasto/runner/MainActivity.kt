package dev.impasto.runner

import dev.impasto.host.ImpastoActivity
import dev.impasto.host.ImpastoHost
import dev.impasto.plugins.filesystem.FileSystemPlugin
import dev.impasto.plugins.linking.LinkingPlugin
import dev.impasto.plugins.permissions.PermissionsPlugin

class MainActivity : ImpastoActivity() {
    override fun registerPlugins(host: ImpastoHost) {
        FileSystemPlugin.register(host)
        LinkingPlugin.register(host)
        PermissionsPlugin.register(host)
    }
}
