package org.jcode.mobile.nativeservicestests;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.net.Uri;
import android.os.Bundle;
import java.io.File;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;

/** Owner-side setup/cleanup only. External access is tested by a DIFFERENT UID. */
public final class CameraOwnerInstrumentation extends Instrumentation {
    private Bundle args;
    private static final String TARGET = "org.jcode.mobile";
    private static final String TEST = "org.jcode.mobile.nativeservicestests";
    private static final int GRANTS = Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION;
    @Override public void onCreate(Bundle args) { super.onCreate(args); this.args = args; start(); }
    private static void check(boolean value, String message) { StorageInstrumentation.check(value, message); }
    private static Field field(Class<?> type, String name) throws Exception {
        Field field = type.getDeclaredField(name); field.setAccessible(true); return field;
    }
    private static Method method(Class<?> type, String name, Class<?>... params) throws Exception {
        Method method = type.getDeclaredMethod(name, params); method.setAccessible(true); return method;
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Throwable failure = null;
        Object services = null;
        Class<?> type = null;
        File state = null;
        boolean cleanup = false;
        try {
            Context context = getTargetContext();
            check(TARGET.equals(context.getPackageName()), "Owner runner must target the production package");
            check(android.os.Process.myUid() == context.getApplicationInfo().uid, "Owner UID mismatch");
            check(context.getPackageManager().getApplicationInfo(TEST, 0).uid != android.os.Process.myUid(), "Test UID must differ");
            type = context.getClassLoader().loadClass("org.jcode.mobile.AndroidServices");
            Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
            Field singleton = field(unsafeClass, "theUnsafe");
            services = unsafeClass.getMethod("allocateInstance", Class.class).invoke(singleton.get(null), type);
            final StorageInstrumentation.PrivateActivity[] activity = new StorageInstrumentation.PrivateActivity[1];
            runOnMainSync(() -> activity[0] = new StorageInstrumentation.PrivateActivity(context));
            field(type, "activity").set(services, activity[0]);
            state = new File(context.getCacheDir(), "camera-grant-test.state");
            String phase = args.getString("phase", "");
            if ("setup".equals(phase)) {
                check(state.createNewFile(), "Refusing to replace preexisting camera test fixture");
                cleanup = true;
                method(type, "prepareCameraOutput").invoke(services);
                Uri uri = (Uri)field(type, "cameraUri").get(services);
                File file = (File)field(type, "cameraFile").get(services);
                check(uri != null && file != null && file.isFile() && file.length() == 0, "Production private camera fixture missing");
                check((TARGET + ".attachments").equals(uri.getAuthority()), "Capture output is not private provider");
                check(file.getCanonicalFile().getParentFile().equals(new File(context.getCacheDir(), "camera").getCanonicalFile()), "Capture output escaped cache");
                Uri malformed = uri.buildUpon().appendPath("..").appendPath("camera-0.jpg").build();
                try (android.os.ParcelFileDescriptor ignored = context.getContentResolver().openFileDescriptor(malformed, "r")) {
                    throw new AssertionError("Owner-side production provider accepted malformed camera path");
                } catch (java.io.FileNotFoundException expected) { /* Reach actual provider validation, not just grant checks. */ }
                Files.write(state.toPath(), uri.toString().getBytes(StandardCharsets.UTF_8));
                context.grantUriPermission(TEST, uri, GRANTS);
                result.putString("uri", uri.toString());
                result.putString("camera", "PASS: production private output prepared and exact URI granted to external test UID");
                cleanup = false; // The separate external runner now owns the write step.
            } else if ("cleanup".equals(phase)) {
                check(state.isFile(), "Camera test fixture state missing");
                Uri uri = Uri.parse(new String(Files.readAllBytes(state.toPath()), StandardCharsets.UTF_8));
                check(uri.toString().equals(args.getString("uri")), "Cleanup URI does not match our fixture");
                check((TARGET + ".attachments").equals(uri.getAuthority()) && uri.getPathSegments().size() == 1
                        && uri.getLastPathSegment().matches("camera-[0-9]+\\.jpg"), "Unsafe fixture URI");
                File file = new File(new File(context.getCacheDir(), "camera"), uri.getLastPathSegment());
                field(type, "cameraUri").set(services, uri);
                field(type, "cameraFile").set(services, file);
                cleanup = true;
                if (!"true".equals(args.getString("cleanupOnly"))) {
                    byte[] jpeg = (byte[])method(type, "jpeg", Uri.class).invoke(services, uri);
                    Bitmap decoded = BitmapFactory.decodeByteArray(jpeg, 0, jpeg.length);
                    check(decoded != null && decoded.getWidth() == 8 && decoded.getHeight() == 6, "Production JPEG conversion lost external camera payload");
                    decoded.recycle();
                }
                method(type, "cleanupCamera").invoke(services);
                check(!file.exists() && field(type, "cameraUri").get(services) == null && field(type, "cameraFile").get(services) == null, "Production cleanup left camera output");
                check(state.delete(), "Camera fixture state cleanup failed");
                cleanup = false;
                result.putString("camera", "PASS: production JPEG conversion and cleanup deleted temporary output and revoked grants");
            } else throw new AssertionError("Expected setup or cleanup phase");
        } catch (Throwable error) { failure = error; }
        finally {
            if (cleanup && services != null && type != null) {
                try { method(type, "cleanupCamera").invoke(services); }
                catch (Throwable error) { if (failure == null) failure = error; else failure.addSuppressed(error); }
                if (state != null && state.exists() && !state.delete() && failure == null) failure = new AssertionError("Fixture state cleanup failed");
            }
        }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", android.util.Log.getStackTraceString(failure));
        finish(failure == null ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
}
