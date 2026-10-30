package dev.impasto.host;

import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.view.Choreographer;
import android.view.Display;

/**
 * One frame callback for each armed vsync. The callback runs on the UI thread.
 */
public final class VsyncWaiter {
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final long intervalNanos;
    private boolean posted;

    public VsyncWaiter(Display display) {
        float fps = display == null ? 60f : display.getRefreshRate();
        if (fps <= 1f) {
            fps = 60f;
        }
        intervalNanos = (long) (1_000_000_000.0 / fps);
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
                Choreographer.FrameTimeline[] lines = data.getFrameTimelines();
                long target = lines.length == 0
                        ? frame + intervalNanos
                        : lines[0].getExpectedPresentationTimeNanos();
                nativeOnVsync(frame, target, intervalNanos);
            });
        } else {
            Choreographer.getInstance().postFrameCallback(frame -> {
                posted = false;
                nativeOnVsync(frame, frame + intervalNanos, intervalNanos);
            });
        }
    }

    private static native void nativeOnVsync(long frameTimeNanos, long targetNanos, long intervalNanos);
}
