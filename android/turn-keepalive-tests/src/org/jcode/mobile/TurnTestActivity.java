package org.jcode.mobile;

import android.app.Activity;
import android.os.Bundle;
import android.widget.TextView;

/** Isolated test UI only, never loaded by the GPUI app. */
public final class TurnTestActivity extends Activity {
    volatile boolean resumed;
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        TextView text = new TextView(this);
        text.setText("Isolated bounded turn service acceptance. No account or network access.");
        setContentView(text);
    }
    @Override public void onResume() { super.onResume(); resumed = true; }
    @Override public void onPause() { resumed = false; super.onPause(); }
    @Override public void onDestroy() { TurnKeepAliveService.cancel(this); super.onDestroy(); }
}
