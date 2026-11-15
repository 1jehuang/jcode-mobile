package org.jcode.mobile.nativeservicestests;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.os.Build;
import android.os.Bundle;
import android.os.SystemClock;
import android.text.Selection;
import android.text.Spannable;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.inspector.WindowInspector;
import android.widget.EditText;
import android.widget.ScrollView;
import android.widget.TextView;
import java.lang.reflect.Field;
import java.util.concurrent.Callable;
import java.util.concurrent.FutureTask;

/** Installed production dialog, actual framework selection/copy. No replacement view or UiAutomation. */
public final class SelectionInstrumentation extends Instrumentation {
    private Activity activity;
    private View dialogRoot;
    private TextView textView;
    private ClipboardManager clipboard;
    private ClipData previousClip;
    private boolean clipboardSaved;
    private static void check(boolean value, String message) {
        if (!value) throw new AssertionError(message);
    }
    private <T> T ui(Callable<T> action) throws Exception {
        FutureTask<T> task = new FutureTask<>(action);
        runOnMainSync(task); return task.get();
    }
    private static Object field(Object object, String name) throws Exception {
        Field f = object.getClass().getDeclaredField(name); f.setAccessible(true); return f.get(object);
    }
    @Override public void onCreate(Bundle args) { super.onCreate(args); start(); }
    private static TextView findText(View view, String expected) {
        if (view instanceof TextView && expected.contentEquals(((TextView)view).getText())) return (TextView)view;
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup)view;
            for (int i = 0; i < group.getChildCount(); i++) {
                TextView result = findText(group.getChildAt(i), expected);
                if (result != null) return result;
            }
        }
        return null;
    }
    private void checkNoEditor(View view) {
        check(!(view instanceof EditText), "Selection dialog contains editable input");
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup)view;
            for (int i = 0; i < group.getChildCount(); i++) checkNoEditor(group.getChildAt(i));
        }
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Throwable failure = null;
        try {
            check(Build.VERSION.SDK_INT >= 29, "WindowInspector acceptance requires API29+, not silently skipped");
            check("org.jcode.mobile".equals(getTargetContext().getPackageName()), "Wrong target package");
            Intent launch = getTargetContext().getPackageManager().getLaunchIntentForPackage("org.jcode.mobile");
            check(launch != null, "Production launch Activity missing");
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
            activity = startActivitySync(launch);
            waitForIdleSync();
            check(activity instanceof android.app.NativeActivity, "Not real production NativeActivity");
            clipboard = (ClipboardManager)activity.getSystemService(Context.CLIPBOARD_SERVICE);
            String marker = "selection-acceptance-" + java.util.UUID.randomUUID();
            StringBuilder longText = new StringBuilder(marker).append('\n');
            for (int i = 0; i < 180; i++) longText.append("Line ").append(i).append(": bounded production selectable text, not an editor.\n");
            final String content = longText.toString();
            check(content.length() < 20000, "Test payload exceeds bound");
            ui(() -> {
                previousClip = clipboard.getPrimaryClip(); clipboardSaved = true;
                Object services = field(activity, "services");
                check(Boolean.TRUE.equals(services.getClass().getMethod("selectText", String.class).invoke(services, content)),
                        "Production selectText rejected");
                return null;
            });
            long deadline = SystemClock.elapsedRealtime() + 5000;
            while (SystemClock.elapsedRealtime() < deadline) {
                if (ui(() -> {
                    for (View root : WindowInspector.getGlobalWindowViews()) {
                        TextView found = findText(root, content);
                        if (found != null && root.isShown() && root.hasWindowFocus()) {
                            dialogRoot = root; textView = found; return true;
                        }
                    }
                    return false;
                })) break;
                SystemClock.sleep(30);
            }
            ui(() -> {
                check(dialogRoot != null && textView != null, "Actual production dialog did not appear/focus");
                check(findText(dialogRoot, "Select text") != null && findText(dialogRoot, "Done") != null,
                        "Production dialog title/action missing");
                checkNoEditor(dialogRoot);
                check(textView.isTextSelectable() && !textView.onCheckIsTextEditor(), "Text is not nonediting selectable text");
                check(textView.getText() instanceof Spannable, "Selectable text is not framework Spannable");
                if (Build.VERSION.SDK_INT >= 30) {
                    WindowInsets insets = dialogRoot.getRootWindowInsets();
                    check(insets != null && !insets.isVisible(WindowInsets.Type.ime()), "Selection dialog displayed editing keyboard");
                }
                check(textView.getParent() instanceof ScrollView, "Production text lacks ScrollView");
                ScrollView scroll = (ScrollView)textView.getParent();
                check(scroll.getHeight() > 0 && textView.getHeight() > scroll.getHeight(), "Long text did not exceed bounded viewport");
                scroll.fullScroll(View.FOCUS_DOWN);
                return null;
            });
            waitForIdleSync();
            ui(() -> {
                ScrollView scroll = (ScrollView)textView.getParent();
                check(scroll.getScrollY() > 0, "Long text failed to scroll");
                scroll.scrollTo(0, 0);
                textView.requestFocus();
                Selection.setSelection((Spannable)textView.getText(), 0, marker.length());
                check(textView.getSelectionStart() == 0 && textView.getSelectionEnd() == marker.length(), "Framework selection failed");
                check(textView.onTextContextMenuItem(android.R.id.copy), "Framework Copy command rejected");
                ClipData actual = clipboard.getPrimaryClip();
                check(actual != null && actual.getItemCount() == 1
                        && marker.contentEquals(actual.getItemAt(0).coerceToText(activity)), "Copy did not copy exact production-view selection");
                check(content.contentEquals(textView.getText()), "Selection/Copy edited production dialog text");
                if (Build.VERSION.SDK_INT >= 30) {
                    WindowInsets insets = dialogRoot.getRootWindowInsets();
                    check(insets != null && !insets.isVisible(WindowInsets.Type.ime()), "Selection/Copy opened editing keyboard");
                }
                return null;
            });
            sendKeyDownUpSync(KeyEvent.KEYCODE_BACK);
            waitForIdleSync();
            ui(() -> {
                check(!dialogRoot.isAttachedToWindow(), "Back did not dismiss selectable dialog");
                check(!activity.isFinishing() && !activity.isDestroyed()
                        && activity.getWindow().getDecorView().isAttachedToWindow(), "Back closed production Activity");
                Object services = field(activity, "services");
                check(Boolean.TRUE.equals(field(services, "foreground")), "Production Activity not resumed after dialog dismissal");
                return null;
            });
            result.putString("selection", "PASS: installed production selectText dialog, noneditable selectable long text, bounded viewport/scroll, exact framework selection Copy, Back dismissal and live NativeActivity");
            result.putString("limits", "Framework selection and Copy command, not long-press/drag handles or Copy toolbar gesture. API29+ window inspection; IME visibility checked API30+. No voice/network service invocation by test.");
        } catch (Throwable error) { failure = error; }
        finally {
            try {
                if (activity != null && dialogRoot != null && ui(() -> dialogRoot.isAttachedToWindow())) {
                    sendKeyDownUpSync(KeyEvent.KEYCODE_BACK); waitForIdleSync();
                }
            } catch (Throwable cleanup) {
                if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup);
            }
            // Clipboard restoration must still run if dismissal itself failed.
            try {
                if (clipboardSaved) ui(() -> {
                    if (previousClip != null) clipboard.setPrimaryClip(previousClip);
                    else clipboard.clearPrimaryClip();
                    check(previousClip != null || !clipboard.hasPrimaryClip(), "Failed to restore empty clipboard");
                    return null;
                });
            } catch (Throwable cleanup) {
                if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup);
            }
        }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", android.util.Log.getStackTraceString(failure));
        finish(failure == null ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
}
