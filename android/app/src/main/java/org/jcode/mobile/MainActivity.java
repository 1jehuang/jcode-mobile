package org.jcode.mobile;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.SharedPreferences;
import android.graphics.Color;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.text.InputType;
import android.view.Gravity;
import android.view.View;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.widget.ArrayAdapter;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ListView;
import android.widget.ScrollView;
import android.widget.TextView;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;
import java.util.ArrayList;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** Native, dependency-free Android shell. All blocking JNI work runs on one worker. */
public final class MainActivity extends Activity {
    private static final int INK = Color.rgb(226, 232, 240);
    private static final int MUTED = Color.rgb(148, 163, 184);
    private static final int BACKGROUND = Color.rgb(15, 23, 42);
    // Process-wide serialization also prevents overlapping native calls across rotation.
    private static final ExecutorService WORKER = Executors.newSingleThreadExecutor();
    private final Handler main = new Handler(Looper.getMainLooper());
    private SharedPreferences preferences;
    private TextView status, heading, empty;
    private EditText composer;
    private Button send, connect, refresh, cloud;
    private String loginState = "signed_out";
    private boolean loginBusy;
    private static String loginEmail = "";
    private LinearLayout transcript;
    private ScrollView scroll;
    private ListView sessionList;
    private ArrayAdapter<String> sessionAdapter;
    private final ArrayList<String> sessionIds = new ArrayList<>();
    private boolean demo, resumed, destroyed, inFlight, connected, dialogOpen;
    private String selected = "", renderedMessages = "", renderedSessions = "", pendingText = "";
    private DemoState demoState;
    private AlertDialog activeDialog;
    private final Runnable poller = new Runnable() {
        @Override public void run() {
            if (!resumed || destroyed || dialogOpen) return;
            if (!inFlight) request("poll", new JSONObject(), false);
            else main.postDelayed(this, 500);
        }
    };

    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        preferences = getSharedPreferences("connection", MODE_PRIVATE);
        Object demoExtra = getIntent().getExtras() == null ? null : getIntent().getExtras().get("demo");
        demo = Boolean.TRUE.equals(demoExtra) || "true".equalsIgnoreCase(String.valueOf(demoExtra));
        if (saved != null) demo = saved.getBoolean("demo", demo);
        demoState = new DemoState();
        if (saved != null && demo) demoState.restore(saved.getString("demo_state"));
        buildUi();
        if (saved != null) composer.setText(saved.getString("draft", ""));
        request("status", new JSONObject(), false);
    }

    private void buildUi() {
        LinearLayout root = column();
        root.setBackgroundColor(BACKGROUND);
        root.setFitsSystemWindows(true);
        root.setPadding(dp(16), dp(12), dp(16), dp(8));
        setContentView(root);
        LinearLayout toolbar = row();
        TextView brand = label("Jcode", 26, INK);
        toolbar.addView(brand, new LinearLayout.LayoutParams(0, -2, 1));
        Button settings = button("Settings");
        settings.setOnClickListener(v -> showSettings());
        toolbar.addView(settings);
        root.addView(toolbar);
        status = label("Starting…", 14, MUTED);
        status.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        root.addView(status);
        LinearLayout actions = row();
        connect = button("Connect");
        connect.setOnClickListener(v -> {
            if (connected) request("disconnect", new JSONObject(), true);
            else connectSaved();
        });
        refresh = button("Refresh");
        refresh.setOnClickListener(v -> request("sessions", new JSONObject(), true));
        actions.addView(connect, new LinearLayout.LayoutParams(0, -2, 1));
        actions.addView(refresh, new LinearLayout.LayoutParams(0, -2, 1));
        cloud = button("Cloud sign in");
        cloud.setOnClickListener(v -> showCloud());
        actions.addView(cloud, new LinearLayout.LayoutParams(0, -2, 1));
        root.addView(actions);
        TextView sessionsLabel = label("SESSIONS", 12, MUTED);
        root.addView(sessionsLabel);
        sessionList = new ListView(this);
        sessionList.setChoiceMode(ListView.CHOICE_MODE_SINGLE);
        sessionList.setBackgroundColor(Color.rgb(30, 41, 59));
        sessionAdapter = new ArrayAdapter<String>(this, android.R.layout.simple_list_item_activated_1, new ArrayList<String>()) {
            @Override public View getView(int position, View convert, android.view.ViewGroup parent) {
                TextView view = (TextView) super.getView(position, convert, parent);
                view.setTextColor(INK);
                view.setTextSize(14);
                return view;
            }
        };
        sessionList.setAdapter(sessionAdapter);
        sessionList.setOnItemClickListener((parent, view, position, id) -> {
            if (!inFlight && position < sessionIds.size()) {
                JSONObject args = new JSONObject();
                put(args, "session_id", sessionIds.get(position));
                request("select_session", args, true);
            }
        });
        root.addView(sessionList, new LinearLayout.LayoutParams(-1, dp(142)));
        heading = label("Conversation", 18, INK);
        heading.setPadding(0, dp(12), 0, dp(8));
        root.addView(heading);
        scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        transcript = column();
        empty = label("Connect to your server, then choose a session.\nUse Settings → Demo mode to explore offline.", 16, MUTED);
        transcript.addView(empty);
        scroll.addView(transcript);
        root.addView(scroll, new LinearLayout.LayoutParams(-1, 0, 1));
        LinearLayout input = row();
        input.setGravity(Gravity.BOTTOM);
        composer = new EditText(this);
        composer.setTextColor(INK);
        composer.setHintTextColor(MUTED);
        composer.setHint("Message Jcode");
        composer.setContentDescription("Message to send");
        composer.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES);
        composer.setMinLines(1);
        composer.setMaxLines(5);
        composer.setImeOptions(EditorInfo.IME_ACTION_SEND);
        composer.setOnEditorActionListener((v, action, event) -> {
            if (action == EditorInfo.IME_ACTION_SEND) { sendMessage(); return true; }
            return false;
        });
        input.addView(composer, new LinearLayout.LayoutParams(0, -2, 1));
        send = button("Send");
        send.setOnClickListener(v -> sendMessage());
        input.addView(send);
        root.addView(input);
        // Preserve transcript and composer space when the IME or landscape reduces height.
        root.getViewTreeObserver().addOnGlobalLayoutListener(() -> {
            boolean compact = root.getHeight() < dp(480);
            int visibility = compact ? View.GONE : View.VISIBLE;
            if (sessionList.getVisibility() != visibility) {
                sessionList.setVisibility(visibility);
                sessionsLabel.setVisibility(visibility);
            }
        });
        updateControls();
    }

    private void sendMessage() {
        String text = composer.getText().toString();
        if (inFlight || !connected || selected.isEmpty() || text.trim().isEmpty()) return;
        JSONObject args = new JSONObject();
        put(args, "text", text);
        pendingText = text;
        request("send", args, true);
    }

    private void connectSaved() {
        String endpoint = preferences.getString("endpoint", "").trim();
        if (!demo && endpoint.isEmpty()) { showSettings(); return; }
        JSONObject args = new JSONObject();
        put(args, "endpoint", endpoint);
        put(args, "token", accessToken);
        request("connect", args, true);
    }

    private void request(String action, JSONObject args, boolean visible) {
        if (destroyed || inFlight) return;
        main.removeCallbacks(poller);
        inFlight = true;
        updateControls();
        if (visible) status.setText(action.equals("connect") ? "Connecting… network setup may take a moment" : "Working…");
        put(args, "action", action);
        final boolean demoRequest = demo;
        WORKER.execute(() -> {
            JSONObject response = null;
            String error = null;
            try {
                if (demoRequest) response = demoState.request(args);
                else {
                    if (!NativeBridge.isAvailable()) throw new IllegalStateException("Native core unavailable. Install a build with the Rust library, or enable Demo mode in Settings.");
                    String raw = NativeBridge.request(args.toString());
                    if (raw == null || raw.trim().isEmpty()) throw new JSONException("Native bridge returned an empty response");
                    response = new JSONObject(raw);
                }
            } catch (Exception | LinkageError e) {
                error = e.getMessage() == null ? e.getClass().getSimpleName() : e.getMessage();
            }
            final JSONObject result = response;
            final String failure = error;
            main.post(() -> {
                if (destroyed) return;
                inFlight = false;
                if (failure != null) {
                    status.setText(failure);
                    if (action.equals("connect") || action.equals("disconnect")) connected = false;
                }
                else {
                    applySnapshot(result);
                    if (action.equals("logout") && result.optBoolean("ok", true)) loginEmail = "";
                    if (action.equals("start_login") && result.optBoolean("ok", true) && loginState.equals("code_sent") && !loginBusy) main.post(this::showCloud);
                    if (action.equals("send") && result.optBoolean("ok", true)) {
                        if (composer.getText().toString().equals(pendingText)) composer.setText("");
                    }
                }
                updateControls();
                if (resumed && !dialogOpen && (demo || NativeBridge.isAvailable())) main.postDelayed(poller, 1000);
            });
        });
    }

    private void applySnapshot(JSONObject response) {
        JSONObject state = response.optJSONObject("state");
        if (state == null) state = response;
        String previousLoginState = loginState;
        loginState = state.optString("login_state", loginState);
        loginBusy = state.optBoolean("login_busy", false);
        if (!loginState.equals(previousLoginState) && resumed
                && (loginState.equals("code_sent") || (loginState.equals("signed_in") && previousLoginState.equals("code_sent")))) main.post(this::showCloud);
        String phase = state.optString("status", "Disconnected");
        connected = state.optBoolean("connected", phase.equalsIgnoreCase("connected") || phase.equalsIgnoreCase("ready") || phase.equalsIgnoreCase("demo"));
        String error = response.optString("error", "");
        String loginError = nullableString(state, "login_error");
        if (!loginError.isEmpty()) error = loginError;
        if (error.equals("null")) error = "";
        status.setText(!response.optBoolean("ok", true) || !error.isEmpty() ? "Error: " + (error.isEmpty() ? "Request failed" : error) : (demo ? "DEMO · " : "") + phase + (state.optBoolean("is_processing", false) ? " · Working…" : ""));
        if (loginBusy && error.isEmpty()) status.setText("Cloud sign-in in progress…");
        if (state.has("selected_session_id")) selected = nullableString(state, "selected_session_id");
        else if (state.has("session_id")) selected = nullableString(state, "session_id");
        JSONArray sessions = state.optJSONArray("sessions");
        if (sessions != null && !(selected + sessions.toString()).equals(renderedSessions)) {
            renderedSessions = selected + sessions.toString();
            sessionAdapter.setNotifyOnChange(false);
            sessionAdapter.clear();
            sessionIds.clear();
            int checked = -1;
            for (int i = 0; i < sessions.length(); i++) {
                JSONObject session = sessions.optJSONObject(i);
                if (session == null) continue;
                String id = session.optString("session_id", session.optString("id", ""));
                if (id.isEmpty()) continue;
                String title = session.optString("title", id);
                if (title.isEmpty() || title.equals("null")) title = id;
                String detail = session.optString("status", session.optString("working_dir", ""));
                if (id.equals(selected)) { checked = sessionIds.size(); heading.setText(title); }
                sessionIds.add(id);
                sessionAdapter.add(title + (detail.isEmpty() ? "" : "\n" + detail));
            }
            sessionAdapter.notifyDataSetChanged();
            sessionList.clearChoices();
            if (checked >= 0) sessionList.setItemChecked(checked, true);
        }
        JSONArray messages = state.optJSONArray("messages");
        if (messages == null) messages = state.optJSONArray("transcript");
        if (messages != null && !(selected + messages.toString()).equals(renderedMessages)) {
            boolean atBottom = scroll.getChildCount() == 0 || scroll.getChildAt(0).getHeight() - scroll.getHeight() - scroll.getScrollY() < dp(80);
            renderedMessages = selected + messages.toString();
            transcript.removeAllViews();
            if (messages.length() == 0) transcript.addView(label(selected.isEmpty() ? "Choose a session to view its conversation." : "No messages yet. Send the first message below.", 16, MUTED));
            for (int i = 0; i < messages.length(); i++) {
                JSONObject message = messages.optJSONObject(i);
                if (message == null) continue;
                String role = message.optString("role", "assistant");
                TextView name = label(role.toUpperCase(java.util.Locale.ROOT), 11, role.equals("user") ? Color.rgb(125, 211, 252) : MUTED);
                name.setPadding(0, dp(10), 0, dp(4));
                transcript.addView(name);
                TextView body = label(message.optString("text", message.optString("content", "")), 16, INK);
                body.setTextIsSelectable(true);
                body.setPadding(0, 0, 0, dp(12));
                transcript.addView(body);
                String reasoning = nullableString(message, "reasoning");
                if (!reasoning.isEmpty()) {
                    TextView thinking = label("Thinking\n" + reasoning, 14, MUTED);
                    thinking.setTextIsSelectable(true);
                    transcript.addView(thinking);
                }
                JSONArray tools = message.optJSONArray("tools");
                if (tools != null) for (int t = 0; t < tools.length(); t++) {
                    JSONObject tool = tools.optJSONObject(t);
                    if (tool == null) continue;
                    String output = nullableString(tool, "output");
                    TextView toolView = label("Tool: " + tool.optString("name", "tool") + (output.isEmpty() ? " · Running…" : "\n" + output), 13, MUTED);
                    toolView.setTextIsSelectable(true);
                    transcript.addView(toolView);
                }
            }
            if (atBottom) scroll.post(() -> scroll.fullScroll(View.FOCUS_DOWN));
        }
    }

    private void showCloud() {
        if (inFlight || destroyed || !resumed || dialogOpen) return;
        if (loginBusy) {
            present(new AlertDialog.Builder(this).setTitle("Cloud sign-in in progress")
                .setMessage("You can wait for sign-in to finish, or cancel and clear the in-memory account.")
                .setPositiveButton("Keep waiting", null)
                .setNegativeButton("Cancel sign-in", (dialog, which) -> { demo = false; request("logout", new JSONObject(), true); }).create());
            return;
        }
        if (loginState.equals("signed_in")) {
            present(new AlertDialog.Builder(this).setTitle("Jcode Cloud")
                .setMessage("Your account is signed in for this app process. Connect to your cloud workspace, or sign out to remove the in-memory account.")
                .setPositiveButton("Connect cloud", (dialog, which) -> { demo = false; request("connect_cloud", new JSONObject(), true); })
                .setNeutralButton("Sign out", (dialog, which) -> { demo = false; request("logout", new JSONObject(), true); })
                .setNegativeButton("Cancel", null).create());
            return;
        }
        boolean codeStep = loginState.equals("code_sent");
        EditText input = new EditText(this);
        input.setSingleLine(true);
        input.setHint(codeStep ? "Email verification code" : "Email address");
        input.setInputType(codeStep ? InputType.TYPE_CLASS_NUMBER : InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS);
        if (codeStep) input.setFilters(new android.text.InputFilter[]{new android.text.InputFilter.LengthFilter(6)});
        if (!codeStep) input.setText(loginEmail);
        LinearLayout panel = column();
        panel.setPadding(dp(20), dp(8), dp(20), dp(8));
        TextView explanation = new TextView(this);
        explanation.setText(codeStep ? "Enter the code sent to " + (loginEmail.isEmpty() ? "your email" : loginEmail) + ". Credentials stay in memory only." : "Sign in to Jcode Cloud with an email verification code. This will leave demo mode if enabled.");
        panel.addView(explanation);
        panel.addView(input);
        AlertDialog dialog = new AlertDialog.Builder(this).setTitle(codeStep ? "Verify email" : "Sign in to Jcode Cloud").setView(panel)
            .setPositiveButton(codeStep ? "Verify" : "Send code", null)
            .setNeutralButton(codeStep ? "Resend code" : "Cancel", (d, which) -> {
                if (codeStep && !loginEmail.isEmpty()) {
                    JSONObject args = new JSONObject(); put(args, "email", loginEmail);
                    demo = false; request("start_login", args, true);
                }
            }).setNegativeButton(codeStep ? "Cancel" : "Not now", null).create();
        dialog.setOnShowListener(ignored -> dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v -> {
            String value = input.getText().toString().trim();
            if (value.isEmpty() || (codeStep && !value.matches("[0-9]{6}")) || (!codeStep && (!value.contains("@") || value.endsWith("@")))) {
                input.setError(codeStep ? "Enter the six-digit verification code" : "Enter a valid email address");
                return;
            }
            JSONObject args = new JSONObject();
            put(args, codeStep ? "code" : "email", value);
            if (!codeStep) loginEmail = value;
            demo = false;
            dialog.dismiss();
            request(codeStep ? "verify_login" : "start_login", args, true);
        }));
        present(dialog);
    }

    private void showSettings() {
        if (inFlight || dialogOpen) return;
        LinearLayout panel = column();
        panel.setPadding(dp(20), dp(8), dp(20), dp(8));
        EditText endpoint = new EditText(this);
        endpoint.setHint("Server endpoint, e.g. tcp://10.0.2.2:7643");
        endpoint.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI);
        endpoint.setSingleLine(true);
        endpoint.setText(preferences.getString("endpoint", ""));
        panel.addView(endpoint);
        EditText token = new EditText(this);
        token.setHint("Access token (kept only until app closes)");
        token.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD);
        token.setSingleLine(true);
        token.setText(accessToken);
        panel.addView(token);
        TextView note = new TextView(this);
        note.setText("Use a forwarded Jcode TCP socket. Endpoint is saved locally, token is held in memory only. This core currently rejects nonempty tokens. Demo mode never contacts a server.");
        panel.addView(note);
        present(new AlertDialog.Builder(this).setTitle("Connection settings").setView(panel)
            .setPositiveButton("Save & connect", (dialog, which) -> {
                preferences.edit().putString("endpoint", endpoint.getText().toString().trim()).apply();
                accessToken = token.getText().toString();
                demo = false;
                connectSaved();
            })
            .setNeutralButton(demo ? "Exit demo" : "Demo mode", (dialog, which) -> {
                demo = !demo;
                selected = "";
                renderedMessages = "";
                request("status", new JSONObject(), false);
            }).setNegativeButton("Cancel", null).create());
    }

    private void present(AlertDialog dialog) {
        dialogOpen = true;
        activeDialog = dialog;
        main.removeCallbacks(poller);
        dialog.setOnDismissListener(ignored -> {
            dialogOpen = false;
            activeDialog = null;
            if (resumed && !destroyed && !inFlight) main.postDelayed(poller, 1000);
        });
        dialog.show();
    }

    private static String accessToken = "";
    private void updateControls() {
        send.setEnabled(!inFlight && connected && !selected.isEmpty());
        connect.setEnabled(!inFlight);
        refresh.setEnabled(!inFlight);
        cloud.setEnabled(!inFlight);
        cloud.setText(loginBusy ? "Cancel login" : loginState.equals("signed_in") ? "Cloud account" : "Cloud sign in");
        sessionList.setEnabled(!inFlight);
        connect.setText(connected ? "Disconnect" : "Connect");
    }
    @Override protected void onResume() {
        super.onResume();
        resumed = true;
        main.removeCallbacks(poller);
        if (!inFlight) request("wake", new JSONObject(), false);
        else main.postDelayed(poller, 1000);
    }
    @Override protected void onPause() {
        resumed = false;
        main.removeCallbacks(poller);
        super.onPause();
    }
    @Override protected void onSaveInstanceState(Bundle out) {
        out.putString("draft", composer.getText().toString());
        out.putBoolean("demo", demo);
        out.putString("demo_state", demoState.snapshot().toString());
        super.onSaveInstanceState(out);
    }
    @Override protected void onDestroy() {
        destroyed = true;
        if (activeDialog != null) activeDialog.dismiss();
        main.removeCallbacksAndMessages(null);
        super.onDestroy();
    }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
    private LinearLayout column() { LinearLayout v = new LinearLayout(this); v.setOrientation(LinearLayout.VERTICAL); return v; }
    private LinearLayout row() { LinearLayout v = new LinearLayout(this); v.setOrientation(LinearLayout.HORIZONTAL); return v; }
    private TextView label(String text, int size, int color) { TextView v = new TextView(this); v.setText(text); v.setTextSize(size); v.setTextColor(color); return v; }
    private Button button(String text) { Button b = new Button(this); b.setText(text); b.setAllCaps(false); return b; }
    private static void put(JSONObject json, String key, Object value) { try { json.put(key, value); } catch (JSONException e) { throw new IllegalArgumentException(e); } }
    private static String nullableString(JSONObject json, String key) { return json.isNull(key) ? "" : json.optString(key, ""); }

    /** Deterministic local fixture, independent of native library/network availability. */
    private static final class DemoState {
        private String selected = "demo-welcome";
        private boolean connected = true;
        private JSONArray welcome = new JSONArray(), review = new JSONArray();
        DemoState() {
            message(welcome, "assistant", "Welcome to Jcode for Android. This deterministic demo runs entirely on your device. Select a session or send a message to try the native chat interface.");
            message(review, "user", "Review the Android client plan.");
            message(review, "assistant", "The client uses standard Android views, a background JNI bridge, a session list, and a keyboard-friendly composer. No WebView is used.");
        }
        synchronized JSONObject request(JSONObject request) {
            String action = request.optString("action");
            if (action.equals("connect")) connected = true;
            if (action.equals("disconnect")) connected = false;
            if (action.equals("select_session")) selected = request.optString("session_id", selected);
            if (action.equals("send")) {
                JSONArray messages = selected.equals("demo-review") ? review : welcome;
                String text = request.optString("text");
                message(messages, "user", text);
                message(messages, "assistant", "Demo received: " + text + "\n\nConnect a server in Settings to chat with your live Jcode sessions.");
            }
            return snapshot();
        }
        synchronized JSONObject snapshot() {
            JSONObject out = new JSONObject();
            put(out, "ok", true); put(out, "connected", connected); put(out, "status", connected ? "Demo ready" : "Disconnected");
            put(out, "selected_session_id", selected);
            JSONArray sessions = new JSONArray();
            for (String[] fixture : new String[][]{{"demo-welcome", "Welcome to Jcode"}, {"demo-review", "Android client review"}}) {
                JSONObject item = new JSONObject(); put(item, "session_id", fixture[0]); put(item, "title", fixture[1]); put(item, "status", "Offline demo"); sessions.put(item);
            }
            put(out, "sessions", sessions); put(out, "messages", selected.equals("demo-review") ? review : welcome);
            put(out, "demo_welcome", welcome); put(out, "demo_review", review);
            return out;
        }
        synchronized void restore(String raw) {
            if (raw == null) return;
            try {
                JSONObject saved = new JSONObject(raw);
                selected = saved.optString("selected_session_id", selected);
                connected = saved.optBoolean("connected", true);
                if (saved.optJSONArray("demo_welcome") != null) welcome = saved.getJSONArray("demo_welcome");
                if (saved.optJSONArray("demo_review") != null) review = saved.getJSONArray("demo_review");
            } catch (JSONException ignored) { /* A malformed saved fixture resets to its defaults. */ }
        }
        private static void message(JSONArray target, String role, String text) { JSONObject message = new JSONObject(); put(message, "role", role); put(message, "text", text); target.put(message); }
    }
}
