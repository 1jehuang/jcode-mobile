package org.jcode.mobile.agentnotificationtests;

import android.Manifest;
import android.app.Activity;
import android.app.Instrumentation;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Process;
import android.os.Bundle;
import android.os.SystemClock;
import android.service.notification.StatusBarNotification;
import dalvik.system.DexClassLoader;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.Callable;
import java.util.concurrent.FutureTask;
import org.json.JSONArray;
import org.json.JSONObject;

/** Real OS records from installed production methods under a separate, no-network test UID. */
public final class AgentNotificationsInstrumentation extends Instrumentation {
    private static final String TEST = "org.jcode.mobile.agentnotificationtests", TARGET = "org.jcode.mobile";
    private static final String PREFIX = "agents.session.", SENTINEL = "agent-acceptance.sentinel";
    private static final int SENTINEL_ID = 4304;
    private Activity activity;
    private Object services;
    private Method sync;
    private NotificationManager manager;
    private Bundle arguments;
    private boolean ownsResources, sentinelPosted;
    private Field owner;
    private final java.util.Set<PendingIntent> pendingIntents = new java.util.HashSet<>();
    private static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    @Override public void onCreate(Bundle args) { super.onCreate(args); arguments = args == null ? new Bundle() : new Bundle(args); start(); }
    private <T> T ui(Callable<T> call) throws Exception {
        FutureTask<T> task = new FutureTask<>(call); runOnMainSync(task); return task.get();
    }
    private boolean apply(String json) throws Exception { return ui(() -> (Boolean) sync.invoke(services, json)); }
    private static String tag(String id) { return PREFIX + Uri.encode(id); }
    private static boolean owned(StatusBarNotification n) {
        return n.getId() == 1 && ("agents".equals(n.getTag()) || (n.getTag() != null && n.getTag().startsWith(PREFIX)));
    }
    private Map<String, Notification> ownedRecords() {
        Map<String, Notification> result = new HashMap<>();
        for (StatusBarNotification n : manager.getActiveNotifications()) if (owned(n)) result.put(n.getTag(), n.getNotification());
        return result;
    }
    private void await(Callable<Boolean> predicate, String message) throws Exception {
        long deadline = SystemClock.uptimeMillis() + 6000;
        while (SystemClock.uptimeMillis() < deadline) { if (predicate.call()) return; SystemClock.sleep(100); }
        check(predicate.call(), message);
    }
    private static JSONObject item(String id, String state) throws Exception {
        return new JSONObject().put("id", id).put("title", "PRIVATE Fixture " + id).put("state", state)
                .put("detail", "PRIVATE Fixture detail").put("origin", "PRIVATE Fixture origin")
                .put("since", System.currentTimeMillis() / 1000 - 20).put("total", 5).put("done", 2).put("active", 1);
    }
    private static String state(int running, int waiting, JSONObject... rows) throws Exception {
        JSONArray items = new JSONArray(); for (JSONObject row : rows) items.put(row);
        return new JSONObject().put("running", running).put("waiting", waiting).put("errored", 0).put("items", items).put("updatedAt", 0).toString();
    }
    private boolean hasSentinel() {
        for (StatusBarNotification n : manager.getActiveNotifications()) if (SENTINEL.equals(n.getTag()) && n.getId() == SENTINEL_ID) return true;
        return false;
    }
    private void checkPrivate(Notification n, boolean summary) {
        check(n != null && n.visibility == Notification.VISIBILITY_PRIVATE, "Missing private record");
        check((n.flags & Notification.FLAG_ONGOING_EVENT) != 0 && (n.flags & Notification.FLAG_ONLY_ALERT_ONCE) != 0, "Ongoing/only-alert-once flags missing");
        check((n.flags & Notification.FLAG_LOCAL_ONLY) != 0, "Notification can bridge private content");
        check(((n.flags & Notification.FLAG_GROUP_SUMMARY) != 0) == summary && "jcode.agents".equals(n.getGroup()), "Group contract mismatch");
        check(n.getGroupAlertBehavior() == (summary ? Notification.GROUP_ALERT_CHILDREN : Notification.GROUP_ALERT_SUMMARY), "Initial/update group mute missing");
        check(n.sound == null && n.defaults == 0, "Notification requests sound/default alerts");
        Notification publicVersion = n.publicVersion;
        check(publicVersion != null && publicVersion.contentIntent == null && (publicVersion.actions == null || publicVersion.actions.length == 0), "Public fallback carries private interaction");
        check("jcode agents".contentEquals(publicVersion.extras.getCharSequence(Notification.EXTRA_TITLE)), "Public fallback title not generic");
        check("Open Jcode to view agent activity".contentEquals(publicVersion.extras.getCharSequence(Notification.EXTRA_TEXT)), "Public fallback text not generic");
        check(n.contentIntent != null && TEST.equals(n.contentIntent.getCreatorPackage()), "Content intent escaped isolated test UID");
        pendingIntents.add(n.contentIntent);
        if (Build.VERSION.SDK_INT >= 31) check(n.contentIntent.isImmutable(), "Mutable content PendingIntent");
    }
    private PendingIntent expectedSession(String id) {
        Intent intent = new Intent(activity, activity.getClass()).setAction(Intent.ACTION_VIEW)
                .setData(Uri.parse("jcode://session/" + Uri.encode(id)))
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        return PendingIntent.getActivity(activity, 4304, intent, PendingIntent.FLAG_NO_CREATE | PendingIntent.FLAG_IMMUTABLE);
    }
    private void dispatchSession(Notification notification, String id) throws Exception {
        int before = AgentTestActivity.deliveries;
        notification.contentIntent.send();
        final int expectedDeliveries = before + 1;
        await(() -> AgentTestActivity.deliveries == expectedDeliveries, "Child PendingIntent did not dispatch exactly once");
        check(Intent.ACTION_VIEW.equals(AgentTestActivity.received.getAction())
                && Uri.parse("jcode://session/" + Uri.encode(id)).equals(AgentTestActivity.received.getData()), "Dispatched session route changed");
        activity = AgentTestActivity.current; owner.set(services, activity);
        check(activity != null && !activity.isDestroyed(), "Isolated receiver Activity not alive");
    }
    private void full(Bundle result) throws Exception {
        check(manager.areNotificationsEnabled(), "Granted phase requires externally granted isolated TEST permission");
        NotificationChannel original = manager.getNotificationChannel("jcode-agents");
        check(original == null, "Refusing pre-existing test channel");
        JSONObject aa = item("Aa", "running").put("plan", "PRIVATE Fixture plan"), bb = item("BB", "waiting");
        check(apply(state(1, 1, aa, bb)), "Initial production call failed");
        await(() -> ownedRecords().size() == 3, "Initial summary/two children not delivered");
        Map<String, Notification> first = ownedRecords();
        Notification firstAa = first.get(tag("Aa")), firstBb = first.get(tag("BB"));
        checkPrivate(first.get("agents"), true); checkPrivate(firstAa, false); checkPrivate(firstBb, false);
        check("Aa".hashCode() == "BB".hashCode() && !firstAa.contentIntent.equals(firstBb.contentIntent), "Java hash-collision sessions share OS PendingIntent");
        check(firstAa.contentIntent.equals(expectedSession("Aa")) && firstBb.contentIntent.equals(expectedSession("BB")), "OS intent identity is not exact encoded session route");
        check("PRIVATE Fixture plan".contentEquals(firstAa.extras.getCharSequence(Notification.EXTRA_TITLE)), "Plan name not shown");
        check(firstAa.extras.getCharSequence(Notification.EXTRA_TEXT).toString().contains("2/5 todos (1 active)"), "Per-session todo state missing");
        check(firstAa.extras.getBoolean(Notification.EXTRA_SHOW_CHRONOMETER), "OS state timer missing");
        dispatchSession(firstAa, "Aa");
        dispatchSession(firstBb, "BB");
        manager.notify(SENTINEL, SENTINEL_ID, new Notification.Builder(activity, "jcode-agents")
                .setSmallIcon(android.R.drawable.stat_notify_sync).setContentTitle("Acceptance sentinel").setOnlyAlertOnce(true).build());
        await(this::hasSentinel, "Unrelated test sentinel not delivered");
        sentinelPosted = true;
        SystemClock.sleep(1500);
        aa.put("plan", "PRIVATE Fixture updated");
        check(apply(state(1, 1, bb, aa)), "Reorder/update failed");
        await(() -> { Notification n = ownedRecords().get(tag("Aa")); return n != null && "PRIVATE Fixture updated".contentEquals(n.extras.getCharSequence(Notification.EXTRA_TITLE)); }, "Updated content not delivered");
        Notification updated = ownedRecords().get(tag("Aa"));
        check(updated.contentIntent.equals(firstAa.contentIntent) && "01".equals(updated.getSortKey()), "Stable session identity/rank lost");
        check(!apply("not-json"), "Malformed payload falsely acknowledged");
        char[] huge = new char[128 * 1024 + 1]; java.util.Arrays.fill(huge, 'x');
        check(!apply(new String(huge)), "Unbounded payload falsely acknowledged");
        check(ownedRecords().size() == 3, "Malformed payload changed owned records");
        SystemClock.sleep(1500);
        JSONObject[] many = new JSONObject[8]; for (int i = 0; i < many.length; i++) many[i] = item("fixture-many-" + i, "running");
        check(apply(state(8, 0, many)), "Bounded many-session publication failed");
        await(() -> ownedRecords().size() == 7 && !ownedRecords().containsKey(tag("Aa")), "At-most-six/obsolete cleanup not delivered");
        check(!ownedRecords().containsKey(tag("fixture-many-6")) && hasSentinel(), "Child bound or unrelated sentinel violated");
        for (Map.Entry<String, Notification> entry : ownedRecords().entrySet()) checkPrivate(entry.getValue(), "agents".equals(entry.getKey()));
        SystemClock.sleep(1500);
        String special = "fixture/id +?%\u2713";
        check(apply(state(1, 0, item(special, "running"))), "Special-id publication failed");
        await(() -> ownedRecords().size() == 2 && ownedRecords().containsKey(tag(special)), "Special-id/obsolete cleanup not delivered");
        check(ownedRecords().get(tag(special)).contentIntent.equals(expectedSession(special)), "Special ID not safely URI-encoded");
        checkPrivate(ownedRecords().get(tag(special)), false);
        dispatchSession(ownedRecords().get(tag(special)), special);
        check(hasSentinel(), "Owned cleanup canceled unrelated notification");
        result.putString("records", "PASS: installed production methods under isolated UID, real OS private summary/children, generic public fallback, distinct/stable immutable intents and actual child dispatch, plans/todos/timers, bounds, malformed rejection, owned cleanup and sentinel preservation");
    }
    @Override public void onStart() {
        Bundle result = new Bundle(); Throwable failure = null;
        try {
            Context context = getTargetContext();
            check(TEST.equals(context.getPackageName()), "Refusing non-isolated context");
            check(Process.myUid() == context.getApplicationInfo().uid
                    && Process.myUid() != context.getPackageManager().getApplicationInfo(TARGET, 0).uid, "Refusing production UID");
            String phase = arguments.getString("phase");
            check("probe".equals(phase) || "denied".equals(phase) || "granted".equals(phase) || "cleanup".equals(phase), "Explicit phase required");
            manager = context.getSystemService(NotificationManager.class);
            check(manager != null, "NotificationManager unavailable");
            result.putString("notification_permission", context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                    == PackageManager.PERMISSION_GRANTED ? "granted" : "denied");
            result.putString("notifications_enabled", manager.areNotificationsEnabled() ? "true" : "false");
            result.putString("namespace_clean", manager.getActiveNotifications().length == 0 && manager.getNotificationChannel("jcode-agents") == null ? "true" : "false");
            if ("cleanup".equals(phase)) {
                check("true".equals(arguments.getString("owns_test_namespace")), "Explicit runner-owned clean-baseline namespace required");
                for (StatusBarNotification n : manager.getActiveNotifications()) {
                    if (owned(n)) { if (n.getNotification().contentIntent != null) n.getNotification().contentIntent.cancel(); manager.cancel(n.getTag(), n.getId()); }
                }
                manager.cancel(SENTINEL, SENTINEL_ID); manager.deleteNotificationChannel("jcode-agents");
                await(() -> manager.getActiveNotifications().length == 0 && manager.getNotificationChannel("jcode-agents") == null, "Runner fallback cleanup failed");
                result.putString("result", "PASS"); finish(Activity.RESULT_OK, result); return;
            }
            if ("probe".equals(phase)) {
                result.putString("result", "PASS"); finish(Activity.RESULT_OK, result); return;
            }
            check(manager.getActiveNotifications().length == 0 && manager.getNotificationChannel("jcode-agents") == null,
                    "Pre-existing isolated test records/channel, refusing overwrite");
            ownsResources = true;
            activity = startActivitySync(new Intent(context, AgentTestActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
            String apk = context.getPackageManager().getApplicationInfo(TARGET, 0).sourceDir;
            result.putString("production_apk", apk);
            DexClassLoader loader = new DexClassLoader(apk, context.getCodeCacheDir().getPath(), null, getClass().getClassLoader());
            Class<?> type = loader.loadClass("org.jcode.mobile.AndroidServices");
            // Bypass nativeInit only. Execute installed production methods unmodified.
            Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
            Field singleton = unsafeClass.getDeclaredField("theUnsafe"); singleton.setAccessible(true);
            services = unsafeClass.getMethod("allocateInstance", Class.class).invoke(singleton.get(null), type);
            owner = type.getDeclaredField("activity"); owner.setAccessible(true); owner.set(services, activity);
            // allocateInstance bypasses field initialization, including the bounded owned-tag set.
            Field tags = type.getDeclaredField("agentNotificationTags"); tags.setAccessible(true);
            tags.set(services, new java.util.HashSet<String>());
            sync = type.getMethod("syncAgents", String.class);
            if ("denied".equals(phase)) {
                check(!manager.areNotificationsEnabled(), "Denied phase requires externally revoked isolated TEST permission");
                check(!apply(state(1, 0, item("fixture-denied", "running"))), "Denied publication falsely acknowledged");
                check(apply(state(0, 0)), "Permission-denied empty teardown not acknowledged");
                await(() -> ownedRecords().isEmpty(), "Denied phase retained private records");
                result.putString("denied", "PASS: actual OS denied update false, empty teardown true, no owned records in isolated UID");
            } else full(result);
            result.putString("limits", "Installed production Java adapter under separate UID, no nativeInit, GPUI, auth, server, network or production permission changes. OS records and isolated PendingIntent dispatch, not shared resolver/lock-screen rendering/FCM acceptance.");
        } catch (Throwable error) { failure = error; }
        finally {
            if (ownsResources) {
                try {
                    if (services != null && sync != null) check(apply(state(0, 0)), "Final production cleanup failed");
                    await(() -> ownedRecords().isEmpty(), "Final production cleanup retained private records");
                    if (sentinelPosted) check(hasSentinel(), "Empty teardown canceled unrelated sentinel");
                } catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
                try {
                    // Dedicated test UID/channel only, even if production cleanup itself failed.
                    for (StatusBarNotification n : manager.getActiveNotifications()) {
                        if (owned(n)) { if (n.getNotification().contentIntent != null) pendingIntents.add(n.getNotification().contentIntent); manager.cancel(n.getTag(), n.getId()); }
                    }
                    manager.cancel(SENTINEL, SENTINEL_ID);
                    for (PendingIntent pending : pendingIntents) pending.cancel();
                    manager.deleteNotificationChannel("jcode-agents");
                    await(() -> manager.getActiveNotifications().length == 0 && manager.getNotificationChannel("jcode-agents") == null,
                            "Isolated fallback cleanup failed");
                } catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
            }
            try { if (activity != null) ui(() -> { activity.finish(); return null; }); }
            catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
        }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", android.util.Log.getStackTraceString(failure));
        finish(failure == null ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
}
