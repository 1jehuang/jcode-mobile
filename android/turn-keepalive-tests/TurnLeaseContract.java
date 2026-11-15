package org.jcode.mobile;

public final class TurnLeaseContract {
    private static void check(boolean value) { if (!value) throw new AssertionError(); }
    public static void main(String[] args) {
        TurnLease lease = new TurnLease();
        check(lease.update(false, true, true, 1000) == 0);
        long first = lease.update(true, true, true, 1000);
        check(first > 0 && lease.valid(first, 1000));
        check(lease.remaining(first, 1000) == 150_000);
        for (long now = 1001; now < 151000; now += 123) {
            check(lease.update(true, true, true, now) == 0);
            check(lease.remaining(first, now) == 151000 - now);
        }
        check(!lease.valid(first, 151000));
        check(lease.update(true, true, true, 200000) == 0); // expiry never rearms busy
        check(lease.update(false, false, false, 200000) == 0);
        check(lease.update(true, false, true, 200001) == 0); // background edge consumed
        check(lease.update(true, true, true, 200002) == 0); // foregrounding is not a new turn
        lease.update(false, true, true, 200003);
        check(lease.update(true, true, false, 200004) == 0); // denial consumed
        check(lease.update(true, true, true, 200005) == 0); // no permission retry loop
        lease.update(false, true, true, 200006);
        long second = lease.update(true, true, true, 200007);
        check(second > first && !lease.valid(first, 200008));
        lease.end(first); // stale service destroy must not retire newer lease
        check(lease.valid(second, 200008));
        lease.end(second);
        check(!lease.active(200008));
        check(lease.update(true, true, true, 200009) == 0); // start rejection consumed
        lease.update(false, true, true, 200010);
        long third = lease.update(true, true, true, 200011);
        lease.cancel(); // signout/destroy
        check(!lease.valid(third, 200012));
        check(lease.update(true, true, true, 200013) == 0);
        lease.update(false, false, false, 200014);
        long fourth = lease.update(true, true, true, 200015);
        lease.update(false, false, false, 200016); // idle before queued start
        check(!lease.valid(fourth, 200017));
        for (int i = 0; i < 10000; i++) {
            long now = 1_000_000L + i * 200_000L;
            lease.update(false, false, false, now);
            long token = lease.update(true, true, true, now);
            check(token > fourth && lease.valid(token, now));
            check(!lease.valid(token - 1, now));
            check(!lease.valid(token, now + TurnLease.MAX_MILLIS));
        }
        System.out.println("PASS: production TurnLease bounds, edges, denial, stale tokens, cancellation, 10000 turns");
    }
}
