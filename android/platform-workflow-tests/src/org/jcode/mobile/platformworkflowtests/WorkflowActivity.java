package org.jcode.mobile.platformworkflowtests;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;

/** Test-only dispatch endpoint. Never starts Rust, signs in or contacts a server. */
public final class WorkflowActivity extends Activity {
    static volatile WorkflowActivity current;
    static volatile Intent received;
    static volatile int deliveries;
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        record(getIntent());
    }
    @Override public void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        record(intent);
    }
    private void record(Intent intent) {
        current = this;
        received = new Intent(intent);
        deliveries++;
    }
}
