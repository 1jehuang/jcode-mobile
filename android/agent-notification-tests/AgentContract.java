// Pure host fixture. The placeholder below is replaced with actual AndroidServices source.
// These fake OS APIs exercise algorithm contracts, never claim real Android delivery.
import java.util.*;
import java.util.concurrent.Callable;
public class AgentContract {
    static class JSONException extends Exception { JSONException(String message) { super(message); } }
    static class JSONObject {
        static final Map<String, JSONObject> fixtures = new HashMap<>();
        final Map<String, Object> values;
        JSONObject() { values = new HashMap<>(); }
        JSONObject(String key) throws JSONException {
            if (!fixtures.containsKey(key)) throw new JSONException("Malformed fixture");
            values = fixtures.get(key).values;
        }
        JSONObject put(String key, Object value) { values.put(key, value); return this; }
        long getLong(String key) throws JSONException {
            Object value = values.get(key);
            if (!(value instanceof Number)) throw new JSONException("Missing number");
            return ((Number)value).longValue();
        }
        long optLong(String key, long fallback) { Object value = values.get(key); return value instanceof Number ? ((Number)value).longValue() : fallback; }
        String getString(String key) throws JSONException {
            Object value = values.get(key); if (!(value instanceof String)) throw new JSONException("Missing string"); return (String)value;
        }
        String optString(String key, String fallback) { Object value = values.get(key); return value == null ? fallback : value.toString(); }
        JSONArray getJSONArray(String key) throws JSONException {
            Object value = values.get(key); if (!(value instanceof JSONArray)) throw new JSONException("Missing items"); return (JSONArray)value;
        }
    }
    static class JSONArray {
        final List<JSONObject> rows = new ArrayList<>();
        int length() { return rows.size(); }
        JSONObject getJSONObject(int i) { return rows.get(i); }
    }
    static class Context { static final String NOTIFICATION_SERVICE = "notification"; }
    static class Activity {
        NotificationManager manager = new NotificationManager();
        Object getSystemService(String key) { return manager; }
    }
    static class android { static class R { static class drawable { static final int stat_notify_sync = 1; } } }
    static class Uri {
        final String value; Uri(String value) { this.value = value; }
        static Uri parse(String value) { return new Uri(value); }
        static String encode(String value) {
            try { return java.net.URLEncoder.encode(value, "UTF-8").replace("+", "%20"); }
            catch (Exception impossible) { throw new AssertionError(impossible); }
        }
    }
    static class Intent {
        static final String ACTION_VIEW = "view", ACTION_MAIN = "main";
        static final int FLAG_ACTIVITY_SINGLE_TOP = 1, FLAG_ACTIVITY_CLEAR_TOP = 2;
        String action; Uri data; int flags;
        Intent(Activity context, Class<?> target) {}
        Intent setAction(String value) { action = value; return this; }
        Intent setData(Uri value) { data = value; return this; }
        Intent addFlags(int value) { flags |= value; return this; }
    }
    static class PendingIntent {
        static final int FLAG_UPDATE_CURRENT = 1, FLAG_IMMUTABLE = 2;
        final String identity; final Intent intent; final int flags;
        PendingIntent(String identity, Intent intent, int flags) { this.identity = identity; this.intent = intent; this.flags = flags; }
        static PendingIntent getActivity(Activity context, int request, Intent intent, int flags) {
            return new PendingIntent(request + ":" + intent.action + ":" + (intent.data == null ? "" : intent.data.value), intent, flags);
        }
    }
    static class NotificationChannel {
        final String id; int importance; boolean badge;
        NotificationChannel(String id, String name, int importance) { this.id = id; this.importance = importance; }
        void setShowBadge(boolean value) { badge = value; }
        int getImportance() { return importance; }
    }
    static class Notification {
        static final int VISIBILITY_PRIVATE = 0, VISIBILITY_PUBLIC = 1, GROUP_ALERT_CHILDREN = 2, GROUP_ALERT_SUMMARY = 1;
        String title, text, sort, group; boolean ongoing, onlyOnce, local, summary, showWhen, timer;
        int visibility, alert, defaults; Object sound; long when;
        Notification publicVersion; PendingIntent pending;
        static class BigTextStyle { BigTextStyle bigText(String ignored) { return this; } }
        static class Builder {
            final Notification n = new Notification();
            Builder(Activity activity, String channel) {}
            Builder setSmallIcon(int value) { return this; }
            Builder setContentTitle(String value) { n.title = value; return this; }
            Builder setContentText(String value) { n.text = value; return this; }
            Builder setOngoing(boolean value) { n.ongoing = value; return this; }
            Builder setOnlyAlertOnce(boolean value) { n.onlyOnce = value; return this; }
            Builder setVisibility(int value) { n.visibility = value; return this; }
            Builder setPublicVersion(Notification value) { n.publicVersion = value; return this; }
            Builder setLocalOnly(boolean value) { n.local = value; return this; }
            Builder setShowWhen(boolean value) { n.showWhen = value; return this; }
            Builder setSound(Object value) { n.sound = value; return this; }
            Builder setDefaults(int value) { n.defaults = value; return this; }
            Builder setGroup(String value) { n.group = value; return this; }
            Builder setGroupSummary(boolean value) { n.summary = value; return this; }
            Builder setGroupAlertBehavior(int value) { n.alert = value; return this; }
            Builder setStyle(BigTextStyle value) { return this; }
            Builder setContentIntent(PendingIntent value) { n.pending = value; return this; }
            Builder setSortKey(String value) { n.sort = value; return this; }
            Builder setWhen(long value) { n.when = value; return this; }
            Builder setUsesChronometer(boolean value) { n.timer = value; return this; }
            Notification build() { return n; }
        }
    }
    static class StatusBarNotification {
        final String tag; final int id; final Notification n;
        StatusBarNotification(String tag, int id, Notification n) { this.tag = tag; this.id = id; this.n = n; }
        String getTag() { return tag; } int getId() { return id; }
    }
    static class NotificationManager {
        static final int IMPORTANCE_LOW = 2, IMPORTANCE_NONE = 0;
        final Map<String, StatusBarNotification> active = new LinkedHashMap<>();
        NotificationChannel channel; boolean enabled = true, hideActive; int failAfter = -1;
        String key(String tag, int id) { return tag + ":" + id; }
        void notify(String tag, int id, Notification n) {
            if (failAfter == 0) throw new IllegalStateException("Synthetic publish failure");
            if (failAfter > 0) failAfter--;
            active.put(key(tag, id), new StatusBarNotification(tag, id, n));
        }
        void cancel(String tag, int id) { active.remove(key(tag, id)); }
        boolean areNotificationsEnabled() { return enabled; }
        void createNotificationChannel(NotificationChannel value) { if (channel == null) channel = value; }
        NotificationChannel getNotificationChannel(String id) { return channel; }
        StatusBarNotification[] getActiveNotifications() { return hideActive ? new StatusBarNotification[0] : active.values().toArray(new StatusBarNotification[0]); }
        Notification get(String tag) { StatusBarNotification value = active.get(key(tag, 1)); return value == null ? null : value.n; }
    }
    final Activity activity = new Activity();
    boolean ui(Callable<Boolean> callback) { try { return callback.call(); } catch (Exception error) { return false; } }
    PRODUCTION_METHODS
    static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    static JSONObject item(String id) { return new JSONObject().put("id", id).put("title", "PRIVATE " + id).put("state", "running").put("detail", "PRIVATE DETAIL"); }
    static String state(String key, long running, JSONObject... items) {
        JSONArray rows = new JSONArray(); rows.rows.addAll(Arrays.asList(items));
        JSONObject.fixtures.put(key, new JSONObject().put("running", running).put("waiting", 0).put("errored", 0).put("items", rows));
        return key;
    }
    static String tag(String id) { return AGENT_CHILD_PREFIX + Uri.encode(id); }
    public static void main(String[] ignored) {
        AgentContract service = new AgentContract(); NotificationManager m = service.activity.manager;
        Notification sentinel = new Notification();
        m.notify("turns", 77, sentinel); m.notify("unrelated", 1, sentinel); m.notify("agents.session.foreign-id", 2, sentinel);
        JSONObject aa = item("Aa").put("plan", "PRIVATE PLAN").put("origin", "PRIVATE ORIGIN").put("since", 1L).put("total", 3).put("done", 999).put("active", 999);
        JSONObject bb = item("BB");
        check("Aa".hashCode() == "BB".hashCode(), "Fixture is not a Java hash collision");
        check(service.syncAgents(state("first", 2, aa, bb)), "Initial publish failed");
        Notification first = m.get(tag("Aa")), second = m.get(tag("BB")), summary = m.get("agents");
        check(first != null && second != null && summary != null && summary.summary, "Summary or independent children missing");
        check(!first.pending.identity.equals(second.pending.identity), "Hash-collision sessions share PendingIntent");
        check(first.title.equals("PRIVATE PLAN") && first.text.contains("3/3 todos") && !first.text.contains("active"), "Plan/progress bounds incorrect");
        check(first.timer && first.when == 1000, "State timer missing");
        for (Notification n : Arrays.asList(first, second, summary)) {
            check(n.visibility == Notification.VISIBILITY_PRIVATE && n.local && n.ongoing && n.onlyOnce, "Private/silent ongoing contract lost");
            check(n.publicVersion != null && n.publicVersion.pending == null && n.publicVersion.title.equals("jcode agents") && n.publicVersion.text.equals("Open Jcode to view agent activity"), "Public fallback contains account details");
            check(n.sound == null && n.defaults == 0, "Not silent");
            check(n.alert == (n.summary ? Notification.GROUP_ALERT_CHILDREN : Notification.GROUP_ALERT_SUMMARY), "Group mute contract lost");
            check((n.pending.flags & PendingIntent.FLAG_IMMUTABLE) != 0, "Mutable session PendingIntent");
        }
        String stable = first.pending.identity;
        aa.put("title", "Updated").put("plan", "");
        check(service.syncAgents(state("reorder", 2, bb, aa)), "Reorder failed");
        check(m.get(tag("Aa")).pending.identity.equals(stable) && m.get(tag("Aa")).sort.equals("01"), "Identity depends on rank");
        check(m.get(tag("Aa")).title.equals("Updated"), "Content update not applied");
        check(!service.syncAgents("malformed") && m.get(tag("Aa")) != null, "Malformed payload acknowledged or changed active state");
        check(!service.syncAgents(state("bad-count", -1, aa)), "Negative count acknowledged");
        check(!service.syncAgents(state("bad-id", 1, item(""))), "Empty route acknowledged");
        check(!service.syncAgents(state("bad-status", 1, item("x").put("state", "unknown"))), "Unknown state acknowledged");
        char[] huge = new char[MAX_AGENT_JSON + 1]; Arrays.fill(huge, 'x');
        check(!service.syncAgents(new String(huge)), "Unbounded JSON accepted");
        JSONObject[] many = new JSONObject[10]; for (int i = 0; i < many.length; i++) many[i] = item("many-" + i);
        check(service.syncAgents(state("many", 10, many)), "Bounded many publish failed");
        int children = 0; for (StatusBarNotification n : m.getActiveNotifications()) if (ownedAgentNotification(n) && !n.tag.equals("agents")) children++;
        check(children == 6 && m.get(tag("Aa")) == null && m.get(tag("many-6")) == null, "Child bound/obsolete cleanup failed");
        JSONObject special = item("id/a +?%\u2713");
        check(service.syncAgents(state("special", 1, special)), "Encoded route failed");
        check(m.get(tag("id/a +?%\u2713")).pending.intent.data.value.equals("jcode://session/" + Uri.encode("id/a +?%\u2713")), "Session URI changed");
        m.enabled = false;
        check(!service.syncAgents(state("denied", 1, aa)), "Denied publish acknowledged");
        for (StatusBarNotification n : m.getActiveNotifications()) check(!ownedAgentNotification(n), "Denied update retained owned private state");
        m.notify(tag("stale"), 1, first); m.notify("agents", 1, summary);
        check(service.syncAgents(state("empty", 0)), "Denied teardown not acknowledged");
        check(m.get(tag("stale")) == null && m.get("agents") == null, "Denied teardown retained owned notifications");
        check(m.active.size() == 3, "Cleanup modified unrelated notifications");
        m.enabled = true; m.channel.importance = NotificationManager.IMPORTANCE_NONE;
        check(!service.syncAgents("first") && m.active.size() == 3, "Blocked channel acknowledged or unrelated modified");
        m.channel.importance = NotificationManager.IMPORTANCE_LOW; m.failAfter = 1;
        check(!service.syncAgents("first"), "Partial publish falsely acknowledged");
        m.failAfter = -1; check(service.syncAgents("empty"), "Partial-failure teardown failed");
        check(m.active.size() == 3, "Partial-failure teardown modified unrelated notifications");
        m.hideActive = true;
        check(service.syncAgents("first") && service.agentNotificationTags.size() == 2, "Attempted tags not retained");
        check(service.syncAgents("empty") && service.agentNotificationTags.isEmpty(), "Fast unenumerated teardown failed");
        check(m.active.size() == 3, "Fast unenumerated teardown leaked private rows");
        m.hideActive = false;
        check(agentHeadline(3, 1, 2).equals("1 needs input, 2 failed, 3 running"), "Headline drift");
        check(agentHeadline(0, 2, 0).equals("2 need input"), "Plural headline drift");
        System.out.println("PASS: actual syncAgents source, synthetic OS only, bounds/privacy/identity/cleanup/ack failures");
    }
}
