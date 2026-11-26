package dev.impasto.host

/** Installs a platform plugin for one activity's host on the main thread. */
fun interface ImpastoAndroidPlugin {
    fun register(host: ImpastoHost)
}
