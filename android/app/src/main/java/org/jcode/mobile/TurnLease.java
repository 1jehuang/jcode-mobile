package org.jcode.mobile;

/** Pure monotonic-clock admission policy. Accessed only on Android's main thread. */
final class TurnLease {
    static final long MAX_MILLIS = 150_000;
    private boolean busy;
    private long generation;
    private long deadline;

    // A denied or expired busy edge is consumed, not retried on stream updates/resume.
    long update(boolean nextBusy, boolean foreground, boolean permitted, long now) {
        if (!nextBusy) {
            busy = false;
            deadline = 0;
            return 0;
        }
        if (busy) return 0;
        busy = true;
        if (!foreground || !permitted) return 0;
        generation++;
        deadline = now + MAX_MILLIS;
        return generation;
    }
    boolean valid(long token, long now) {
        return token != 0 && token == generation && deadline != 0 && now < deadline;
    }
    boolean active(long now) { return valid(generation, now); }
    long remaining(long token, long now) { return valid(token, now) ? deadline - now : 0; }
    void end(long token) { if (token == generation) deadline = 0; }
    void cancel() { deadline = 0; }
}
