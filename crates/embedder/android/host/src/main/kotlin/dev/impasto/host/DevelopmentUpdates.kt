package dev.impasto.host

import android.net.LocalServerSocket
import android.net.LocalSocket
import android.os.SystemClock
import java.io.Closeable
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.File
import java.io.IOException
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

/** A debug-only local socket forwarded by ADB. The kernel checks the peer UID. */
internal class DevelopmentUpdates(
    packageName: String,
    private val directory: File,
    private val submit: (String, ByteArray, (Boolean) -> Unit) -> Unit,
) : Closeable {
    private val server = LocalServerSocket("impasto-hot-v1-$packageName")
    private val closed = AtomicBoolean(false)
    @Volatile private var client: LocalSocket? = null

    init {
        Thread(::listen, "impasto-development-updates").apply {
            isDaemon = true
            start()
        }
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
                    if (!closed.get() && socket.peerCredentials.uid in ADB_UIDS) serve(socket)
                } catch (_: IOException) {
                    // The CLI can reconnect after closing or rebooting the device.
                } finally {
                    client = null
                }
            }
        }
    }

    private fun serve(socket: LocalSocket) {
        val input = DataInputStream(socket.inputStream.buffered())
        val output = DataOutputStream(socket.outputStream)
        output.writeByte(1)
        while (!closed.get()) {
            val revision = input.readLong().toULong().toString()
            val length = input.readInt()
            if (length !in 1..MAX_SOURCE_BYTES) return
            val source = ByteArray(length)
            input.readFully(source)
            val submitted = AtomicReference<Boolean>()
            submit(revision, source, submitted::set)
            val receipt = File(directory, "v1-$revision.result")
            val deadline = SystemClock.elapsedRealtime() + 2_000
            while (!closed.get() && submitted.get() != false &&
                !receipt.isFile && SystemClock.elapsedRealtime() < deadline) {
                Thread.sleep(1)
            }
            val result = when {
                submitted.get() == false -> ByteArray(0)
                receipt.isFile && receipt.length() in 1..MAX_RECEIPT_BYTES ->
                    receipt.readBytes().also { receipt.delete() }
                else -> "error\nThe UI did not confirm the source update".toByteArray()
            }
            output.writeInt(result.size)
            output.write(result)
        }
    }

    override fun close() {
        if (closed.getAndSet(true)) return
        runCatching { client?.close() }
        runCatching { server.close() }
    }

    private companion object {
        val ADB_UIDS = setOf(0, 2000) // root and Android's shell user
        const val MAX_SOURCE_BYTES = 16 * 1024 * 1024
        const val MAX_RECEIPT_BYTES = 1024L * 1024
    }
}
