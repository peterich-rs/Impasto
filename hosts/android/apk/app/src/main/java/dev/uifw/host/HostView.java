package dev.uifw.host;

import android.app.Activity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.graphics.Insets;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.ScaleGestureDetector;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsAnimation;
import android.view.accessibility.AccessibilityNodeProvider;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.widget.FrameLayout;

import java.util.List;

/**
 * The content view. It receives pointers, keys, insets, and creates the
 * input connection. The child surface is only the window.
 */
public final class HostView extends FrameLayout {
    private final HostSurfaceView surface;
    private final TextInput text;
    private final VsyncWaiter vsync;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final AccessBridge access = new AccessBridge();
    private final ScaleGestureDetector scale;
    private float density = 1f;
    private WindowInsets deferredInsets;
    private boolean imeAnimating;
    private boolean saveImeInset;

    public HostView(Context context) {
        super(context);
        density = context.getResources().getDisplayMetrics().density;
        surface = new HostSurfaceView(context);
        text = new TextInput(this);
        android.view.Display display = null;
        if (Build.VERSION.SDK_INT >= 30 && context instanceof Activity activity) {
            display = activity.getDisplay();
        }
        vsync = new VsyncWaiter(display);
        scale = new ScaleGestureDetector(context, new ScaleGestureDetector.SimpleOnScaleGestureListener() {
            @Override
            public boolean onScale(ScaleGestureDetector detector) {
                nativeZoom(
                        detector.getScaleFactor(),
                        detector.getFocusX(),
                        detector.getFocusY(),
                        SystemClock.uptimeMillis(),
                        density);
                return true;
            }
        });
        setFocusable(true);
        setFocusableInTouchMode(true);
        setClickable(true);
        setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_YES);
        addView(surface, new LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT));
        access.attach(this);
        if (Build.VERSION.SDK_INT >= 30) {
            installImeAnimation();
        }
    }

    @Override
    protected void onAttachedToWindow() {
        super.onAttachedToWindow();
        requestApplyInsets();
    }

    float density() {
        return density;
    }

    public void scheduleFrame() {
        vsync.schedule();
    }

    public void finishHost() {
        runOnUi(() -> {
            Context context = getContext();
            if (context instanceof Activity activity) {
                activity.finish();
            }
        });
    }

    public void openUrl(String url) {
        runOnUi(() -> {
            if (url == null || url.isEmpty()) {
                return;
            }
            android.content.Intent intent = new android.content.Intent(
                    android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url));
            intent.addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK);
            getContext().startActivity(intent);
        });
    }

    public String documentDirectory() {
        return getContext().getFilesDir().getAbsolutePath();
    }

    public String readClipboard() {
        ClipboardManager manager = (ClipboardManager) getContext().getSystemService(Context.CLIPBOARD_SERVICE);
        if (manager == null || !manager.hasPrimaryClip() || manager.getPrimaryClip() == null) {
            return "";
        }
        ClipData.Item item = manager.getPrimaryClip().getItemAt(0);
        CharSequence value = item.getText();
        return value == null ? "" : value.toString();
    }

    public void writeClipboardNow(String text) {
        ClipboardManager manager = (ClipboardManager) getContext().getSystemService(Context.CLIPBOARD_SERVICE);
        if (manager != null) {
            manager.setPrimaryClip(ClipData.newPlainText("uifw", text == null ? "" : text));
        }
    }

    public void postNative(final long work) {
        handler.post(() -> nativeRunWork(work));
    }

    public void publishSemantics(
            long[] ids,
            int[] roles,
            String[] labels,
            String[] values,
            int[] flags,
            int[] actions,
            float[] scroll,
            String[] descriptions,
            long[] parents) {
        runOnUi(() -> {
            access.update(ids, roles, labels, values, flags, actions, scroll, descriptions, parents);
            invalidate();
        });
    }

    public void applyIme(
            int op,
            String text,
            int selStart,
            int selEnd,
            int compStart,
            int compEnd,
            float caretX,
            float caretY,
            float caretW,
            float caretH,
            int inputType,
            int imeAction) {
        runOnUi(() -> this.text.apply(
                op, text, selStart, selEnd, compStart, compEnd,
                caretX, caretY, caretW, caretH, inputType, imeAction));
    }

    @Override
    public AccessibilityNodeProvider getAccessibilityNodeProvider() {
        return access;
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
        InputConnection connection = text.createConnection(outAttrs);
        return connection == null ? super.onCreateInputConnection(outAttrs) : connection;
    }

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        requestUnbufferedDispatch(event);
        scale.onTouchEvent(event);
        emitPointers(event);
        return true;
    }

    @Override
    public boolean onHoverEvent(MotionEvent event) {
        int action = event.getActionMasked();
        if (action == MotionEvent.ACTION_HOVER_ENTER || action == MotionEvent.ACTION_HOVER_MOVE) {
            emitOne(event, 0, 4);
            return true;
        }
        return super.onHoverEvent(event);
    }

    @Override
    public boolean onGenericMotionEvent(MotionEvent event) {
        if (event.getActionMasked() == MotionEvent.ACTION_SCROLL) {
            emitOne(event, 0, 5);
            return true;
        }
        return super.onGenericMotionEvent(event);
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        if (event.getKeyCode() == KeyEvent.KEYCODE_BACK) {
            return super.dispatchKeyEvent(event);
        }
        if (event.getAction() == KeyEvent.ACTION_DOWN && event.getRepeatCount() == 0) {
            getKeyDispatcherState().startTracking(event, this);
        } else if (event.getAction() == KeyEvent.ACTION_UP) {
            getKeyDispatcherState().handleUpEvent(event);
        }
        if (text.clientActive() && event.getAction() == KeyEvent.ACTION_DOWN) {
            if (event.getKeyCode() == KeyEvent.KEYCODE_DEL) {
                text.deleteSurrounding(1, 0);
                return true;
            }
            if (event.getKeyCode() == KeyEvent.KEYCODE_FORWARD_DEL) {
                text.deleteSurrounding(0, 1);
                return true;
            }
        }
        int action = event.getAction() == KeyEvent.ACTION_DOWN ? 0 : 1;
        if (event.getAction() == KeyEvent.ACTION_DOWN || event.getAction() == KeyEvent.ACTION_UP) {
            nativeKey(action, event.getKeyCode(), metaBits(event.getMetaState()), event.getUnicodeChar());
            return true;
        }
        return super.dispatchKeyEvent(event);
    }

    @Override
    public WindowInsets onApplyWindowInsets(WindowInsets insets) {
        WindowInsets result = super.onApplyWindowInsets(insets);
        reportInsets(insets);
        return result;
    }

    @Override
    protected void onSizeChanged(int w, int h, int oldw, int oldh) {
        super.onSizeChanged(w, h, oldw, oldh);
        if (Build.VERSION.SDK_INT >= 30) {
            WindowInsets insets = getRootWindowInsets();
            if (insets != null) {
                reportInsets(insets);
            }
        }
    }

    private void reportInsets(WindowInsets insets) {
        density = getResources().getDisplayMetrics().density;
        int padLeft = 0;
        int padTop = 0;
        int padRight = 0;
        int padBottom = 0;
        int imeLeft = 0;
        int imeTop = 0;
        int imeRight = 0;
        int imeBottom = 0;
        if (Build.VERSION.SDK_INT >= 30) {
            Insets bars = insets.getInsets(WindowInsets.Type.systemBars());
            Insets cutout = insets.getInsets(WindowInsets.Type.displayCutout());
            padLeft = Math.max(bars.left, cutout.left);
            padTop = Math.max(bars.top, cutout.top);
            padRight = Math.max(bars.right, cutout.right);
            padBottom = Math.max(bars.bottom, cutout.bottom);
            Insets ime = insets.getInsets(WindowInsets.Type.ime());
            imeLeft = ime.left;
            imeTop = ime.top;
            imeRight = ime.right;
            imeBottom = ime.bottom;
        } else {
            padTop = insets.getSystemWindowInsetTop();
            padLeft = insets.getSystemWindowInsetLeft();
            padRight = insets.getSystemWindowInsetRight();
            int bottom = insets.getSystemWindowInsetBottom();
            int screen = getRootView().getHeight();
            if (screen > 0 && bottom > screen * 0.18f) {
                imeBottom = bottom;
            } else {
                padBottom = bottom;
            }
        }
        int width = getWidth();
        int height = getHeight();
        if (width <= 0 || height <= 0) {
            return;
        }
        nativeMetrics(
                width, height, density,
                padLeft, padTop, padRight, padBottom,
                imeLeft, imeTop, imeRight, imeBottom);
    }

    private void installImeAnimation() {
        setWindowInsetsAnimationCallback(new WindowInsetsAnimation.Callback(
                WindowInsetsAnimation.Callback.DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
            @Override
            public void onPrepare(WindowInsetsAnimation animation) {
                saveImeInset = true;
                if ((animation.getTypeMask() & WindowInsets.Type.ime()) != 0) {
                    imeAnimating = true;
                }
            }

            @Override
            public WindowInsets onProgress(WindowInsets insets, List<WindowInsetsAnimation> running) {
                if (!imeAnimating || saveImeInset) {
                    return insets;
                }
                boolean matching = false;
                for (WindowInsetsAnimation animation : running) {
                    if ((animation.getTypeMask() & WindowInsets.Type.ime()) != 0) {
                        matching = true;
                    }
                }
                if (!matching) {
                    return insets;
                }
                Insets ime = insets.getInsets(WindowInsets.Type.ime());
                WindowInsets.Builder builder = new WindowInsets.Builder(
                        deferredInsets == null ? insets : deferredInsets);
                builder.setInsets(WindowInsets.Type.ime(), Insets.of(0, 0, 0, Math.max(ime.bottom, 0)));
                HostView.this.onApplyWindowInsets(builder.build());
                return insets;
            }

            @Override
            public void onEnd(WindowInsetsAnimation animation) {
                if (!imeAnimating || (animation.getTypeMask() & WindowInsets.Type.ime()) == 0) {
                    return;
                }
                imeAnimating = false;
                if (deferredInsets != null) {
                    dispatchApplyWindowInsets(deferredInsets);
                }
            }
        });
        setOnApplyWindowInsetsListener((view, insets) -> {
            if (saveImeInset) {
                deferredInsets = insets;
                saveImeInset = false;
            }
            if (imeAnimating) {
                return WindowInsets.CONSUMED;
            }
            return onApplyWindowInsets(insets);
        });
    }

    private void emitPointers(MotionEvent event) {
        int action = event.getActionMasked();
        int phase = phase(action);
        if (phase < 0) {
            return;
        }
        int count = event.getPointerCount();
        int index = event.getActionIndex();
        if (action == MotionEvent.ACTION_MOVE || action == MotionEvent.ACTION_CANCEL) {
            for (int i = 0; i < count; i++) {
                emitOne(event, i, phase);
            }
            return;
        }
        if (index >= 0 && index < count) {
            emitOne(event, index, phase);
        }
    }

    private void emitOne(MotionEvent event, int index, int phase) {
        int kind = 0;
        int tool = event.getToolType(index);
        if (tool == MotionEvent.TOOL_TYPE_MOUSE) {
            kind = 1;
        } else if (tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER) {
            kind = 2;
        }
        int button = 0;
        if (phase == 4) {
            button = 0;
        } else if (kind == 0) {
            button = phase == 3 ? 0 : 1;
        } else {
            int state = event.getButtonState();
            if ((state & MotionEvent.BUTTON_SECONDARY) != 0) {
                button = 2;
            } else if ((state & MotionEvent.BUTTON_TERTIARY) != 0) {
                button = 3;
            } else if ((state & MotionEvent.BUTTON_PRIMARY) != 0 || phase == 0 || phase == 2) {
                button = 1;
            }
        }
        int buttons = 0;
        int state = event.getButtonState();
        if ((state & MotionEvent.BUTTON_PRIMARY) != 0 || (kind == 0 && phase != 3 && phase != 4)) {
            buttons |= 1;
        }
        if ((state & MotionEvent.BUTTON_SECONDARY) != 0) {
            buttons |= 2;
        }
        if ((state & MotionEvent.BUTTON_TERTIARY) != 0) {
            buttons |= 4;
        }
        nativePointer(
                phase,
                event.getPointerId(index),
                event.getX(index),
                event.getY(index),
                event.getEventTime(),
                kind,
                button,
                buttons,
                metaBits(event.getMetaState()),
                event.getAxisValue(MotionEvent.AXIS_HSCROLL, index),
                event.getAxisValue(MotionEvent.AXIS_VSCROLL, index),
                density);
    }

    private static int phase(int action) {
        return switch (action) {
            case MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> 0;
            case MotionEvent.ACTION_MOVE -> 1;
            case MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> 2;
            case MotionEvent.ACTION_CANCEL -> 3;
            case MotionEvent.ACTION_SCROLL -> 5;
            default -> -1;
        };
    }

    private static int metaBits(int meta) {
        int bits = 0;
        if ((meta & KeyEvent.META_SHIFT_ON) != 0) {
            bits |= 1;
        }
        if ((meta & KeyEvent.META_CTRL_ON) != 0) {
            bits |= 2;
        }
        if ((meta & KeyEvent.META_ALT_ON) != 0) {
            bits |= 4;
        }
        if ((meta & KeyEvent.META_META_ON) != 0) {
            bits |= 8;
        }
        return bits;
    }

    private void runOnUi(Runnable runnable) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            runnable.run();
        } else {
            handler.post(runnable);
        }
    }

    private static native void nativeMetrics(
            int width,
            int height,
            float density,
            int padLeft,
            int padTop,
            int padRight,
            int padBottom,
            int imeLeft,
            int imeTop,
            int imeRight,
            int imeBottom);

    private static native void nativePointer(
            int phase,
            int pointerId,
            float x,
            float y,
            long timeMs,
            int kind,
            int button,
            int buttons,
            int modifiers,
            float wheelX,
            float wheelY,
            float density);

    private static native void nativeZoom(float scale, float x, float y, long timeMs, float density);

    private static native void nativeKey(int action, int keyCode, int meta, int unicode);

    private static native void nativeRunWork(long work);
}
