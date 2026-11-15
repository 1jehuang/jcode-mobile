package org.jcode.mobile;

import android.Manifest;
import android.app.*;
import android.content.*;
import android.content.pm.PackageManager;
import android.os.*;
import android.service.notification.StatusBarNotification;

/** Actual OS service/notification assertions under a separate UID, not end-to-end GPUI evidence. */
public final class TurnInstrumentation extends Instrumentation {
    private static final String TEST = "org.jcode.mobile.turnkeepalivetests";
    private static final String SENTINEL_CHANNEL = "turn-test-sentinel";
    private Context context;
    private NotificationManager manager;
    private TurnTestActivity activity;
    private Bundle args;
    private boolean owns;
    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); args = arguments; start(); }
    private static void check(boolean condition, String message) { if (!condition) throw new AssertionError(message); }
    private boolean granted() { return context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED; }
    private boolean clean() { return manager.getActiveNotifications().length == 0 && manager.getNotificationChannels().isEmpty(); }
    private void ui(Runnable run) { runOnMainSync(run); }
    private boolean update(boolean busy, boolean foreground) {
        boolean[] result = {false};
        ui(() -> result[0] = TurnKeepAliveService.update(context, busy, foreground));
        return result[0];
    }
    private StatusBarNotification record() {
        for (StatusBarNotification item : manager.getActiveNotifications()) {
            if (item.getId() == TurnKeepAliveService.NOTIFICATION && item.getTag() == null) return item;
        }
        return null;
    }
    private void awaitRecord(boolean present, long waitMillis) {
        long deadline = SystemClock.elapsedRealtime() + waitMillis;
        while ((record() != null) != present && SystemClock.elapsedRealtime() < deadline) SystemClock.sleep(50);
        check((record() != null) == present, present ? "Missing actual foreground notification" : "Foreground notification not removed");
    }
    private void launch() {
        activity = (TurnTestActivity) startActivitySync(new Intent(context, TurnTestActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        waitForIdleSync();
        check(activity.resumed, "Visible resumed test Activity required");
    }
    private void sentinel() {
        manager.createNotificationChannel(new NotificationChannel(SENTINEL_CHANNEL, "Test sentinel", NotificationManager.IMPORTANCE_LOW));
        manager.notify("sentinel", 99, new Notification.Builder(context, SENTINEL_CHANNEL)
                .setSmallIcon(android.R.drawable.stat_notify_sync).setContentTitle("Isolated sentinel").build());
    }
    private void sentinelIntact() {
        for (StatusBarNotification item : manager.getActiveNotifications()) {
            if ("sentinel".equals(item.getTag()) && item.getId() == 99) return;
        }
        throw new AssertionError("Unrelated notification was removed");
    }
    private void grantedPhase() {
        check(granted() && manager.areNotificationsEnabled(), "Actual test notification grant required");
        launch();
        sentinel();
        update(false, true);
        check(!update(true, false), "Background admission was not denied");
        check(!update(true, true), "Consumed background edge retried on resume");
        awaitRecord(false, 1000);
        update(false, true);
        long started = SystemClock.elapsedRealtime();
        check(update(true, activity.resumed), "Visible foreground start rejected");
        awaitRecord(true, 5000);
        Notification n = record().getNotification();
        check((n.flags & Notification.FLAG_FOREGROUND_SERVICE) != 0, "Not an actual foreground service record");
        check((n.flags & Notification.FLAG_ONGOING_EVENT) != 0, "Not ongoing");
        check((n.flags & Notification.FLAG_LOCAL_ONLY) != 0, "Not local-only");
        check(n.visibility == Notification.VISIBILITY_PRIVATE && n.publicVersion != null, "Privacy contract missing");
        check(n.contentIntent == null && n.actions == null && n.publicVersion.contentIntent == null, "Unexpected account/session intent");
        check("Jcode".contentEquals(n.publicVersion.extras.getCharSequence(Notification.EXTRA_TITLE)), "Public version not generic");
        check(!update(true, true), "Repeated busy refreshed lease");
        // Leave the Activity while retaining the already-legally-started lease.
        ui(() -> activity.moveTaskToBack(true));
        SystemClock.sleep(1000);
        check(!activity.resumed, "Activity did not background");
        check(record() != null, "Lease disappeared immediately on pause");
        // Wait for the actual unshortened production 150-second timer. Never mutate clock/timeout.
        awaitRecord(false, 155000);
        long elapsed = SystemClock.elapsedRealtime() - started;
        check(elapsed >= TurnLease.MAX_MILLIS - 1000 && elapsed <= TurnLease.MAX_MILLIS + 7000,
                "Actual bounded timer outside acceptance window: " + elapsed);
        check(!update(true, false), "Expired busy restarted in background");
        awaitRecord(false, 1000);
        sentinelIntact();
        ui(() -> activity.finish());
        waitForIdleSync();
        launch();
        update(false, true);
        check(update(true, true), "New turn not admitted");
        awaitRecord(true, 5000);
        update(false, false);
        awaitRecord(false, 5000);
        sentinelIntact();
        check(update(true, true), "New turn after idle not admitted");
        awaitRecord(true, 5000);
        ui(() -> TurnKeepAliveService.cancel(context)); // Same production signout/destroy primitive.
        awaitRecord(false, 5000);
        check(!update(true, true), "Cancel rearmed current busy turn");
        sentinelIntact();
        update(false, true);
        check(update(true, true), "Destroy fixture start failed");
        awaitRecord(true, 5000);
        ui(() -> activity.finish());
        waitForIdleSync();
        awaitRecord(false, 5000);
        sentinelIntact();
    }
    private void cleanup() {
        ui(() -> {
            TurnKeepAliveService.cancel(context);
            if (activity != null && !activity.isFinishing()) activity.finish();
        });
        awaitRecord(false, 5000);
        manager.cancel("sentinel", 99);
        manager.deleteNotificationChannel(TurnKeepAliveService.CHANNEL);
        manager.deleteNotificationChannel(SENTINEL_CHANNEL);
        check(clean(), "Owned cleanup left resources");
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Throwable failure = null;
        String phase = args == null ? "" : args.getString("phase", "");
        try {
            context = getTargetContext();
            check(TEST.equals(context.getPackageName()), "Refusing non-test package");
            check(context.getApplicationInfo().uid == android.os.Process.myUid(), "Wrong execution UID");
            check(context.getPackageManager().getApplicationInfo("org.jcode.mobile", 0).uid != android.os.Process.myUid(), "Production UID forbidden");
            manager = context.getSystemService(NotificationManager.class);
            check(manager != null, "NotificationManager missing");
            if ("probe".equals(phase)) {
                result.putString("notification_permission", granted() ? "granted" : "denied");
                result.putString("notifications_enabled", Boolean.toString(manager.areNotificationsEnabled()));
                result.putString("namespace_clean", Boolean.toString(clean()));
            } else {
                check("true".equals(args.getString("owns_test_namespace")), "Explicit namespace ownership required");
                if ("cleanup".equals(phase)) { owns = true; cleanup(); }
                else {
                    check(clean(), "Refusing pre-existing test namespace");
                    check("denied".equals(phase) || "granted".equals(phase), "Unknown phase");
                    owns = true;
                    if ("denied".equals(phase)) {
                        check(!granted(), "Actual denied test permission required");
                        launch();
                        update(false, true);
                        check(!update(true, true), "Denied permission admitted service");
                        awaitRecord(false, 1000);
                    } else if ("granted".equals(phase)) grantedPhase();
                    else throw new AssertionError("Unknown phase");
                }
            }
        } catch (Throwable error) { failure = error; }
        finally {
            if (owns && context != null && manager != null) {
                try { cleanup(); } catch (Throwable cleanupError) {
                    if (failure == null) failure = cleanupError; else failure.addSuppressed(cleanupError);
                }
            }
        }
        if (failure == null) {
            result.putString("result", "PASS");
            finish(Activity.RESULT_OK, result);
        } else {
            result.putString("result", "FAIL");
            result.putString("error", android.util.Log.getStackTraceString(failure));
            finish(Activity.RESULT_CANCELED, result);
        }
    }
}
