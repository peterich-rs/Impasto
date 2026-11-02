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
    private long lastFrameNanos;
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
                long target = data.getPreferredFrameTimeline().getExpectedPresentationTimeNanos();
                nativeOnVsync(frame, target, frameInterval(frame));
            });
        } else {
            Choreographer.getInstance().postFrameCallback(frame -> {
                posted = false;
                long interval = frameInterval(frame);
                nativeOnVsync(frame, frame + interval, interval);
            });
        }
    }

    private long frameInterval(long frame) {
        long interval = intervalNanos;
        if (lastFrameNanos > 0 && frame > lastFrameNanos) {
            long delta = frame - lastFrameNanos;
            if (delta > 1_000_000L && delta < 100_000_000L) {
                interval = delta;
            }
        }
        lastFrameNanos = frame;
        return interval;
    }

    private static native void nativeOnVsync(long frameTimeNanos, long targetNanos, long intervalNanos);
}
