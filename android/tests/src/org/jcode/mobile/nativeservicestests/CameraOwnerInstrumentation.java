package org.jcode.mobile.nativeservicestests;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.ResultReceiver;
import java.io.File;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;

/** One owner instrumentation session, with a real separately-UIDed Activity recipient. */
public final class CameraOwnerInstrumentation extends Instrumentation {
    private static final String TARGET = "org.jcode.mobile";
    private static final String TEST = "org.jcode.mobile.nativeservicestests";
    private static final int GRANTS = Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION;
    @Override public void onCreate(Bundle args) { super.onCreate(args); start(); }
    private static void check(boolean value, String message) { StorageInstrumentation.check(value, message); }
    private static Field field(Class<?> type, String name) throws Exception {
        Field field = type.getDeclaredField(name); field.setAccessible(true); return field;
    }
    private static Method method(Class<?> type, String name, Class<?>... params) throws Exception {
        Method method = type.getDeclaredMethod(name, params); method.setAccessible(true); return method;
    }
    private static Bundle reply(Consumer<ResultReceiver> send) throws Exception {
        CountDownLatch ready = new CountDownLatch(1);
        AtomicReference<Bundle> result = new AtomicReference<>();
        AtomicInteger resultCode = new AtomicInteger(Activity.RESULT_CANCELED);
        ResultReceiver receiver = new ResultReceiver(new Handler(Looper.getMainLooper())) {
            @Override protected void onReceiveResult(int code, Bundle data) {
                resultCode.set(code); result.set(data); ready.countDown();
            }
        };
        send.accept(CameraResultReceiver.forIpc(receiver));
        check(ready.await(30, TimeUnit.SECONDS), "External camera recipient timed out");
        Bundle response = result.get();
        check(resultCode.get() == Activity.RESULT_OK && response != null && "PASS".equals(response.getString("result")),
                "External camera checks failed: " + (response == null ? "no reply" : response.getString("failure")));
        return response;
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Throwable failure = null;
        Object services = null;
        Class<?> type = null;
        ResultReceiver control = null;
        try {
            Context context = getTargetContext();
            check(TARGET.equals(context.getPackageName()), "Owner runner must target production package");
            check(android.os.Process.myUid() == context.getApplicationInfo().uid, "Owner UID mismatch");
            check(context.getPackageManager().getApplicationInfo(TEST, 0).uid != android.os.Process.myUid(), "External UID must differ");
            type = context.getClassLoader().loadClass("org.jcode.mobile.AndroidServices");
            Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
            services = unsafeClass.getMethod("allocateInstance", Class.class).invoke(field(unsafeClass, "theUnsafe").get(null), type);
            final StorageInstrumentation.PrivateActivity[] activity = new StorageInstrumentation.PrivateActivity[1];
            runOnMainSync(() -> activity[0] = new StorageInstrumentation.PrivateActivity(context));
            field(type, "activity").set(services, activity[0]);
            method(type, "prepareCameraOutput").invoke(services);
            Uri uri = (Uri)field(type, "cameraUri").get(services);
            File file = (File)field(type, "cameraFile").get(services);
            check(uri != null && file != null && file.isFile() && file.length() == 0, "Private production output missing");
            check((TARGET + ".attachments").equals(uri.getAuthority()), "Output not private provider");
            check(file.getCanonicalFile().getParentFile().equals(new File(context.getCacheDir(), "camera").getCanonicalFile()), "Output escaped cache");
            Uri malformed = uri.buildUpon().appendPath("..").appendPath("camera-0.jpg").build();
            try (android.os.ParcelFileDescriptor descriptor = context.getContentResolver().openFileDescriptor(malformed, "r")) {
                throw new AssertionError("Production provider accepted malformed path: " + (descriptor != null));
            } catch (java.io.FileNotFoundException expected) { /* Exercise provider validation under owner UID. */ }
            Bundle checks = new Bundle(); checks.putString("uri", uri.toString()); checks.putString("phase", "granted");
            Bundle granted = reply(receiver -> {
                Intent capture = new Intent().setClassName(TEST, TEST + ".CameraGrantActivity")
                        .putExtra("checks", checks).putExtra("reply", receiver)
                        .putExtra(android.provider.MediaStore.EXTRA_OUTPUT, uri)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | GRANTS);
                capture.setClipData(ClipData.newRawUri("camera", uri));
                context.startActivity(capture);
            });
            control = granted.getParcelable("control");
            check(control != null, "External recipient control missing");
            result.putString("external", granted.getString("camera"));
            // The external Activity is STILL ALIVE: its grant has not been
            // automatically revoked by Activity finish or instrumentation startup.
            byte[] jpeg = (byte[])method(type, "jpeg", Uri.class).invoke(services, uri);
            Bitmap decoded = BitmapFactory.decodeByteArray(jpeg, 0, jpeg.length);
            check(decoded != null && decoded.getWidth() == 8 && decoded.getHeight() == 6, "Production JPEG conversion lost external payload");
            decoded.recycle();
            method(type, "cleanupCamera").invoke(services);
            check(!file.exists() && field(type, "cameraUri").get(services) == null && field(type, "cameraFile").get(services) == null, "Production cleanup left output");
            final ResultReceiver external = control;
            Bundle revoked = reply(receiver -> { Bundle request = new Bundle(); request.putParcelable("reply", receiver); external.send(1, request); });
            control = null;
            result.putString("revoked", revoked.getString("camera"));
            result.putString("camera", "PASS: real distinct UID intent grant, JPEG write/read, sibling/traversal denial, production decode/cleanup, explicit revocation while recipient alive");
        } catch (Throwable error) { failure = error; }
        finally {
            if (services != null && type != null) try { method(type, "cleanupCamera").invoke(services); }
            catch (Throwable error) { if (failure == null) failure = error; else failure.addSuppressed(error); }
            if (control != null) control.send(2, new Bundle());
        }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", android.util.Log.getStackTraceString(failure));
        finish(failure == null ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
}
