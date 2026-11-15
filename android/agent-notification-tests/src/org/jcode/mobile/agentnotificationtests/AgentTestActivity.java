package org.jcode.mobile.agentnotificationtests;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;

/** Isolated dispatch recorder. No Rust, credentials, server connection or network permission. */
public final class AgentTestActivity extends Activity {
    static volatile AgentTestActivity current;
    static volatile Intent received;
    static volatile int deliveries;
    @Override public void onCreate(Bundle state) { super.onCreate(state); record(getIntent()); }
    @Override public void onNewIntent(Intent intent) { super.onNewIntent(intent); setIntent(intent); record(intent); }
    private void record(Intent intent) { current = this; received = new Intent(intent); deliveries++; }
}
