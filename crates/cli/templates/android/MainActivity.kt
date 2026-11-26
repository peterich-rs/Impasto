package dev.impasto.runner

import dev.impasto.host.ImpastoActivity
import dev.impasto.host.ImpastoHost

class MainActivity : ImpastoActivity() {
    override fun registerPlugins(host: ImpastoHost) {
        GeneratedPluginRegistrant.register(host)
    }
}
