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

    private void runChecks() throws Exception {
        require("disconnected".equals(request("disconnect").getString("status")), "disconnect must reset state");
        require(!request("send", "text", "not connected").getBoolean("ok"), "offline send must fail");
        require(!request("connect", "endpoint", "https://not-a-harness.invalid").getBoolean("ok"), "invalid endpoint must fail");
        require(!request("connect", "endpoint", "tcp://127.0.0.1:1", "token", "test-secret").toString().contains("test-secret"), "errors must not disclose secrets");
        require(!request("unknown_action").getBoolean("ok"), "unknown actions must fail safely");
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
            require(request("send", "text", "Android emulator round trip").getBoolean("ok"), "real send failed");
            waitForText("actual streamed reply");
            require(request("select_session", "session_id", "s2").getBoolean("ok"), "select session failed");
            waitFor("selected_session_id", "s2");
            require(request("wake").getBoolean("ok"), "foreground wake failed");
            require("disconnected".equals(request("disconnect").getString("status")), "real disconnect failed");
        }
    }
}
