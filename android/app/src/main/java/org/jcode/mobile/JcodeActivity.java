package org.jcode.mobile;

import android.app.NativeActivity;
import android.content.Intent;
import android.os.Bundle;

/** NativeActivity lifecycle adapter only. The shared GPUI app owns all UI. */
public final class JcodeActivity extends NativeActivity {
    static { System.loadLibrary("jcode_mobile"); }

    private AndroidServices services;
    private boolean resumed;
    private static native void nativeConfigureDebugDemo(String mode);

    @Override public void onCreate(Bundle state) {
        // Only debug APKs expose this shared Rust demo route. Release strips the call.
        if (BuildConfig.DEBUG && getIntent() != null) {
            String demo = getIntent().getStringExtra("gpui_demo");
            if (demo != null) nativeConfigureDebugDemo(demo);
        }
        // Install app-private storage/services before NativeActivity starts Rust.
        services = new AndroidServices(this);
        super.onCreate(state);
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
