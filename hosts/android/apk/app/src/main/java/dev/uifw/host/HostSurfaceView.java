package dev.uifw.host;

import android.content.Context;
import android.graphics.PixelFormat;
import android.graphics.Region;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;

/**
 * The Vulkan window. Touch and the input connection stay on {@link HostView}.
 * {@code surfaceDestroyed} blocks until the render thread drops the window.
 */
public final class HostSurfaceView extends SurfaceView implements SurfaceHolder.Callback {
    public HostSurfaceView(Context context) {
        super(context);
        setFocusable(false);
        setFocusableInTouchMode(false);
        setClickable(false);
        getHolder().setFormat(PixelFormat.OPAQUE);
        getHolder().addCallback(this);
    }

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        Surface surface = holder.getSurface();
        if (surface != null && surface.isValid()) {
            nativeSurfaceCreated(surface);
        }
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        nativeSurfaceChanged(width, height);
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        nativeSurfaceDestroyed();
    }

    @Override
    public boolean gatherTransparentRegion(Region region) {
        if (getAlpha() < 1f) {
            return false;
        }
        int[] location = new int[2];
        getLocationInWindow(location);
        region.op(
                location[0],
                location[1],
                location[0] + getRight() - getLeft(),
                location[1] + getBottom() - getTop(),
                Region.Op.DIFFERENCE);
        return true;
    }

    private static native void nativeSurfaceCreated(Surface surface);

    private static native void nativeSurfaceChanged(int width, int height);

    private static native void nativeSurfaceDestroyed();
}
