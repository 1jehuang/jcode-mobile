package org.jcode.mobile.nativeservicestests;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.os.Bundle;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.concurrent.FutureTask;

/** Bounded production IME readiness bookkeeping, not a native prefix-delivery claim. */
public final class ColdInputReadinessInstrumentation extends Instrumentation {
    @Override public void onCreate(Bundle args) { super.onCreate(args); start(); }

    private static Field field(Class<?> type, String name) throws Exception {
        Field field = type.getDeclaredField(name); field.setAccessible(true); return field;
    }

    private void verify() throws Exception {
        Context context = getTargetContext();
        StorageInstrumentation.check("org.jcode.mobile".equals(context.getPackageName()), "Wrong readiness target");
        StorageInstrumentation.check((context.getApplicationInfo().flags & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0,
                "Debug target required for isolated readiness bookkeeping checks");
        // Load the production JNI library before closeConnection emits FINISH.
        // No Activity is launched and no application text/keyboard is injected.
        Class.forName("org.jcode.mobile.JcodeActivity", true, context.getClassLoader());
        Class<?> type = context.getClassLoader().loadClass("org.jcode.mobile.GpuiInput");
        Constructor<?> constructor = type.getDeclaredConstructor(Context.class); constructor.setAccessible(true);
        View input = (View)constructor.newInstance(context);
        Method ready = type.getDeclaredMethod("isImeConnectionReady"); ready.setAccessible(true);
        Field current = field(type, "currentConnection");
        StorageInstrumentation.check(Boolean.FALSE.equals(ready.invoke(input)), "Detached endpoint without connection reported ready");
        InputConnection previous = input.onCreateInputConnection(new EditorInfo());
        StorageInstrumentation.check(previous != null && current.get(input) == previous, "Fresh connection not tracked");
        StorageInstrumentation.check(Boolean.FALSE.equals(ready.invoke(input)), "Detached endpoint with connection reported ready");
        InputConnection replacement = input.onCreateInputConnection(new EditorInfo());
        StorageInstrumentation.check(replacement != previous && current.get(input) == replacement, "Connection replacement lost identity");
        previous.closeConnection();
        StorageInstrumentation.check(current.get(input) == replacement, "Stale close invalidated newer connection");
        StorageInstrumentation.check(Boolean.FALSE.equals(ready.invoke(input)), "Detached replacement reported ready");
        replacement.closeConnection();
        StorageInstrumentation.check(current.get(input) == null, "Current close retained readiness connection");
        StorageInstrumentation.check(Boolean.FALSE.equals(ready.invoke(input)), "Closed endpoint reported ready");
    }

    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            FutureTask<Void> task = new FutureTask<>(() -> { verify(); return null; });
            runOnMainSync(task); task.get();
            result.putString("readiness", "PASS: detached and closed endpoints reject readiness; stale close preserves replacement identity. Not native handler or first-character acceptance.");
            result.putString("result", "PASS");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putString("result", "FAIL");
            result.putString("failure", android.util.Log.getStackTraceString(error));
            finish(Activity.RESULT_CANCELED, result);
        }
    }
}
