package dev.impasto.plugins.camera

import dev.impasto.host.ImpastoAndroidPlugin
import dev.impasto.host.ImpastoHost

/** The Rust camera session opens the bridge on demand through the host's platform view. */
object CameraPlugin : ImpastoAndroidPlugin {
    override fun register(host: ImpastoHost) {
        CameraBridge
    }
}
