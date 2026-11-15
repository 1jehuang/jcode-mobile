package org.jcode.mobile.nativeservicestests;

import android.app.Activity;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.ResultReceiver;

/** TEST APK ONLY. Keep this distinct-UID recipient alive until explicit revocation. */
public final class CameraGrantActivity extends Activity {
    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        ResultReceiver reply = getIntent().getParcelableExtra("reply");
        Bundle checks = getIntent().getBundleExtra("checks");
        if (reply == null || checks == null) { finish(); return; }
        ResultReceiver control = new ResultReceiver(new Handler(Looper.getMainLooper())) {
            @Override protected void onReceiveResult(int command, Bundle args) {
                try {
                    if (command == 1) {
                        checks.putString("phase", "revoked");
                        Bundle result = CameraExternalInstrumentation.verify(CameraGrantActivity.this, checks);
                        ResultReceiver response = args.getParcelable("reply");
                        if (response != null) response.send("PASS".equals(result.getString("result")) ? RESULT_OK : RESULT_CANCELED, result);
                    }
                } finally { finish(); }
            }
        };
        Bundle result = CameraExternalInstrumentation.verify(this, checks);
        result.putParcelable("control", CameraResultReceiver.forIpc(control));
        reply.send("PASS".equals(result.getString("result")) ? RESULT_OK : RESULT_CANCELED, result);
        // Do not finish on success. Otherwise Android's Activity grant teardown
        // would make a broken production revoke implementation appear to pass.
        if (!"PASS".equals(result.getString("result"))) finish();
    }
}
