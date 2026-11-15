package org.jcode.mobile;

import android.app.Activity;
import android.content.Context;
import android.graphics.Rect;
import android.os.Build;
import android.os.Bundle;
import android.view.MotionEvent;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.view.accessibility.AccessibilityEvent;
import android.view.accessibility.AccessibilityManager;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityNodeProvider;
import java.lang.ref.WeakReference;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.WeakHashMap;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/** OS accessibility service adapter for GPUI's existing native content view.
 * No widgets, application state, drawing, layout, or Java application UI.
 * All methods that touch Android views execute on the Java UI thread. Native
 * callbacks enqueue work for GPUI's native UI loop, never mutate GPUI here.
 */
@SuppressWarnings("deprecation") // Compatibility factories/actions remain required on Android 8/API26.
public final class GpuiAccessibility extends AccessibilityNodeProvider {
    private static final int NONE = -1;
    private static final int MAX_NODES = 16384;
    private static final WeakHashMap<Activity, WeakReference<GpuiAccessibility>> ADAPTERS = new WeakHashMap<>();
    private static final WeakHashMap<Activity, Long> TOKENS = new WeakHashMap<>();
    private static final WeakHashMap<Activity, String> PENDING = new WeakHashMap<>();
    private final View host;
    private final long token;
    private final AccessibilityManager manager;
    private final AccessibilityManager.AccessibilityStateChangeListener stateListener;
    private final LinkedHashMap<Integer, Item> items = new LinkedHashMap<>();
    private int accessibilityFocus = NONE;
    private int inputFocus = NONE;
    private int hovered = NONE;
    private boolean disposed;
    // Content-free integration counters for diagnosing real service discovery.
    // They never retain labels, values, event payloads or user input.
    private long hostQueries, virtualQueries, accessibilityFocusRequests, hoverEvents;

    private static native void nativeStatus(long token, boolean enabled);
    private static native boolean nativeAction(long token, int node, int action, String value);

    private GpuiAccessibility(View host, long token) {
        this.host = host;
        this.token = token;
        manager = (AccessibilityManager) host.getContext().getSystemService(Context.ACCESSIBILITY_SERVICE);
        stateListener = enabled -> nativeStatus(token, enabled);
        host.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_YES);
        host.setAccessibilityDelegate(new View.AccessibilityDelegate() {
            @Override public AccessibilityNodeProvider getAccessibilityNodeProvider(View view) {
                return GpuiAccessibility.this;
            }
        });
        host.setOnHoverListener((view, event) -> hover(event));
        if (manager != null) {
            manager.addAccessibilityStateChangeListener(stateListener);
            nativeStatus(token, manager.isEnabled());
        }
    }

    /** Called via JNI. Android view installation is marshalled to Java UI. */
    public static void install(Activity activity, long token) {
        activity.runOnUiThread(() -> {
            Long existingToken = TOKENS.get(activity);
            if (existingToken != null && existingToken.longValue() == token && current(activity, token) != null) return;
            GpuiAccessibility old = currentAny(activity);
            if (old != null) old.close();
            TOKENS.put(activity, token);
            installNow(activity, token, 0);
        });
    }

    private static void installNow(Activity activity, long token, int attempt) {
        Long requested = TOKENS.get(activity);
        if (requested == null || requested.longValue() != token || activity.isFinishing() || activity.isDestroyed()) return;
        // NativeActivity takes a Surface through NativeContentView, which is a plain
        // View, not a SurfaceView. Only inspect application content, never system decor.
        View content = activity.findViewById(android.R.id.content);
        View surface = findNativeHost(content);
        if (surface == null || !surface.isAttachedToWindow()) {
            if (attempt < 100) activity.getWindow().getDecorView().postDelayed(() -> installNow(activity, token, attempt + 1), 50);
            return;
        }
        GpuiAccessibility adapter = new GpuiAccessibility(surface, token);
        ADAPTERS.put(activity, new WeakReference<>(adapter));
        String snapshot = PENDING.remove(activity);
        if (snapshot != null) adapter.apply(snapshot);
    }

    private static View findNativeHost(View content) {
        // Prefer the framework's actual takeSurface owner over any nested SurfaceView.
        View nativeContent = findHostKind(content, true);
        return nativeContent != null ? nativeContent : findHostKind(content, false);
    }

    private static View findHostKind(View view, boolean nativeContent) {
        if (view == null) return null;
        // Do not infer a host from size, focusability, or a generic View: that can
        // select the non-drawing GpuiInput endpoint or a system/decor container.
        if (nativeContent
                ? "android.app.NativeActivity$NativeContentView".equals(view.getClass().getName())
                : view instanceof SurfaceView) return view;
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup) view;
            for (int index = 0; index < group.getChildCount(); index++) {
                View found = findHostKind(group.getChildAt(index), nativeContent);
                if (found != null) return found;
            }
        }
        return null;
    }

    /** Called via JNI with a complete immutable semantic snapshot. */
    public static void update(Activity activity, long token, String snapshot) {
        activity.runOnUiThread(() -> {
            Long requested = TOKENS.get(activity);
            if (requested == null || requested.longValue() != token) return;
            GpuiAccessibility adapter = current(activity, token);
            if (adapter != null) adapter.apply(snapshot);
            else PENDING.put(activity, snapshot);
        });
    }

    public static void dispose(Activity activity, long token) {
        activity.runOnUiThread(() -> {
            Long requested = TOKENS.get(activity);
            if (requested == null || requested.longValue() != token) return;
            GpuiAccessibility adapter = current(activity, token);
            if (adapter != null) adapter.close();
            ADAPTERS.remove(activity);
            TOKENS.remove(activity);
            PENDING.remove(activity);
        });
    }

    private static GpuiAccessibility currentAny(Activity activity) {
        WeakReference<GpuiAccessibility> reference = ADAPTERS.get(activity);
        return reference == null ? null : reference.get();
    }
    private static GpuiAccessibility current(Activity activity, long token) {
        GpuiAccessibility adapter = currentAny(activity);
        return adapter != null && adapter.token == token && !adapter.disposed ? adapter : null;
    }

    private void close() {
        if (disposed) return;
        disposed = true;
        if (manager != null) manager.removeAccessibilityStateChangeListener(stateListener);
        host.setAccessibilityDelegate(null);
        host.setOnHoverListener(null);
        items.clear();
        // Do not call nativeStatus here: the native bridge can already be gone.
    }

    private void apply(String snapshot) {
        if (disposed) return;
        try {
            JSONObject root = new JSONObject(snapshot);
            JSONArray nodes = root.getJSONArray("nodes");
            if (nodes.length() > MAX_NODES) return;
            LinkedHashMap<Integer, Item> next = new LinkedHashMap<>();
            for (int index = 0; index < nodes.length(); index++) {
                Item item = new Item(nodes.getJSONObject(index));
                if (next.put(item.id, item) != null) throw new JSONException("duplicate virtual ID");
            }
            int nextInputFocus = root.optInt("focus", NONE);
            if (accessibilityFocus != NONE && !next.containsKey(accessibilityFocus)) {
                send(accessibilityFocus, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED);
                accessibilityFocus = NONE;
            }
            LinkedHashMap<Integer, Item> previous = new LinkedHashMap<>(items);
            items.clear();
            items.putAll(next);
            for (Item item : items.values()) {
                Item old = previous.get(item.id);
                if (old != null && !old.password && !item.password && !old.value.equals(item.value)) {
                    AccessibilityEvent changed = AccessibilityEvent.obtain(AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED);
                    changed.setSource(host, item.id);
                    changed.setPackageName(host.getContext().getPackageName());
                    changed.setClassName(item.className);
                    changed.setBeforeText(old.value);
                    changed.setFromIndex(0);
                    changed.setRemovedCount(old.value.length());
                    changed.setAddedCount(item.value.length());
                    changed.getText().add(item.value);
                    emit(changed);
                }
            }
            if (inputFocus != nextInputFocus && next.containsKey(nextInputFocus)) {
                inputFocus = nextInputFocus;
                send(inputFocus, AccessibilityEvent.TYPE_VIEW_FOCUSED);
            } else inputFocus = next.containsKey(nextInputFocus) ? nextInputFocus : NONE;
            if (!items.containsKey(hovered)) hovered = NONE;
            AccessibilityEvent event = AccessibilityEvent.obtain(AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED);
            event.setContentChangeTypes(AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE);
            event.setSource(host);
            event.setPackageName(host.getContext().getPackageName());
            emit(event);
        } catch (JSONException | IllegalArgumentException ignored) {
            // Malformed or stale snapshots never partially replace the tree.
            // Do not log labels, values, or other user/private content.
        }
    }

    @Override public AccessibilityNodeInfo createAccessibilityNodeInfo(int virtualId) {
        if (disposed) return null;
        if (virtualId == HOST_VIEW_ID) {
            hostQueries++;
            AccessibilityNodeInfo info = AccessibilityNodeInfo.obtain(host);
            host.onInitializeAccessibilityNodeInfo(info);
            info.setClassName("android.view.View");
            info.setSource(host);
            for (Integer id : items.keySet()) info.addChild(host, id);
            return info;
        }
        virtualQueries++;
        Item item = items.get(virtualId);
        if (item == null) return null;
        AccessibilityNodeInfo info = AccessibilityNodeInfo.obtain();
        info.setSource(host, virtualId);
        info.setParent(host);
        info.setPackageName(host.getContext().getPackageName());
        info.setClassName(item.className);
        info.setEnabled(item.enabled);
        // Virtual nodes are allocated independently of View initialization. Unlike
        // the real host, they do not inherit IMPORTANT_FOR_ACCESSIBILITY_YES.
        // Explicitly expose GPUI's semantic nodes to service-side tree traversal.
        info.setImportantForAccessibility(true);
        info.setFocusable(true);
        info.setFocused(inputFocus == virtualId);
        info.setAccessibilityFocused(accessibilityFocus == virtualId);
        info.setSelected(item.selected);
        info.setPassword(item.password);
        info.setEditable(item.editable);
        info.setContentDescription(item.label.isEmpty() ? null : item.label);
        if (!item.value.isEmpty() && !item.password) info.setText(item.value);
        info.setCheckable(item.checked != null);
        if (item.checked != null) info.setChecked(item.checked.booleanValue());
        if (Build.VERSION.SDK_INT >= 28) info.setHeading(item.heading);
        info.setBoundsInParent(item.bounds);
        Rect visible = new Rect();
        boolean shown = host.isShown() && host.getLocalVisibleRect(visible) && visible.intersect(item.bounds);
        info.setVisibleToUser(shown);
        Rect screen = new Rect(item.bounds);
        int[] location = new int[2];
        host.getLocationOnScreen(location);
        screen.offset(location[0], location[1]);
        info.setBoundsInScreen(screen);
        info.addAction(accessibilityFocus == virtualId ? AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS : AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS);
        if (item.enabled) {
            if ((item.actions & 1) != 0) { info.setClickable(true); info.addAction(AccessibilityNodeInfo.ACTION_CLICK); }
            if ((item.actions & 2) != 0) info.addAction(AccessibilityNodeInfo.ACTION_FOCUS);
            if ((item.actions & 32) != 0 && inputFocus == virtualId) info.addAction(AccessibilityNodeInfo.ACTION_CLEAR_FOCUS);
            if ((item.actions & 4) != 0) info.addAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD);
            if ((item.actions & 8) != 0) info.addAction(AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD);
            if ((item.actions & 16) != 0) info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_TEXT);
            info.setScrollable((item.actions & 12) != 0);
        }
        return info;
    }

    @Override public AccessibilityNodeInfo findFocus(int focus) {
        if (focus != AccessibilityNodeInfo.FOCUS_ACCESSIBILITY && focus != AccessibilityNodeInfo.FOCUS_INPUT) return null;
        int id = focus == AccessibilityNodeInfo.FOCUS_ACCESSIBILITY ? accessibilityFocus : inputFocus;
        return id == NONE ? null : createAccessibilityNodeInfo(id);
    }

    @Override public List<AccessibilityNodeInfo> findAccessibilityNodeInfosByText(String text, int virtualId) {
        List<AccessibilityNodeInfo> result = new ArrayList<>();
        if (text == null || (virtualId != HOST_VIEW_ID && !items.containsKey(virtualId))) return result;
        String query = text.toLowerCase(Locale.ROOT);
        for (Item item : items.values()) {
            if (virtualId != HOST_VIEW_ID && item.id != virtualId) continue;
            if (item.label.toLowerCase(Locale.ROOT).contains(query) || (!item.password && item.value.toLowerCase(Locale.ROOT).contains(query))) {
                result.add(createAccessibilityNodeInfo(item.id));
            }
        }
        return result;
    }

    @Override public boolean performAction(int id, int action, Bundle arguments) {
        Item item = items.get(id);
        if (disposed || item == null) return false;
        if (action == AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS) {
            accessibilityFocusRequests++;
            if (manager == null || !manager.isEnabled() || accessibilityFocus == id) return false;
            if (accessibilityFocus != NONE) send(accessibilityFocus, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED);
            accessibilityFocus = id;
            // Accessibility focus and input focus are distinct. Native chooses
            // supported Focus/ScrollIntoView, or leaves application focus alone.
            nativeAction(token, id, action, null);
            send(id, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED);
            host.invalidate();
            return true;
        }
        if (action == AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS) {
            if (accessibilityFocus != id) return false;
            accessibilityFocus = NONE;
            send(id, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED);
            host.invalidate();
            return true;
        }
        if (!item.enabled) return false;
        boolean supported = (action == AccessibilityNodeInfo.ACTION_CLICK && (item.actions & 1) != 0)
            || (action == AccessibilityNodeInfo.ACTION_FOCUS && (item.actions & 2) != 0)
            || (action == AccessibilityNodeInfo.ACTION_CLEAR_FOCUS && (item.actions & 32) != 0)
            || (action == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD && (item.actions & 4) != 0)
            || (action == AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD && (item.actions & 8) != 0)
            || (action == AccessibilityNodeInfo.ACTION_SET_TEXT && (item.actions & 16) != 0);
        if (!supported) return false;
        String value = null;
        if (action == AccessibilityNodeInfo.ACTION_SET_TEXT) {
            CharSequence input = arguments == null ? null : arguments.getCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE);
            if (input == null) return false;
            value = input.toString();
        }
        return nativeAction(token, id, action, value);
    }

    private boolean hover(MotionEvent event) {
        hoverEvents++;
        if (disposed || manager == null || !manager.isEnabled() || !manager.isTouchExplorationEnabled()) return false;
        int next = NONE;
        if (event.getActionMasked() != MotionEvent.ACTION_HOVER_EXIT) {
            if (event.getActionMasked() != MotionEvent.ACTION_HOVER_ENTER && event.getActionMasked() != MotionEvent.ACTION_HOVER_MOVE) return false;
            long area = Long.MAX_VALUE;
            for (Item item : items.values()) {
                if (item.bounds.contains((int) event.getX(), (int) event.getY())) {
                    long candidate = (long) item.bounds.width() * item.bounds.height();
                    if (candidate <= area) { next = item.id; area = candidate; }
                }
            }
        }
        if (hovered != next) {
            int old = hovered;
            hovered = next;
            if (next != NONE) send(next, AccessibilityEvent.TYPE_VIEW_HOVER_ENTER);
            if (old != NONE) send(old, AccessibilityEvent.TYPE_VIEW_HOVER_EXIT);
        }
        return next != NONE || event.getActionMasked() == MotionEvent.ACTION_HOVER_EXIT;
    }

    private void send(int id, int type) {
        AccessibilityEvent event = AccessibilityEvent.obtain(type);
        event.setSource(host, id);
        event.setPackageName(host.getContext().getPackageName());
        Item item = items.get(id);
        if (item != null) {
            event.setClassName(item.className);
            event.setEnabled(item.enabled);
            event.setPassword(item.password);
            if (!item.label.isEmpty()) event.setContentDescription(item.label);
            if (!item.password && !item.value.isEmpty()) event.getText().add(item.value);
        }
        emit(event);
    }
    private void emit(AccessibilityEvent event) {
        ViewParent parent = host.getParent();
        if (parent != null && manager != null && manager.isEnabled()) parent.requestSendAccessibilityEvent(host, event);
        else event.recycle();
    }

    private static final class Item {
        final int id;
        final String label, value, className;
        final Rect bounds;
        final boolean enabled, selected, password, heading, editable;
        final Boolean checked;
        final int actions;
        Item(JSONObject node) throws JSONException {
            id = node.getInt("id");
            if (id <= 0) throw new JSONException("invalid virtual ID");
            label = node.getString("label");
            value = node.getString("value");
            className = node.getString("class");
            JSONArray frame = node.getJSONArray("bounds");
            if (frame.length() != 4) throw new JSONException("invalid bounds");
            double left = frame.getDouble(0), top = frame.getDouble(1), right = frame.getDouble(2), bottom = frame.getDouble(3);
            if (!Double.isFinite(left) || !Double.isFinite(top) || !Double.isFinite(right) || !Double.isFinite(bottom)) throw new JSONException("nonfinite bounds");
            bounds = new Rect((int) Math.floor(left), (int) Math.floor(top), (int) Math.ceil(right), (int) Math.ceil(bottom));
            if (bounds.isEmpty()) throw new JSONException("empty bounds");
            enabled = node.getBoolean("enabled");
            selected = node.getBoolean("selected");
            password = node.getBoolean("password");
            heading = node.getBoolean("heading");
            editable = node.getBoolean("editable");
            checked = node.isNull("checked") ? null : Boolean.valueOf(node.getBoolean("checked"));
            actions = node.getInt("actions");
        }
    }
}
