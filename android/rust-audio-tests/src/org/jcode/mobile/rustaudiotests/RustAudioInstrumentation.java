package org.jcode.mobile.rustaudiotests;

import android.Manifest;
import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Bundle;
import android.os.SystemClock;
import dalvik.system.DexClassLoader;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.lang.reflect.Field;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/** Installed production Rust+Java, isolated UID, no INTERNET, no GPUI/auth startup. */
public final class RustAudioInstrumentation extends Instrumentation {
    private static native String nativeRun(Object productionServices, int scenario);
    private static void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }
    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); start(); }
    private static Field field(Class<?> type, String name) throws Exception {
        Field result = type.getDeclaredField(name); result.setAccessible(true); return result;
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Throwable failure = null;
        Activity activity = null;
        Object helper = null;
        Class<?> services = null;
        File library = null;
        try {
            Context context = getTargetContext();
            check("org.jcode.mobile.rustaudiotests".equals(context.getPackageName()), "Wrong isolated package");
            check(context.checkSelfPermission(Manifest.permission.INTERNET) == PackageManager.PERMISSION_DENIED, "Network permission forbidden");
            check(context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED, "Grant test UID microphone first");
            android.content.pm.ApplicationInfo production = context.getPackageManager().getApplicationInfo("org.jcode.mobile", 0);
            check(production.uid != android.os.Process.myUid(), "Must not run as production UID");
            check((production.flags & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0, "Debug production APK required");
            activity = startActivitySync(new Intent(context, AudioActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
            waitForIdleSync();
            long deadline = SystemClock.elapsedRealtime() + 3000;
            final Activity host = activity;
            final boolean[] focused = {false};
            do {
                runOnMainSync(() -> focused[0] = host.hasWindowFocus());
                if (focused[0]) break;
                SystemClock.sleep(20);
            } while (SystemClock.elapsedRealtime() < deadline);
            check(focused[0], "Foreground Activity missing. Unlock emulator");
            // Load the exact installed native library, not a test implementation.
            // Only the library is copied. Microphone PCM/WAV never leaves native memory.
            library = new File(context.getCodeCacheDir(), "libinstalled_jcode_mobile.so");
            try (ZipFile zip = new ZipFile(production.sourceDir)) {
                ZipEntry entry = null;
                for (String abi : android.os.Build.SUPPORTED_ABIS) {
                    entry = zip.getEntry("lib/" + abi + "/libjcode_mobile.so");
                    if (entry != null) break;
                }
                check(entry != null, "Installed APK lacks supported native ABI");
                try (InputStream input = zip.getInputStream(entry); FileOutputStream output = new FileOutputStream(library)) {
                    byte[] buffer = new byte[65536]; int count;
                    while ((count = input.read(buffer)) != -1) output.write(buffer, 0, count);
                }
            }
            System.load(library.getAbsolutePath());
            DexClassLoader loader = new DexClassLoader(production.sourceDir, context.getCodeCacheDir().getPath(), null, getClass().getClassLoader());
            services = loader.loadClass("org.jcode.mobile.AndroidServices");
            Class<?> unsafeType = Class.forName("sun.misc.Unsafe");
            Object unsafe = field(unsafeType, "theUnsafe").get(null);
            helper = unsafeType.getMethod("allocateInstance", Class.class).invoke(unsafe, services);
            field(services, "activity").set(helper, activity);
            for (int scenario = 0; scenario < 5; scenario++) {
                field(services, "foreground").setBoolean(helper, true);
                String evidence = nativeRun(helper, scenario);
                check(evidence != null && evidence.startsWith("PASS "), "Native scenario " + scenario + ": " + evidence);
                check(field(services, "recorder").get(helper) == null, "Scenario retained AudioRecord " + scenario);
                result.putString("scenario" + scenario, evidence);
            }
            result.putString("limits", "Real installed production JNI worker, isolated UID and injected service lifecycle. No shared GPUI controls, actual Activity wiring, acoustic fidelity, privacy switch/focus, 300s limit or transcription/send acceptance.");
        } catch (Throwable error) { failure = error; }
        finally {
            try {
                if (helper != null) services.getMethod("audioStop").invoke(helper);
                if (activity != null) { final Activity host = activity; runOnMainSync(() -> host.finish()); }
                if (library != null && library.exists()) check(library.delete(), "Native test copy cleanup failed");
            } catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
        }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", android.util.Log.getStackTraceString(failure));
        finish(failure == null ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
}
