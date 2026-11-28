package dev.impasto.host

import android.net.LocalServerSocket
import android.net.LocalSocket
import java.io.Closeable
import java.io.IOException
import java.net.InetAddress
import java.net.Socket
import java.util.concurrent.atomic.AtomicBoolean

/** Relays an authenticated ADB peer into the Rust development endpoint. */
internal class DevelopmentUpdates(packageName: String, private val port: Int) : Closeable {
    private val server = LocalServerSocket("impasto-dev-v2-$packageName")
    private val closed = AtomicBoolean(false)
    @Volatile private var client: LocalSocket? = null
    @Volatile private var endpoint: Socket? = null

    init {
        background("impasto-development-adb", ::listen)
    }

    private fun listen() {
        while (!closed.get()) {
            val socket = try {
                server.accept()
            } catch (_: IOException) {
                return
            }
            socket.use {
                client = socket
                try {
                    if (!closed.get() && socket.peerCredentials.uid in ADB_UIDS) {
                        relay(socket)
                    }
                } catch (_: IOException) {
                    // EOF wakes both halves. ADB can then open a fresh stream.
                } finally {
                    client = null
                }
            }
        }
    }

    private fun relay(client: LocalSocket) {
        Socket(InetAddress.getByName("127.0.0.1"), port).use { target ->
            endpoint = target
            target.tcpNoDelay = true
            if (closed.get()) return
            val upload = background("impasto-development-adb-input") {
                try {
                    client.inputStream.copyTo(target.getOutputStream())
                } catch (_: IOException) {
                    // Either side may close during a transfer.
                } finally {
                    runCatching { target.close() }
                }
            }
            try {
                target.getInputStream().copyTo(client.outputStream)
            } finally {
                runCatching { client.close() }
                upload.join()
                endpoint = null
            }
        }
    }

    override fun close() {
        if (closed.getAndSet(true)) return
        runCatching { client?.close() }
        runCatching { endpoint?.close() }
        runCatching { server.close() }
    }

    private companion object {
        val ADB_UIDS = setOf(0, 2000) // root and Android's shell user
        fun background(name: String, run: () -> Unit): Thread =
            Thread({ run() }, name).apply { isDaemon = true; start() }
    }
}
