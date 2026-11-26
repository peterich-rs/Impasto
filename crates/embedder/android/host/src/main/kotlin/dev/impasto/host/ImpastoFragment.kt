package dev.impasto.host

import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.fragment.app.Fragment

/** Mounts one Impasto session for the lifetime of a Fragment's view. */
open class ImpastoFragment : Fragment() {
    protected var impastoHost: ImpastoHost? = null
        private set

    protected open val restorationId: String? = null
    protected open val platformViewFactories: Map<String, PlatformViewFactory> = emptyMap()
    protected open fun registerPlugins(host: ImpastoHost) {}

    override fun onCreateView(inflater: LayoutInflater, container: ViewGroup?, state: Bundle?): View =
        FrameLayout(inflater.context)

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)
        val activity = requireActivity()
        val host = ImpastoHost.attach(
            activity, viewLifecycleOwner,
            restorationId = restorationId,
            backDispatcher = activity.onBackPressedDispatcher,
            platformViewFactories = platformViewFactories,
        ) ?: return
        impastoHost = host
        registerPlugins(host)
        (view as FrameLayout).addView(
            host.view, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT),
        )
    }

    override fun onDestroyView() {
        impastoHost?.close()
        impastoHost = null
        super.onDestroyView()
    }
}
