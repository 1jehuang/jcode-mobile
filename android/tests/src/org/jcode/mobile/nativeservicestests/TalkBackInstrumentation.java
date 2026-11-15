package org.jcode.mobile.nativeservicestests;

import android.accessibilityservice.AccessibilityServiceInfo;
import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.InputDevice;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.accessibility.AccessibilityManager;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityNodeProvider;
import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.Map;
import java.util.concurrent.Callable;
import java.util.concurrent.atomic.AtomicReference;

/** Real installed TalkBack only. Never obtains UiAutomation or injects semantic snapshots. */
@SuppressWarnings("deprecation")
public final class TalkBackInstrumentation extends Instrumentation {
    private Activity activity;
    private AccessibilityNodeProvider provider;
    private int edited = -1;
    private String original;
    private <T> T ui(Callable<T> operation) throws Exception {
        AtomicReference<T> value = new AtomicReference<>();
        AtomicReference<Throwable> error = new AtomicReference<>();
        runOnMainSync(() -> { try { value.set(operation.call()); } catch (Throwable e) { error.set(e); } });
        if (error.get() != null) {
            if (error.get() instanceof Error) throw (Error)error.get();
            throw (Exception)error.get();
        }
        return value.get();
    }
    private static void require(boolean condition, String reason) {
        if (!condition) throw new AssertionError(reason);
    }
    private View host(View view) {
        if (view == null) return null;
        if ("android.app.NativeActivity$NativeContentView".equals(view.getClass().getName())) return view;
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup)view;
            for (int i = 0; i < group.getChildCount(); i++) { View found = host(group.getChildAt(i)); if (found != null) return found; }
        }
        return null;
    }
    private ArrayList<Integer> ids() throws Exception {
        Field field = provider.getClass().getDeclaredField("items");
        field.setAccessible(true);
        ArrayList<Integer> result = new ArrayList<>();
        for (Object key : ((Map<?, ?>)field.get(provider)).keySet()) result.add((Integer)key);
        return result;
    }
    private int focus() throws Exception {
        return ui(() -> {
            for (int id : ids()) {
                AccessibilityNodeInfo node = provider.createAccessibilityNodeInfo(id);
                if (node != null) { boolean focused = node.isAccessibilityFocused(); node.recycle(); if (focused) return id; }
            }
            return -1;
        });
    }
    private void requireTargetWindow() throws Exception {
        // Permission prompts, TalkBack onboarding and other system windows must be
        // handled explicitly by the coordinator. Never dismiss or grant them here.
        for (int attempt = 0; attempt < 100; attempt++) {
            if (ui(() -> activity != null && !activity.isFinishing() && !activity.isDestroyed()
                    && activity.hasWindowFocus()
                    && activity.getWindow().getDecorView().hasWindowFocus())) return;
            SystemClock.sleep(100);
        }
        throw new AssertionError("UNTESTABLE_SETUP_OBSTRUCTION: real target Activity lacks window focus; an external permission/tutorial/system window may be active. Coordinator must inspect and resolve setup explicitly, then rerun. No traversal established.");
    }
    private void swipeRight() throws Exception {
        requireTargetWindow();
        int[] size = ui(() -> new int[] {activity.getWindow().getDecorView().getWidth(), activity.getWindow().getDecorView().getHeight()});
        long down = SystemClock.uptimeMillis();
        for (int i = 0; i <= 12; i++) {
            int action = i == 0 ? MotionEvent.ACTION_DOWN : i == 12 ? MotionEvent.ACTION_UP : MotionEvent.ACTION_MOVE;
            MotionEvent event = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action,
                    size[0] * (0.25f + i * 0.5f / 12), size[1] * 0.5f, 0);
            event.setSource(InputDevice.SOURCE_TOUCHSCREEN);
            try {
                require(ui(() -> activity.hasWindowFocus() && activity.getWindow().getDecorView().hasWindowFocus()),
                        "UNTESTABLE_SETUP_OBSTRUCTION: target window lost focus during gesture; no traversal established");
                sendPointerSync(event);
            } catch (IllegalArgumentException | SecurityException injectionFailure) {
                throw new AssertionError("UNTESTABLE_SETUP_OBSTRUCTION: targeted input injection was rejected, possibly because an external window owns focus. Coordinator must resolve setup explicitly; no traversal established.", injectionFailure);
            } finally { event.recycle(); }
            SystemClock.sleep(15);
        }
    }
    private boolean textEquals(int id, String expected) throws Exception {
        return ui(() -> {
            AccessibilityNodeInfo node = provider.createAccessibilityNodeInfo(id);
            if (node == null) return false;
            boolean matches = expected.contentEquals(node.getText() == null ? "" : node.getText());
            node.recycle(); return matches;
        });
    }
    private void setText(String text) throws Exception {
        ui(() -> {
            Bundle args = new Bundle();
            args.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text);
            require(provider.performAction(edited, AccessibilityNodeInfo.ACTION_SET_TEXT, args), "Native SET_TEXT rejected");
            return null;
        });
        for (int i = 0; i < 100; i++) { if (textEquals(edited, text)) return; SystemClock.sleep(50); }
        throw new AssertionError("Native semantic text did not converge");
    }
    @Override public void onCreate(Bundle args) { super.onCreate(args); start(); }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Throwable failure = null;
        try {
            AccessibilityManager manager = (AccessibilityManager)getTargetContext().getSystemService(Context.ACCESSIBILITY_SERVICE);
            boolean installedEnabled = false;
            for (AccessibilityServiceInfo info : manager.getEnabledAccessibilityServiceList(AccessibilityServiceInfo.FEEDBACK_ALL_MASK)) {
                if ("com.google.android.marvin.talkback".equals(info.getResolveInfo().serviceInfo.packageName)) installedEnabled = true;
            }
            require(installedEnabled && manager.isEnabled() && manager.isTouchExplorationEnabled(), "UNTESTABLE: real installed TalkBack not bound with touch exploration");
            activity = startActivitySync(new Intent().setClassName("org.jcode.mobile", "org.jcode.mobile.JcodeActivity")
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK).putExtra("gpui_demo", "todos"));
            requireTargetWindow();
            for (int i = 0; i < 200; i++) {
                provider = ui(() -> { View view = host(activity.findViewById(android.R.id.content)); return view == null ? null : view.getAccessibilityNodeProvider(); });
                if (provider != null && ui(() -> ids().size()) >= 2) break;
                SystemClock.sleep(50);
            }
            require(provider != null && ui(() -> ids().size()) >= 2, "UNTESTABLE: shared GPUI semantic tree absent");
            // Never call ACTION_ACCESSIBILITY_FOCUS ourselves. An enabled accessibility service must move it.
            int first = -1;
            boolean traversed = false;
            for (int gesture = 0; gesture < 12 && !traversed; gesture++) {
                swipeRight();
                for (int poll = 0; poll < 30; poll++) {
                    int next = focus();
                    if (next != -1) { if (first == -1) first = next; else if (next != first) { traversed = true; break; } }
                    SystemClock.sleep(50);
                }
            }
            require(traversed, "UNTESTABLE: installed TalkBack swipe did not traverse two native semantic nodes");
            result.putString("talkback_traversal", "two native virtual focus targets observed after injected touchscreen swipes");
            edited = ui(() -> {
                for (int id : ids()) {
                    AccessibilityNodeInfo node = provider.createAccessibilityNodeInfo(id);
                    if (node == null) continue;
                    boolean eligible = node.isEditable() && !node.isPassword() && (node.getText() == null || node.getText().length() == 0)
                            && node.getActionList().contains(AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_TEXT);
                    node.recycle(); if (eligible) return id;
                }
                return -1;
            });
            require(edited != -1, "UNTESTABLE: no empty non-password native editable fixture");
            original = "";
            setText("talkback-native-action-probe");
            result.putString("native_action", "SET_TEXT observed in shared GPUI semantics, not TalkBack activation");
            result.putString("password", "NOT_TESTED: no real password fixture injected or production state altered");
            result.putString("spoken_audio", "NOT_OBSERVED");
        } catch (Throwable error) { failure = error; }
        finally {
            try { if (edited != -1 && original != null) setText(original); }
            catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
            try { if (activity != null) ui(() -> { activity.finish(); return null; }); }
            catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
        }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", failure.getClass().getSimpleName() + ": " + failure.getMessage());
        finish(failure == null ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
}
