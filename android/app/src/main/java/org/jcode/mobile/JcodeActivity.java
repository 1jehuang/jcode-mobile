package org.jcode.mobile;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import android.os.Looper;
import android.view.ViewGroup;
import java.util.concurrent.Callable;
import java.util.concurrent.FutureTask;
import java.util.concurrent.TimeUnit;

/** NativeActivity lifecycle adapter only. The shared GPUI app owns all UI. */
public final class JcodeActivity extends NativeActivity {
    static { System.loadLibrary("jcode_mobile"); }

    private AndroidServices services;
    private GpuiInput input;
    private native void nativeInitInput();
    private boolean resumed;
    private static native void nativeConfigureDebugDemo(String mode);
    private static native void nativeWindowInsets(int left, int top, int right, int bottom,
            int imeLeft, int imeTop, int imeRight, int imeBottom);

    /** Isolate newer framework types so API 26 can still load the Activity. */
    private static final class Api30 {
        static void install(JcodeActivity activity) {
            android.view.View decor = activity.getWindow().getDecorView();
            decor.setOnApplyWindowInsetsListener((view, insets) -> {
                android.graphics.Insets safe = insets.getInsets(
                        android.view.WindowInsets.Type.systemBars()
                        | android.view.WindowInsets.Type.displayCutout());
                android.graphics.Insets ime = insets.getInsets(android.view.WindowInsets.Type.ime());
                nativeWindowInsets(safe.left, safe.top, safe.right, safe.bottom,
                        ime.left, ime.top, ime.right, ime.bottom);
                // Observe, do not consume or bypass DecorView's normal inset handling.
                return view.onApplyWindowInsets(insets);
            });
            decor.requestApplyInsets();
        }
    }

    @Override public void onCreate(Bundle state) {
        // Only debug APKs expose this shared Rust demo route. Release strips the call.
        if (BuildConfig.DEBUG && getIntent() != null) {
            String demo = getIntent().getStringExtra("gpui_demo");
            if (demo != null) nativeConfigureDebugDemo(demo);
        }
        // Install app-private storage/services before NativeActivity starts Rust.
        nativeInitInput();
        services = new AndroidServices(this);
        super.onCreate(state);
        // A non-drawing one-pixel IME endpoint, not an application widget.
        input = new GpuiInput(this);
        addContentView(input, new ViewGroup.LayoutParams(1, 1));
        if (android.os.Build.VERSION.SDK_INT >= 30) Api30.install(this);
    }

    private <T> T gpuiUi(Callable<T> action, T fallback) {
        if (isFinishing() || isDestroyed()) return fallback;
        if (Looper.myLooper() == Looper.getMainLooper()) {
            try { return action.call(); } catch (Exception ignored) { return fallback; }
        }
        FutureTask<T> task = new FutureTask<>(action);
        runOnUiThread(task);
        try { return task.get(3, TimeUnit.SECONDS); }
        catch (Exception ignored) { task.cancel(false); return fallback; }
    }

    public boolean gpuiKeyboardVisible(boolean visible) {
        return gpuiUi(() -> {
            if (input == null) return false;
            input.setKeyboardVisible(visible);
            return true;
        }, false);
    }
    public boolean gpuiTextInputConfiguration(int inputType, int imeOptions, String autofillHint) {
        return gpuiUi(() -> {
            if (input == null) return false;
            input.setTextInputConfiguration(inputType, imeOptions, autofillHint);
            return true;
        }, false);
    }
    public boolean gpuiOpenUrl(String url) {
        return services != null && services.openUrl(url);
    }
    public String gpuiReadClipboard() {
        return gpuiUi(() -> {
            ClipboardManager manager = (ClipboardManager)getSystemService(Context.CLIPBOARD_SERVICE);
            ClipData clip = manager == null ? null : manager.getPrimaryClip();
            if (clip == null || clip.getItemCount() == 0) return null;
            CharSequence text = clip.getItemAt(0).coerceToText(this);
            return text == null ? null : text.toString();
        }, null);
    }
    public boolean gpuiWriteClipboard(String text) {
        return gpuiUi(() -> {
            ClipboardManager manager = (ClipboardManager)getSystemService(Context.CLIPBOARD_SERVICE);
            if (manager == null) return false;
            manager.setPrimaryClip(ClipData.newPlainText("Jcode", text));
            return true;
        }, false);
    }

    @Override protected void onResume() {
        super.onResume();
        resumed = true;
        if (services != null) services.onResume(getIntent());
    }

    @Override protected void onPause() {
        resumed = false;
        if (services != null) services.onPause();
        super.onPause();
    }

    @Override protected void onDestroy() {
        if (services != null) services.onDestroy();
        super.onDestroy();
    }

    @Override public void onConfigurationChanged(android.content.res.Configuration configuration) {
        super.onConfigurationChanged(configuration);
        if (services != null) services.onConfigurationChanged();
    }

    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (services != null) services.onRequestPermissionsResult(request);
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        if (services == null || !services.onActivityResult(request, result, data)) {
            super.onActivityResult(request, result, data);
        }
    }

    @Override protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        if (resumed && services != null) services.onResume(intent);
    }
}
