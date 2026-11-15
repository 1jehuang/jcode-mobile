package org.jcode.mobile.nativeservicestests;

import android.app.Activity;
import android.app.Instrumentation;
import android.app.NativeActivity;
import android.app.UiAutomation;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.SurfaceView;
import android.view.View;
import android.widget.FrameLayout;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityNodeProvider;
import android.view.accessibility.AccessibilityManager;
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
    private View nativeHost(View content) throws Exception {
        Class<?> adapter = activity.getClassLoader().loadClass("org.jcode.mobile.GpuiAccessibility");
        java.lang.reflect.Method discover = adapter.getDeclaredMethod("findNativeHost", View.class);
        discover.setAccessible(true);
        return (View)discover.invoke(null, content);
    }
    private void checkHostDiscovery() throws Exception {
        ui(() -> {
            check(nativeHost(null) == null, "Null content selected as host");
            FrameLayout detached = new FrameLayout(activity);
            detached.addView(new View(activity));
            check(nativeHost(detached) == null, "Generic content/container selected as native host");
            check(nativeHost((View)field(activity, "input")) == null, "IME endpoint selected as native host");
            SurfaceView alternate = new SurfaceView(activity);
            detached.addView(alternate);
            check(nativeHost(detached) == alternate, "Alternate SurfaceView host not found");
            View host = nativeHost(activity.findViewById(android.R.id.content));
            check(host != null && "android.app.NativeActivity$NativeContentView".equals(host.getClass().getName()),
                    "Actual NativeActivity takeSurface owner not found");
            check(host != activity.getWindow().getDecorView() && host != field(activity, "input"), "Unsafe host selected");
            return null;
        });
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
    private void awaitAccessibility(AccessibilityManager manager, boolean expected) throws Exception {
        for (int attempt = 0; attempt < 100; attempt++) {
            if (ui(() -> manager.isEnabled()) == expected) return;
            SystemClock.sleep(50);
        }
        throw new AssertionError("Temporary accessibility connection did not reach/restore expected enabled state");
    }
    private int awaitEmptyNativeField() throws Exception {
        for (int attempt = 0; attempt < 200; attempt++) {
            int candidate = ui(() -> {
                // These are production snapshot IDs, not injected/synthetic nodes.
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
                return 0;
            });
            if (candidate != 0) return candidate;
            SystemClock.sleep(50);
        }
        throw new AssertionError("UNTESTABLE: no empty native editable demo field observable within 10 seconds with real accessibility enabled. No IME success claimed.");
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Throwable failure = null;
        AccessibilityManager accessibility = (AccessibilityManager)getTargetContext().getSystemService(Context.ACCESSIBILITY_SERVICE);
        boolean originalAccessibility = false;
        boolean automationRequested = false;
        try {
            check("org.jcode.mobile".equals(getTargetContext().getPackageName()), "Wrong instrumentation target");
            check((getTargetContext().getApplicationInfo().flags & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0, "Debug APK required");
            // A real, temporary framework accessibility connection drives nativeStatus.
            // Preserve existing accessibility services, and never force production state.
            originalAccessibility = ui(() -> accessibility.isEnabled());
            automationRequested = true;
            UiAutomation automation = getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES);
            check(automation != null, "Cannot establish real instrumentation accessibility connection");
            awaitAccessibility(accessibility, true);
            Intent launch = new Intent(Intent.ACTION_MAIN).setClassName("org.jcode.mobile", "org.jcode.mobile.JcodeActivity")
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK).putExtra("gpui_demo", "todos");
            activity = startActivitySync(launch);
            check(activity instanceof NativeActivity, "Not the real NativeActivity");
            checkHostDiscovery();
            result.putString("host", "PASS: production discovery finds NativeContentView, rejects IME/generic containers, supports alternate SurfaceView");
            for (int n = 0; n < 100 && provider == null; n++) {
                provider = ui(() -> { View host = nativeHost(activity.findViewById(android.R.id.content)); return host == null ? null : host.getAccessibilityNodeProvider(); });
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
            fieldId = awaitEmptyNativeField();
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
            try {
                if (automationRequested) {
                    // UiAutomation.disconnect/destroy are hidden SDK APIs. Changing flags
                    // uses Instrumentation's public API to disconnect accessibility first.
                    // finish() below performs the framework-owned final disconnection.
                    check(getUiAutomation(UiAutomation.FLAG_DONT_USE_ACCESSIBILITY
                            | UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES) != null,
                            "Cannot release temporary accessibility connection");
                    awaitAccessibility(accessibility, originalAccessibility);
                    result.putString("accessibility_connection", "PASS: real temporary UiAutomation accessibility enabled and original enabled state restored");
                }
            } catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
        }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", android.util.Log.getStackTraceString(failure));
        finish(failure == null ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
}
