package org.jcode.mobile.nativeservicestests;

import android.accessibilityservice.AccessibilityServiceInfo;
import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.Intent;
import android.graphics.Point;
import android.graphics.Rect;
import android.os.Bundle;
import android.os.SystemClock;
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
    private String nonce;
    private String navigation;
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
    private String counterSummary() throws Exception {
        return ui(() -> {
            StringBuilder summary = new StringBuilder();
            for (String name : new String[] {"hostQueries", "virtualQueries", "accessibilityFocusRequests", "hoverEvents", "accessibilityFocusClears", "accessibilityFocusRemovals", "lastAccessibilityFocusRequest", "accessibilityFocus"}) {
                if (summary.length() != 0) summary.append(',');
                summary.append(name).append('=');
                try { Field field = provider.getClass().getDeclaredField(name); field.setAccessible(true); summary.append(field.getLong(provider)); }
                catch (NoSuchFieldException unavailable) { summary.append("unavailable-old-target"); }
            }
            return summary.toString();
        });
    }
    private void reportMetadata() throws Exception {
        Bundle diagnostics = new Bundle();
        diagnostics.putString("talkback_provider_counters_before", counterSummary());
        diagnostics.putString("talkback_host_metadata", ui(() -> {
            View view = host(activity.findViewById(android.R.id.content));
            StringBuilder chain = new StringBuilder();
            for (View next = view; next != null; next = next.getParent() instanceof View ? (View)next.getParent() : null) {
                if (chain.length() != 0) chain.append(" <- ");
                chain.append(next.getClass().getName()).append("[important=").append(next.getImportantForAccessibility())
                        .append(",shown=").append(next.isShown()).append(",size=").append(next.getWidth()).append('x').append(next.getHeight()).append(']');
            }
            AccessibilityNodeInfo root = provider.createAccessibilityNodeInfo(AccessibilityNodeProvider.HOST_VIEW_ID);
            if (root != null) {
                Rect bounds = new Rect(); root.getBoundsInScreen(bounds);
                chain.append(" providerHost[children=").append(root.getChildCount()).append(",visible=").append(root.isVisibleToUser())
                        .append(",focusable=").append(root.isFocusable()).append(",important=").append(root.isImportantForAccessibility())
                        .append(",window=").append(root.getWindowId()).append(",bounds=").append(bounds.toShortString()).append(']');
                root.recycle();
            }
            return chain.toString();
        }));
        diagnostics.putString("talkback_nodes_metadata", ui(() -> {
            StringBuilder summary = new StringBuilder();
            int total = 0;
            for (int id : ids()) {
                if (total++ >= 24) { summary.append(" [remaining omitted]"); break; }
                AccessibilityNodeInfo node = provider.createAccessibilityNodeInfo(id);
                if (node == null) continue;
                Rect bounds = new Rect(); node.getBoundsInScreen(bounds);
                summary.append("id=").append(id).append("[class=").append(node.getClassName())
                        .append(",visible=").append(node.isVisibleToUser()).append(",focusable=").append(node.isFocusable())
                        .append(",important=").append(node.isImportantForAccessibility()).append(",enabled=").append(node.isEnabled())
                        .append(",password=").append(node.isPassword()).append(",hasText=").append(node.getText() != null && node.getText().length() > 0)
                        .append(",hasLabel=").append(node.getContentDescription() != null && node.getContentDescription().length() > 0)
                        .append(",actions=").append(node.getActions()).append(",window=").append(node.getWindowId())
                        .append(",bounds=").append(bounds.toShortString()).append("] ");
                node.recycle();
            }
            return summary.toString();
        }));
        sendStatus(2, diagnostics);
    }
    private void publishGestureReady() throws Exception {
        requireTargetWindow();
        String coordinates = ui(() -> {
            View decor = activity.getWindow().getDecorView();
            int[] origin = new int[2];
            decor.getLocationOnScreen(origin);
            int width = decor.getWidth(), height = decor.getHeight();
            require(width > 100 && height > 100, "UNTESTABLE: invalid real target window bounds");
            Point display = new Point();
            activity.getWindowManager().getDefaultDisplay().getRealSize(display);
            require(activity.getWindowManager().getDefaultDisplay().getRotation() == 0,
                    "UNTESTABLE_SETUP_OBSTRUCTION: raw hardware calibration currently requires unrotated emulator display");
            return (origin[0] + width / 4) + "," + (origin[1] + height / 2) + ","
                    + (origin[0] + width * 3 / 4) + "," + (origin[1] + height / 2) + "," + display.x + "," + display.y;
        });
        Bundle ready = new Bundle();
        ready.putString("talkback_ready", nonce + "," + coordinates);
        sendStatus(1, ready);
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
    @Override public void onCreate(Bundle args) {
        super.onCreate(args);
        nonce = args == null ? null : args.getString("talkback_nonce");
        navigation = args == null ? null : args.getString("talkback_navigation", "touch");
        start();
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Throwable failure = null;
        try {
            require(nonce != null && nonce.matches("[0-9a-f]{32}"), "UNTESTABLE: use separate TalkBack shell runner with fresh readiness nonce");
            require("touch".equals(navigation) || "keyboard".equals(navigation), "UNTESTABLE: invalid hardware navigation mode");
            result.putString("talkback_navigation_mode", navigation);
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
            reportMetadata();
            // Never call ACTION_ACCESSIBILITY_FOCUS ourselves. An enabled accessibility service must move it.
            int first = focus();
            StringBuilder observed = new StringBuilder().append(first);
            int previous = first;
            boolean traversed = false;
            publishGestureReady();
            long deadline = SystemClock.uptimeMillis() + 40000;
            while (SystemClock.uptimeMillis() < deadline && !traversed) {
                require(ui(() -> activity.hasWindowFocus() && activity.getWindow().getDecorView().hasWindowFocus()),
                        "UNTESTABLE_SETUP_OBSTRUCTION: target window lost focus while awaiting global emulator hardware gestures; no traversal established");
                int next = focus();
                if (next != previous) { observed.append(',').append(next); previous = next; }
                if (next != -1) { if (first == -1) first = next; else if (next != first) traversed = true; }
                SystemClock.sleep(50);
            }
            result.putString("talkback_observed_focus_ids", observed.toString());
            require(traversed, "UNTESTABLE: installed TalkBack hardware " + navigation + " navigation did not traverse two native semantic nodes");
            result.putString("talkback_traversal", "two native virtual focus targets observed around nonce-coordinated emulator hardware " + navigation + " navigation");
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
            try { if (provider != null) result.putString("talkback_provider_counters_after", counterSummary()); }
            catch (Throwable diagnostics) { if (failure == null) failure = diagnostics; else failure.addSuppressed(diagnostics); }
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
