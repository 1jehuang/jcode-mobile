package org.jcode.mobile.platformworkflowtests;

import android.Manifest;
import android.app.Activity;
import android.app.Instrumentation;
import android.app.Notification;
import android.app.NotificationManager;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.pm.ShortcutInfo;
import android.content.pm.ShortcutManager;
import android.os.Bundle;
import android.os.Process;
import android.os.SystemClock;
import android.service.notification.StatusBarNotification;
import dalvik.system.DexClassLoader;
import java.lang.reflect.Field;
import java.util.Arrays;
import java.util.UUID;
import java.util.concurrent.Callable;
import java.util.concurrent.FutureTask;
import org.json.JSONArray;

/** Real OS workflows using unmodified installed production methods under a separate test UID. */
public final class PlatformWorkflowInstrumentation extends Instrumentation {
    private static final String TEST = "org.jcode.mobile.platformworkflowtests";
    private static final String TARGET = "org.jcode.mobile";
    private Activity activity;
    private Object services;
    private Class<?> type;
    private ShortcutManager shortcuts;
    private NotificationManager notifications;
    private String phase;
    private String headline;
    private boolean ownsResources;
    private String session;
    private String second;
    private static void check(boolean value, String message) {
        if (!value) throw new AssertionError(message);
    }
    private <T> T ui(Callable<T> action) throws Exception {
        FutureTask<T> task = new FutureTask<>(action);
        runOnMainSync(task);
        return task.get();
    }
    private boolean call(String method, String value) throws Exception {
        return ui(() -> (Boolean)type.getMethod(method, String.class).invoke(services, value));
    }
    private static String rows(String... values) throws Exception {
        JSONArray rows = new JSONArray();
        for (int i = 0; i < values.length; i += 2) rows.put(new JSONArray().put(values[i]).put(values[i + 1]));
        return rows.toString();
    }
    private ShortcutInfo shortcut(String id) {
        for (ShortcutInfo item : shortcuts.getDynamicShortcuts()) if (id.equals(item.getId())) return item;
        throw new AssertionError("Real ShortcutManager missing " + id);
    }
    private StatusBarNotification live() {
        for (StatusBarNotification item : notifications.getActiveNotifications()) {
            if ("agents".equals(item.getTag()) && item.getId() == 1) return item;
        }
        return null;
    }
    private void await(Callable<Boolean> condition, String message) throws Exception {
        long deadline = SystemClock.elapsedRealtime() + 5000;
        do {
            if (condition.call()) return;
            SystemClock.sleep(40);
        } while (SystemClock.elapsedRealtime() < deadline);
        throw new AssertionError(message);
    }
    @Override public void onCreate(Bundle args) {
        super.onCreate(args);
        phase = args == null ? null : args.getString("phase");
        headline = args == null ? null : args.getString("headline");
        start();
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Throwable failure = null;
        try {
            Context context = getTargetContext();
            check(TEST.equals(context.getPackageName()), "Refusing non-isolated target context");
            check(Process.myUid() == context.getApplicationInfo().uid
                    && Process.myUid() != context.getPackageManager().getApplicationInfo(TARGET, 0).uid,
                    "Refusing production UID");
            check("probe".equals(phase) || "denied".equals(phase) || "granted".equals(phase), "Explicit phase required");
            result.putString("notification_permission", context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                    == PackageManager.PERMISSION_GRANTED ? "granted" : "denied");
            if ("probe".equals(phase)) {
                result.putString("result", "PASS");
                finish(Activity.RESULT_OK, result);
                return;
            }
            check(headline != null && headline.matches("workflow-owned-[a-zA-Z0-9-]{1,80}"), "Bounded owned headline required");
            activity = startActivitySync(new Intent(context, WorkflowActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
            shortcuts = activity.getSystemService(ShortcutManager.class);
            notifications = activity.getSystemService(NotificationManager.class);
            check(shortcuts != null && notifications != null, "Missing real platform services");
            check(shortcuts.getDynamicShortcuts().isEmpty() && shortcuts.getPinnedShortcuts().isEmpty(),
                    "Preexisting test shortcuts: refusing replacement. Uninstall only test package before retry");
            check(notifications.getActiveNotifications().length == 0
                    && notifications.getNotificationChannel("jcode-agents") == null,
                    "Preexisting test notifications/channel: refusing overwrite");
            ownsResources = true;
            String apk = context.getPackageManager().getApplicationInfo(TARGET, 0).sourceDir;
            result.putString("production_apk", apk);
            DexClassLoader loader = new DexClassLoader(apk, context.getCodeCacheDir().getPath(), null, getClass().getClassLoader());
            type = loader.loadClass("org.jcode.mobile.AndroidServices");
            // Only nativeInit is bypassed. No Java adapter method is replaced or copied.
            Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
            Field singleton = unsafeClass.getDeclaredField("theUnsafe"); singleton.setAccessible(true);
            services = unsafeClass.getMethod("allocateInstance", Class.class).invoke(singleton.get(null), type);
            Field owner = type.getDeclaredField("activity"); owner.setAccessible(true); owner.set(services, activity);
            if ("denied".equals(phase)) {
            session = "workflow-" + UUID.randomUUID() + " /?雪";
            second = "workflow-" + UUID.randomUUID();
            check(!shortcuts.isRateLimitingActive(), "Test namespace already rate limited, cannot claim publication");
            check(shortcuts.getMaxShortcutCountPerActivity() >= 2, "Device shortcut capacity below bounded test requirement");
            check(call("indexSessions", rows(session, "Workflow initial", second, "Workflow delete")), "Production shortcut install failed");
            check(shortcuts.getDynamicShortcuts().size() == 2, "Real shortcut install count mismatch");
            ShortcutInfo initial = shortcut("session-" + session);
            check("Workflow initial".contentEquals(initial.getShortLabel()), "Initial label mismatch");
            Intent dispatch = initial.getIntent();
            check(dispatch != null && Intent.ACTION_VIEW.equals(dispatch.getAction()) && dispatch.getData() != null
                    && "jcode".equals(dispatch.getData().getScheme()) && "session".equals(dispatch.getData().getHost())
                    && session.equals(dispatch.getData().getLastPathSegment()), "Shortcut URI lost escaped session ownership");
            check(dispatch.getComponent() != null && TEST.equals(dispatch.getComponent().getPackageName()), "Shortcut escaped isolated package");
            int before = WorkflowActivity.deliveries;
            ui(() -> { activity.startActivity(dispatch); return null; });
            final int shortcutDeliveries = before + 1;
            await(() -> WorkflowActivity.deliveries == shortcutDeliveries, "Stored shortcut Intent did not reach actual Activity once");
            check(dispatch.getData().equals(WorkflowActivity.received.getData()), "Shortcut dispatch changed session URI");
            check(call("indexSessions", rows(session, "Workflow renamed")), "Production shortcut update failed");
            check(shortcuts.getDynamicShortcuts().size() == 1
                    && "Workflow renamed".contentEquals(shortcut("session-" + session).getShortLabel()), "Rename/deletion not reflected in OS");
            check(call("indexSessions", "[]") && shortcuts.getDynamicShortcuts().isEmpty(), "Production shortcut clear failed");
            result.putString("shortcuts", "PASS: real OS publication, encoded stored Intent dispatch, rename, replacement deletion and empty clear in test UID");

            check(!notifications.areNotificationsEnabled(), "Denied phase requires externally revoked TEST permission");
            check(!call("syncLive", headline), "Denied production notification unexpectedly acknowledged");
            check(live() == null, "Denied notification appeared");
            check(call("syncLive", ""), "Empty cancellation failed while denied");
            result.putString("notifications", "PASS: denied production publication rejected; empty cancellation acknowledged");
            } else {
            check(notifications.areNotificationsEnabled(), "Granted phase requires externally granted TEST permission");
            check(call("syncLive", headline), "Unchanged headline rejected after grant");
            await(() -> live() != null, "Notification absent after acknowledged unchanged retry");
            Notification notification = live().getNotification();
            check(headline.contentEquals(notification.extras.getCharSequence(Notification.EXTRA_TEXT))
                    && (notification.flags & Notification.FLAG_ONGOING_EVENT) != 0
                    && "jcode-agents".equals(notification.getChannelId()), "Actual notification content/ongoing/channel mismatch");
            check(notification.contentIntent != null && TEST.equals(notification.contentIntent.getCreatorPackage()), "Notification open escaped test UID");
            int before = WorkflowActivity.deliveries;
            notification.contentIntent.send();
            final int expectedDeliveries = before + 1;
            await(() -> WorkflowActivity.deliveries == expectedDeliveries, "Actual notification PendingIntent did not dispatch once");
            check(Intent.ACTION_MAIN.equals(WorkflowActivity.received.getAction()), "Notification did not dispatch MAIN");
            check(call("syncLive", ""), "Notification cancellation rejected");
            await(() -> live() == null, "Notification cancellation left actual active notification");
            result.putString("notifications", "PASS: granted unchanged headline, actual active ongoing content, real PendingIntent open and empty cancellation");
            }
            result.putString("limits", "Installed production Java adapters under test UID, not Rust cache retry, launcher UI tap/pinning, shared deep-link consumer, GPUI foreground settings invalidation, user notifications or physical hardware. No nativeInit, auth, model or network call.");
        } catch (Throwable error) { failure = error; }
        finally {
            try {
                if (ownsResources) {
                    // Only test-owned resources, never production package state or blanket cancelAll.
                    if (session != null && second != null) shortcuts.removeDynamicShortcuts(Arrays.asList("session-" + session, "session-" + second));
                    notifications.cancel("agents", 1);
                    notifications.deleteNotificationChannel("jcode-agents");
                    check(shortcuts.getDynamicShortcuts().isEmpty(), "Shortcut cleanup failed");
                    await(() -> live() == null, "Notification cleanup failed");
                }
            } catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
            try {
                if (activity != null) ui(() -> { activity.finish(); return null; });
            } catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
        }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", android.util.Log.getStackTraceString(failure));
        finish(failure == null ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
}
