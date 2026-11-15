package org.jcode.mobile.nativeservicestests;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.pm.ProviderInfo;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import java.io.InputStream;
import java.io.OutputStream;

/** Runs under the isolated test UID, never under the provider owner's UID. */
public final class CameraExternalInstrumentation extends Instrumentation {
    private Bundle args;
    @Override public void onCreate(Bundle args) { super.onCreate(args); this.args = args; start(); }
    private static void check(boolean value, String message) { StorageInstrumentation.check(value, message); }
    private static void denied(Context context, Uri uri, String mode) throws Exception {
        try (android.os.ParcelFileDescriptor descriptor = context.getContentResolver().openFileDescriptor(uri, mode)) {
            throw new AssertionError("Unpermitted camera URI access accepted: " + mode);
        } catch (SecurityException expected) { /* Exact URI grants must not authorize siblings or descendants. */ }
    }
    @Override public void onStart() {
        Bundle result = verify(getTargetContext(), args);
        finish("PASS".equals(result.getString("result")) ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
    static Bundle verify(Context context, Bundle args) {
        Bundle result = new Bundle();
        Throwable failure = null;
        try {
            check("org.jcode.mobile.nativeservicestests".equals(context.getPackageName()), "External test must run in isolated package");
            check(android.os.Process.myUid() == context.getApplicationInfo().uid
                    && android.os.Process.myUid() != context.getPackageManager().getApplicationInfo("org.jcode.mobile", 0).uid,
                    "External camera test accidentally shares provider owner UID");
            ProviderInfo info = context.getPackageManager().resolveContentProvider("org.jcode.mobile.attachments", 0);
            check(info != null && !info.exported && info.grantUriPermissions, "Camera provider is not non-exported/grant-only");
            Uri uri = Uri.parse(args.getString("uri", ""));
            check("org.jcode.mobile.attachments".equals(uri.getAuthority()) && uri.getPathSegments().size() == 1
                    && uri.getLastPathSegment().matches("camera-[0-9]+\\.jpg"), "Refusing unsafe test URI");
            if ("revoked".equals(args.getString("phase"))) {
                denied(context, uri, "r"); denied(context, uri, "w");
                result.putString("camera", "PASS: external UID read and write both denied after production grant revocation");
            } else {
                Bitmap bitmap = Bitmap.createBitmap(8, 6, Bitmap.Config.ARGB_8888);
                bitmap.eraseColor(Color.BLUE);
                try (OutputStream out = context.getContentResolver().openOutputStream(uri, "w")) {
                    check(out != null && bitmap.compress(Bitmap.CompressFormat.JPEG, 88, out), "External UID cannot write valid JPEG to exact granted URI");
                } finally { bitmap.recycle(); }
                try (InputStream in = context.getContentResolver().openInputStream(uri)) {
                    Bitmap decoded = BitmapFactory.decodeStream(in);
                    check(decoded != null && decoded.getWidth() == 8 && decoded.getHeight() == 6, "Exact URI read grant failed after external write");
                    decoded.recycle();
                }
                Uri sibling = new Uri.Builder().scheme("content").authority(uri.getAuthority()).appendPath("camera-0.jpg").build();
                if (sibling.equals(uri)) sibling = sibling.buildUpon().path("camera-1.jpg").build();
                denied(context, sibling, "r"); denied(context, sibling, "w");
                Uri traversal = uri.buildUpon().appendPath("..").appendPath("camera-0.jpg").build();
                denied(context, traversal, "r"); denied(context, traversal, "w");
                result.putString("camera", "PASS: distinct UID wrote and read actual production exact URI; sibling and traversal read/write denied");
            }
        } catch (Throwable error) { failure = error; }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", android.util.Log.getStackTraceString(failure));
        return result;
    }
}
