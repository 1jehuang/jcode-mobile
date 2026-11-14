package org.jcode.mobile.tests;

import android.app.Instrumentation;
import android.os.Bundle;
import org.json.JSONArray;
import org.json.JSONObject;
import java.lang.reflect.Method;

/** Runs the real packaged JNI library on Android, not a host mock or Java demo. */
public final class SmokeInstrumentation extends Instrumentation {
    private Bundle arguments;
    private Method bridge;
    private int checks;

    @Override public void onCreate(Bundle args) {
        super.onCreate(args);
        arguments = args == null ? new Bundle() : args;
        start();
    }

    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            Class<?> nativeBridge = Class.forName("org.jcode.mobile.NativeBridge", true,
                    getTargetContext().getClassLoader());
            bridge = nativeBridge.getMethod("request", String.class);
            runChecks();
            result.putString("stream", "\nJCODE_TESTS_PASSED: " + checks + " assertions on Android API "
                    + android.os.Build.VERSION.SDK_INT + " / " + android.os.Build.SUPPORTED_ABIS[0] + "\n");
            finish(-1, result);
        } catch (Throwable error) {
            result.putString("stream", "\nJCODE_TESTS_FAILED\n" + android.util.Log.getStackTraceString(error));
            finish(0, result);
        }
    }

    private JSONObject request(String action, String... fields) throws Exception {
        JSONObject input = new JSONObject().put("action", action);
        for (int i = 0; i < fields.length; i += 2) input.put(fields[i], fields[i + 1]);
        String raw = (String) bridge.invoke(null, input.toString());
        require(raw != null, "JNI returned null");
        return new JSONObject(raw);
    }

    private void require(boolean value, String message) {
        if (!value) throw new AssertionError(message);
        checks++;
    }

    private JSONObject waitFor(String field, String value) throws Exception {
        long deadline = android.os.SystemClock.elapsedRealtime() + 20000;
        JSONObject state;
        do {
            state = request("poll");
            if (value.equals(state.optString(field))) return state;
            if ("failed".equals(state.optString("status"))) throw new AssertionError("Connection failed: " + state);
            Thread.sleep(50);
        } while (android.os.SystemClock.elapsedRealtime() < deadline);
        throw new AssertionError("Timed out waiting for " + field + "=" + value + ": " + state);
    }

    private JSONObject waitForText(String text) throws Exception {
        long deadline = android.os.SystemClock.elapsedRealtime() + 20000;
        JSONObject state;
        do {
            state = request("poll");
            JSONArray messages = state.optJSONArray("messages");
            if (messages != null) for (int i = 0; i < messages.length(); i++) {
                if (messages.getJSONObject(i).optString("text").contains(text)) return state;
            }
            Thread.sleep(50);
        } while (android.os.SystemClock.elapsedRealtime() < deadline);
        throw new AssertionError("Timed out waiting for transcript text " + text + ": " + state);
    }

    private JSONObject waitForReplyAfter(String userMessage) throws Exception {
        long deadline = android.os.SystemClock.elapsedRealtime() + 20000;
        JSONObject state;
        do {
            state = request("poll");
            JSONArray messages = state.optJSONArray("messages");
            boolean foundUser = false;
            if (messages != null) for (int i = 0; i < messages.length(); i++) {
                JSONObject message = messages.getJSONObject(i);
                if (userMessage.equals(message.optString("text"))) foundUser = true;
                else if (foundUser && "assistant".equals(message.optString("role"))
                        && message.optString("text").contains("actual streamed reply")
                        && !state.optBoolean("is_processing") && "connected".equals(state.optString("status"))) return state;
            }
            Thread.sleep(50);
        } while (android.os.SystemClock.elapsedRealtime() < deadline);
        throw new AssertionError("Timed out waiting for a fresh completed reply: " + state);
    }

    private void runChecks() throws Exception {
        require("disconnected".equals(request("disconnect").getString("status")), "disconnect must reset state");
        require(!request("send", "text", "not connected").getBoolean("ok"), "offline send must fail");
        require(!request("connect", "endpoint", "https://not-a-harness.invalid").getBoolean("ok"), "invalid endpoint must fail");
        require(!request("connect", "endpoint", "tcp://127.0.0.1:1", "token", "test-secret").toString().contains("test-secret"), "errors must not disclose secrets");
        require(!request("unknown_action").getBoolean("ok"), "unknown actions must fail safely");
        require(!request("start_login", "email", "not-an-email").getBoolean("ok"), "invalid cloud email must fail without networking");
        require(!request("verify_login", "code", "123456").getBoolean("ok"), "verification without a pending code must fail");
        require(!request("connect_cloud").getBoolean("ok"), "cloud connection requires sign-in");
        JSONObject demo = request("demo");
        require(demo.getBoolean("offline_demo"), "demo must be explicitly labeled");
        require("demo".equals(demo.getString("status")), "native demo status missing");
        require(!request("send", "text", "  ").getBoolean("ok"), "blank message must fail");
        // JNI modified UTF-8 must preserve actual Unicode, multiline text and emoji.
        String unicode = "Android JNI: caf\u00e9 \u4f60\u597d \ud83d\ude80\nsecond line";
        JSONObject sent = request("send", "text", unicode);
        require(sent.getBoolean("ok"), "native demo send failed");
        boolean unicodePreserved = false;
        JSONArray unicodeMessages = sent.getJSONArray("messages");
        for (int i = 0; i < unicodeMessages.length(); i++) {
            if (unicode.equals(unicodeMessages.getJSONObject(i).optString("text"))) unicodePreserved = true;
        }
        require(unicodePreserved, "Unicode, emoji or multiline transcript corrupted");
        JSONObject reset = request("disconnect");
        require(!reset.getBoolean("offline_demo"), "disconnect must exit native demo");
        require(reset.getJSONArray("messages").length() == 0, "disconnect must clear transcript");
        String endpoint = arguments.getString("endpoint", "");
        if (!endpoint.isEmpty()) {
            require(request("connect", "endpoint", endpoint).getBoolean("ok"), "fixture connect command failed");
            JSONObject connected = waitFor("status", "connected");
            require(!connected.getBoolean("offline_demo"), "fixture must use real networking");
            waitForText("earlier");
            long deadline = android.os.SystemClock.elapsedRealtime() + 10000;
            do {
                connected = request("poll");
                if (connected.getJSONArray("sessions").length() >= 2) break;
                Thread.sleep(50);
            } while (android.os.SystemClock.elapsedRealtime() < deadline);
            require(connected.getJSONArray("sessions").length() >= 2, "real server session list missing");
            require(request("set_model", "model", "fixture-alternate").getBoolean("ok"), "model selection failed");
            waitFor("model", "fixture-alternate");
            String roundTripMessage = "Android emulator round trip " + java.util.UUID.randomUUID();
            require(request("send", "text", roundTripMessage).getBoolean("ok"), "real send failed");
            waitForReplyAfter(roundTripMessage);
            require(request("select_session", "session_id", "s2").getBoolean("ok"), "select session failed");
            waitFor("selected_session_id", "s2");
            require(request("wake").getBoolean("ok"), "foreground wake failed");
            require("disconnected".equals(request("disconnect").getString("status")), "real disconnect failed");
        }
        String rejectEndpoint = arguments.getString("reject_endpoint", "");
        String slowEndpoint = arguments.getString("slow_endpoint", "");
        if (!slowEndpoint.isEmpty()) {
            request("connect", "endpoint", slowEndpoint);
            waitFor("status", "connected");
            waitForText("earlier");
            String cancelMessage = "Cancel Android turn " + java.util.UUID.randomUUID();
            require(request("send", "text", cancelMessage).getBoolean("ok"), "cancellable send failed");
            long deadline = android.os.SystemClock.elapsedRealtime() + 10000;
            boolean sawPartial = false;
            JSONObject partialState = new JSONObject();
            do {
                partialState = request("poll");
                JSONArray messages = partialState.getJSONArray("messages");
                for (int i = 0; i < messages.length(); i++) {
                    JSONObject message = messages.getJSONObject(i);
                    if ("assistant".equals(message.optString("role")) && "actual".equals(message.optString("text").trim()) && message.optBoolean("streaming")) sawPartial = true;
                }
                if (!sawPartial) Thread.sleep(50);
            } while (!sawPartial && android.os.SystemClock.elapsedRealtime() < deadline);
            require(sawPartial, "slow fixture must deliver partial streaming text before cancellation: " + partialState);
            require(request("cancel").getBoolean("ok"), "cancellation command failed");
            JSONObject cancelled = waitFor("is_processing", "false");
            JSONArray messages = cancelled.getJSONArray("messages");
            require(!messages.getJSONObject(messages.length() - 1).getString("text").contains("streamed reply"), "cancellation must stop before full reply");
            request("disconnect");
        }
        if (!rejectEndpoint.isEmpty()) {
            require(request("connect", "endpoint", rejectEndpoint).getBoolean("ok"), "rejection fixture connect failed");
            JSONObject failed = waitFor("status", "failed");
            require(!failed.isNull("error"), "failed connection must expose a safe actionable error");
            require(!request("send", "text", "must not send").getBoolean("ok"), "failed connection must not allow send");
            request("disconnect");
        }
        String reconnectEndpoint = arguments.getString("reconnect_endpoint", "");
        if (!reconnectEndpoint.isEmpty()) {
            require(request("connect", "endpoint", reconnectEndpoint).getBoolean("ok"), "reconnect fixture connect failed");
            waitFor("status", "connected");
            waitForText("earlier");
            String originalSession = request("poll").getString("selected_session_id");
            String message = "Reconnect exactly once from Android " + java.util.UUID.randomUUID();
            require(request("send", "text", message).getBoolean("ok"), "reconnect fixture send failed");
            JSONObject recovered = waitForReplyAfter(message);
            require(originalSession.equals(recovered.getString("selected_session_id")), "reconnect must resume original session");
            int occurrences = 0;
            JSONArray history = recovered.getJSONArray("messages");
            for (int i = 0; i < history.length(); i++) {
                if (message.equals(history.getJSONObject(i).optString("text"))) occurrences++;
            }
            require(occurrences == 1, "reconnect must not duplicate a sent message");
            request("disconnect");
        }
        require("signed_out".equals(request("logout").getString("login_state")), "logout must clear cloud account state");
    }
}
