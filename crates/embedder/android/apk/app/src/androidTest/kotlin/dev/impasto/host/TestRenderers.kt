package dev.impasto.host

import androidx.test.platform.app.InstrumentationRegistry

internal fun testRenderers(): List<ImpastoHost.Renderer> =
    when (InstrumentationRegistry.getArguments().getString("impastoTestVulkan", "true")) {
        "true" -> listOf(ImpastoHost.Renderer.VULKAN, ImpastoHost.Renderer.GLES)
        "false" -> listOf(ImpastoHost.Renderer.GLES)
        else -> error("impastoTestVulkan must be true or false")
    }
