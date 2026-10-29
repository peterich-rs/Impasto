package dev.uifw.host;

import android.app.Activity;
import android.content.res.Configuration;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.view.View;
import android.view.WindowManager;
import android.window.OnBackInvokedCallback;
import android.window.OnBackInvokedDispatcher;

/**
 * Window host. Lifecycle, focus, and the system back key land here.
 * The view owns the surface, touch, and the input connection.
 */
public final class HostActivity extends Activity {
    static {
        System.loadLibrary("uifw");
    }

    private HostView view;
    private boolean attached;
    private Object backCallback;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        layoutBehindSystemBars();
        view = new HostView(this);
        int code = nativeAttach(
                this,
                view,
                getAssets(),
                scriptExtra(),
                launchUrl(),
                locale(),
                textScale(),
                dark(),
                reduceMotion(),
                boldText(),
                highContrast());
        if (code != 0) {
            finish();
            return;
        }
        attached = true;
        setContentView(view);
        registerBack();
        registerComponentCallbacks(new android.content.ComponentCallbacks2() {
            @Override
            public void onTrimMemory(int level) {
                if (attached) {
                    nativeTrimMemory();
                }
            }

            @Override
            public void onConfigurationChanged(android.content.res.Configuration newConfig) {}

            @Override
            public void onLowMemory() {
                if (attached) {
                    nativeTrimMemory();
                }
            }
        });
    }

    @Override
    protected void onResume() {
        super.onResume();
        if (attached) {
            nativeResume();
        }
    }

    @Override
    protected void onPause() {
        if (attached) {
            nativeInactive();
        }
        super.onPause();
    }

    @Override
    protected void onStop() {
        if (attached) {
            nativeHidden();
        }
        super.onStop();
    }

    @Override
    @SuppressWarnings("deprecation")
    public void onTrimMemory(int level) {
        super.onTrimMemory(level);
        if (attached) {
            nativeTrimMemory();
        }
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (attached) {
            nativeFocus(hasFocus);
        }
    }

    @Override
    public void onConfigurationChanged(Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        if (attached) {
            nativeSettings(locale(), textScale(), dark(), reduceMotion(), boldText(), highContrast());
        }
    }

    @Override
    public void onBackPressed() {
        if (attached) {
            nativeBack();
            return;
        }
        super.onBackPressed();
    }

    @Override
    protected void onDestroy() {
        if (attached) {
            attached = false;
            nativeDetach();
        }
        super.onDestroy();
    }

    private void layoutBehindSystemBars() {
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
        } else {
            getWindow().getDecorView().setSystemUiVisibility(
                    View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                            | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                            | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_DRAWS_SYSTEM_BAR_BACKGROUNDS);
    }

    private void registerBack() {
        if (Build.VERSION.SDK_INT < 33) {
            return;
        }
        OnBackInvokedCallback callback = HostActivity::nativeBack;
        backCallback = callback;
        getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                OnBackInvokedDispatcher.PRIORITY_DEFAULT, callback);
    }

    private String scriptExtra() {
        if (getIntent() == null) {
            return null;
        }
        return getIntent().getStringExtra("uifw.script");
    }

    private String launchUrl() {
        if (getIntent() == null) {
            return null;
        }
        android.net.Uri data = getIntent().getData();
        if (data == null) {
            return null;
        }
        String url = data.toString();
        return url.isEmpty() ? null : url;
    }

    private String locale() {
        return getResources().getConfiguration().getLocales().get(0).toLanguageTag();
    }

    private float textScale() {
        return getResources().getConfiguration().fontScale;
    }

    private boolean dark() {
        int night = getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK;
        return night == Configuration.UI_MODE_NIGHT_YES;
    }

    private boolean reduceMotion() {
        float scale = Settings.Global.getFloat(
                getContentResolver(), Settings.Global.ANIMATOR_DURATION_SCALE, 1f);
        return scale == 0f;
    }

    private boolean boldText() {
        if (Build.VERSION.SDK_INT < 31) {
            return false;
        }
        return getResources().getConfiguration().fontWeightAdjustment >= 300;
    }

    private boolean highContrast() {
        return Settings.Secure.getInt(getContentResolver(), "high_text_contrast_enabled", 0) == 1;
    }

    private static native int nativeAttach(
            Activity activity,
            HostView view,
            android.content.res.AssetManager assets,
            String script,
            String launchUrl,
            String locale,
            float textScale,
            boolean dark,
            boolean reduceMotion,
            boolean boldText,
            boolean highContrast);

    private static native void nativeDetach();

    private static native void nativeResume();

    private static native void nativeInactive();

    private static native void nativeHidden();

    private static native void nativeTrimMemory();

    private static native void nativeFocus(boolean focused);

    private static native void nativeBack();

    private static native void nativeSettings(
            String locale,
            float textScale,
            boolean dark,
            boolean reduceMotion,
            boolean boldText,
            boolean highContrast);
}
