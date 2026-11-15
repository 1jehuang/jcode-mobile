package org.jcode.mobile.audiotests;

import android.Manifest;
import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.media.AudioRecord;
import android.os.Bundle;
import android.os.SystemClock;
import dalvik.system.DexClassLoader;
import java.io.IOException;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationTargetException;

/** Invokes installed production Java, never a copied recorder or native voice sender. */
public final class AudioInstrumentation extends Instrumentation {
    private String mode;
    private Object services;
    private Class<?> type;
    private Activity activity;
    private static void check(boolean value, String message) {
        if (!value) throw new AssertionError(message);
    }
    @Override public void onCreate(Bundle args) {
        super.onCreate(args);
        mode = args == null ? null : args.getString("mode");
        start();
    }
    private Field field(String name) throws Exception {
        Field f = type.getDeclaredField(name); f.setAccessible(true); return f;
    }
    private Object call(String name) throws Exception { return type.getMethod(name).invoke(services); }
    private void stopped() throws Exception {
        check(field("recorder").get(services) == null, "Recorder retained after stop");
        try { call("audioRead"); throw new AssertionError("Read after stop succeeded"); }
        catch (InvocationTargetException expected) {
            check(expected.getCause() instanceof IOException, "Stopped read did not raise IOException");
        }
    }
    private void capture(String end) throws Exception {
        field("foreground").setBoolean(services, true);
        check(((Integer)call("audioStart")) == 0, "Real microphone start failed or unavailable");
        AudioRecord record = (AudioRecord)field("recorder").get(services);
        check(record != null && record.getState() == AudioRecord.STATE_INITIALIZED
                && record.getRecordingState() == AudioRecord.RECORDSTATE_RECORDING,
                "No live real AudioRecord");
        check(record.getSampleRate() == 16000 && record.getChannelCount() == 1
                && record.getAudioFormat() == android.media.AudioFormat.ENCODING_PCM_16BIT,
                "Production PCM format changed");
        check(((Integer)call("audioStart")) == -1 && field("recorder").get(services) == record,
                "Duplicate start replaced active recorder");
        long deadline = SystemClock.elapsedRealtime() + 2500;
        int bytes = 0;
        while (bytes < 3200 && SystemClock.elapsedRealtime() < deadline) {
            byte[] pcm = (byte[])call("audioRead");
            check(pcm != null && pcm.length <= 1600 && pcm.length % 2 == 0, "Invalid PCM chunk");
            bytes += pcm.length;
            SystemClock.sleep(10);
        }
        check(bytes >= 3200, "No bounded real PCM capture. Empty/unavailable input is not success");
        call(end);
        stopped();
        check(record.getState() == AudioRecord.STATE_UNINITIALIZED, "Native AudioRecord not released");
        call("audioStop"); stopped();
        if ("onPause".equals(end)) {
            check(!field("foreground").getBoolean(services), "Pause retained foreground state");
            check(((Integer)call("audioStart")) == -1, "Background recording started");
        }
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Throwable failure = null;
        SharedPreferences prefs = null;
        boolean hadRequested = false, previousRequested = false;
        try {
            Context context = getTargetContext();
            check("org.jcode.mobile.audiotests".equals(context.getPackageName()), "Wrong test UID context");
            check(context.getPackageManager().getApplicationInfo("org.jcode.mobile", 0).uid
                    != android.os.Process.myUid(), "Production UID must remain untouched");
            check(context.checkSelfPermission(Manifest.permission.INTERNET) != PackageManager.PERMISSION_GRANTED,
                    "Audio test must not have network permission");
            check("granted".equals(mode) || "denied".equals(mode), "Explicit granted/denied mode required");
            activity = startActivitySync(new Intent(context, AudioActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
            waitForIdleSync();
            long focusDeadline = SystemClock.elapsedRealtime() + 3000;
            final boolean[] focused = {false};
            do {
                runOnMainSync(() -> focused[0] = activity.hasWindowFocus());
                if (focused[0]) break;
                SystemClock.sleep(20);
            } while (SystemClock.elapsedRealtime() < focusDeadline);
            check(focused[0], "Audio Activity not foreground/focused. Unlock emulator first");
            String apk = context.getPackageManager().getApplicationInfo("org.jcode.mobile", 0).sourceDir;
            DexClassLoader loader = new DexClassLoader(apk, context.getCodeCacheDir().getPath(), null, getClass().getClassLoader());
            type = loader.loadClass("org.jcode.mobile.AndroidServices");
            // Constructor calls nativeInit. Isolate Java acceptance from native app/network startup.
            Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
            Field singleton = unsafeClass.getDeclaredField("theUnsafe"); singleton.setAccessible(true);
            Object unsafe = singleton.get(null);
            services = unsafeClass.getMethod("allocateInstance", Class.class).invoke(unsafe, type);
            field("activity").set(services, activity);
            field("foreground").setBoolean(services, true);
            if ("denied".equals(mode)) {
                check(activity.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_DENIED,
                        "Denied mode unexpectedly has microphone permission");
                check(!activity.shouldShowRequestPermissionRationale(Manifest.permission.RECORD_AUDIO),
                        "Denied test requires no-rationale state");
                prefs = activity.getSharedPreferences("permissions", Context.MODE_PRIVATE);
                hadRequested = prefs.contains("microphone-requested");
                previousRequested = prefs.getBoolean("microphone-requested", false);
                check(prefs.edit().putBoolean("microphone-requested", true).commit(), "Cannot prepare isolated requested state");
                check(((Integer)call("audioStart")) == -2, "Denied production start not explicit -2");
                stopped();
                result.putString("audio", "PASS: production permanent-denial branch, no recorder/read success");
            } else {
                check(activity.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED,
                        "Grant microphone to isolated audio test package first");
                capture("audioStop");
                capture("onPause");
                capture("onDestroy");
                result.putString("audio", "PASS: installed production AudioRecord PCM16 mono 16000, bounded nonempty reads, duplicate start, stop/restart, pause/destroy release, stopped-read failure");
            }
            result.putString("limits", "Isolated test UID and injected Java foreground flag. No JNI/Rust WAV, voice UI/send, acoustic fidelity, user permission dialog, or real Activity lifecycle wiring acceptance.");
        } catch (Throwable error) { failure = error; }
        finally {
            try {
                if (services != null) { call("audioStop"); stopped(); }
                if (prefs != null) {
                    SharedPreferences.Editor editor = prefs.edit();
                    if (hadRequested) editor.putBoolean("microphone-requested", previousRequested);
                    else editor.remove("microphone-requested");
                    check(editor.commit(), "Failed to restore isolated permission preferences");
                }
                if (activity != null) runOnMainSync(() -> activity.finish());
            } catch (Throwable cleanup) {
                if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup);
            }
        }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", android.util.Log.getStackTraceString(failure));
        finish(failure == null ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
}
