package org.jcode.mobile.nativeservicestests;

import android.app.Activity;
import android.app.Instrumentation;
import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewGroup;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityNodeProvider;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import java.lang.reflect.Field;
import java.util.concurrent.Callable;
import java.util.concurrent.FutureTask;

/** No Java app UI. Operates on the installed GPUI NativeActivity and native semantic tree. */
public final class NativeInstrumentation extends Instrumentation {
    private Activity activity;
    private AccessibilityNodeProvider provider;
    private int fieldId;
    private String original;
    private ClipData previousClip;
    private boolean clipboardSaved;
    private ClipboardManager clipboard;
    @Override public void onCreate(Bundle args) { super.onCreate(args); start(); }
    private static void check(boolean value, String message) { StorageInstrumentation.check(value, message); }
    private <T> T ui(Callable<T> action) throws Exception {
        FutureTask<T> task = new FutureTask<>(action); runOnMainSync(task); return task.get();
    }
    private static Object field(Object object, String name) throws Exception {
        Field field = object.getClass().getDeclaredField(name); field.setAccessible(true); return field.get(object);
    }
    private static View surface(View view) {
        if (view instanceof SurfaceView) return view;
        if (view instanceof ViewGroup) for (int i = 0; i < ((ViewGroup)view).getChildCount(); i++) {
            View found = surface(((ViewGroup)view).getChildAt(i)); if (found != null) return found;
        }
        return null;
    }
    private void awaitText(String expected) throws Exception {
        for (int attempt = 0; attempt < 100; attempt++) {
            String value = ui(() -> {
                AccessibilityNodeInfo node = provider.createAccessibilityNodeInfo(fieldId);
                return node == null || node.getText() == null ? "" : node.getText().toString();
            });
            if (expected.equals(value)) return;
            SystemClock.sleep(50);
        }
        throw new AssertionError("Native GPUI field did not expose expected text after InputConnection edit (values redacted)");
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Throwable failure = null;
        try {
            check("org.jcode.mobile".equals(getTargetContext().getPackageName()), "Wrong instrumentation target");
            check((getTargetContext().getApplicationInfo().flags & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0, "Debug APK required");
            Intent launch = new Intent(Intent.ACTION_MAIN).setClassName("org.jcode.mobile", "org.jcode.mobile.JcodeActivity")
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK).putExtra("gpui_demo", "todos");
            activity = startActivitySync(launch);
            check(activity instanceof NativeActivity, "Not the real NativeActivity");
            for (int n = 0; n < 100 && provider == null; n++) {
                provider = ui(() -> { View host = surface(activity.getWindow().getDecorView()); return host == null ? null : host.getAccessibilityNodeProvider(); });
                if (provider == null) SystemClock.sleep(50);
            }
            clipboard = (ClipboardManager)activity.getSystemService(Context.CLIPBOARD_SERVICE);
            ui(() -> {
                previousClip = clipboard.getPrimaryClip(); clipboardSaved = true;
                String marker = "jcode-native-clipboard-" + java.util.UUID.randomUUID();
                check(Boolean.TRUE.equals(activity.getClass().getMethod("gpuiWriteClipboard", String.class).invoke(activity, marker)), "Native host clipboard write rejected");
                check(marker.equals(activity.getClass().getMethod("gpuiReadClipboard").invoke(activity)), "Native host clipboard roundtrip mismatch");
                return null;
            });
            result.putString("clipboard", "PASS: real host copy/read marker roundtrip, restored in cleanup");
            // Exercise adapter lifecycle callbacks on the real host, without sending an Intent/deep link.
            ui(() -> {
                Object services = field(activity, "services");
                check(Boolean.TRUE.equals(field(services, "foreground")), "Services did not resume");
                callActivityOnPause(activity);
                check(Boolean.FALSE.equals(field(services, "foreground")), "Services did not pause");
                check(field(services, "recorder") == null, "Pause retained audio recorder");
                callActivityOnResume(activity);
                check(Boolean.TRUE.equals(field(services, "foreground")), "Services did not resume again");
                return null;
            });
            result.putString("lifecycle", "PASS: real Activity pause/resume callbacks and service foreground/recorder state (not OS process recreation)");
            check(provider != null, "UNTESTABLE: native accessibility provider unavailable. No native-field IME success claimed.");
            fieldId = ui(() -> {
                // The provider publishes stable IDs internally. Select only an empty, non-password
                // editable demo field, so no existing user draft or credential is overwritten.
                Field itemsField = provider.getClass().getDeclaredField("items"); itemsField.setAccessible(true);
                java.util.Map<?, ?> items = (java.util.Map<?, ?>)itemsField.get(provider);
                for (Object key : items.keySet()) {
                    int id = ((Integer)key).intValue(); AccessibilityNodeInfo node = provider.createAccessibilityNodeInfo(id);
                    if (node != null && node.isEditable() && !node.isPassword() && (node.getText() == null || node.getText().length() == 0)) {
                        check(provider.performAction(id, AccessibilityNodeInfo.ACTION_FOCUS, null), "Native field focus rejected");
                        original = "";
                        return id;
                    }
                }
                throw new AssertionError("UNTESTABLE: no empty native editable demo field observable. No IME success claimed.");
            });
            SystemClock.sleep(200);
            InputConnection connection = ui(() -> {
                View input = (View)field(activity, "input");
                check(input.getClass().getName().equals("org.jcode.mobile.GpuiInput"), "Not production GPUI input endpoint");
                return input.onCreateInputConnection(new EditorInfo());
            });
            check(connection != null, "No production InputConnection");
            ui(() -> { check(connection.setComposingText("draft", 1), "Compose rejected"); return null; });
            awaitText("draft");
            ui(() -> { check(connection.setComposingText("draftZ", 1), "Composition replacement rejected"); return null; });
            awaitText("draftZ");
            ui(() -> { check(connection.commitText("native", 1), "Commit rejected"); return null; });
            awaitText("native");
            ui(() -> { check(connection.deleteSurroundingText(1, 0), "Delete rejected"); return null; });
            awaitText("nativ");
            ui(() -> { check(connection.finishComposingText(), "Finish composition rejected"); connection.closeConnection(); return null; });
            result.putString("ime", "PASS: actual GPUI field observed after compose/replace/commit/delete through production InputConnection");
        } catch (Throwable error) { failure = error; }
        finally {
            try {
                if (activity != null && provider != null && original != null) {
                    ui(() -> {
                        Bundle args = new Bundle(); args.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, original);
                        check(provider.performAction(fieldId, AccessibilityNodeInfo.ACTION_SET_TEXT, args), "Cannot restore test field");
                        return null;
                    });
                    awaitText(original);
                }
            } catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
            try {
                if (activity != null && clipboardSaved) ui(() -> {
                    if (previousClip != null) clipboard.setPrimaryClip(previousClip);
                    else if (android.os.Build.VERSION.SDK_INT >= 28) clipboard.clearPrimaryClip();
                    else throw new AssertionError("Cannot restore empty clipboard before API 28");
                    return null;
                });
            } catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
        }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", android.util.Log.getStackTraceString(failure));
        finish(failure == null ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
}
