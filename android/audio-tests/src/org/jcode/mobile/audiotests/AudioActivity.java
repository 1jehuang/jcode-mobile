package org.jcode.mobile.audiotests;

import android.app.Activity;
import android.os.Bundle;
import android.view.WindowManager;
import android.widget.TextView;

/** Real visible foreground owner, with no app/native/network startup. */
public final class AudioActivity extends Activity {
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        TextView label = new TextView(this);
        label.setText("Bounded local microphone acceptance. No audio is saved or sent.");
        setContentView(label);
    }
}
