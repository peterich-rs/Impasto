package dev.impasto.host;

import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.view.Choreographer;
import android.view.Display;
import android.view.View;

/**
 * One frame callback for each armed vsync. The callback runs on the UI thread.
 */
public final class VsyncWaiter {
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final View view;
    private boolean posted;

    VsyncWaiter(View view) {
        this.view = view;
    }

    public void schedule() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            handler.post(this::schedule);
            return;
        }
        if (posted) {
            return;
        }
        posted = true;
        if (Build.VERSION.SDK_INT >= 33) {
            Choreographer.getInstance().postVsyncCallback(data -> {
                posted = false;
                long frame = data.getFrameTimeNanos();
                long target = data.getPreferredFrameTimeline().getExpectedPresentationTimeNanos();
                nativeOnVsync(frame, target, frameInterval());
            });
        } else {
            Choreographer.getInstance().postFrameCallback(frame -> {
                posted = false;
                long interval = frameInterval();
                nativeOnVsync(frame, frame + interval, interval);
            });
        }
    }

    private long frameInterval() {
        // Callback gaps can contain missed vsyncs or time spent idle.
        Display display = view.getDisplay();
        float fps = display == null ? 60f : display.getRefreshRate();
        return (long) (1_000_000_000.0 / (fps > 1f ? fps : 60f));
    }

    private static native void nativeOnVsync(long frameTimeNanos, long targetNanos, long intervalNanos);
}
