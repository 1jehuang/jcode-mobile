package org.jcode.mobile;

import android.Manifest;
import android.app.*;
import android.content.*;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.os.*;

/** Bounded process-priority assistance only, not push delivery or a socket/wake guarantee. */
public final class TurnKeepAliveService extends Service {
    static final String CHANNEL = "turn-keepalive";
    static final int NOTIFICATION = 4304;
    private static final TurnLease LEASE = new TurnLease();
    private final Handler handler = new Handler(Looper.getMainLooper());
    private long token;
    private final Runnable expire = this::finishLease;

    static boolean permitted(Context context) {
        try {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager == null || !manager.areNotificationsEnabled()) return false;
        if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED) return false;
        NotificationChannel channel = manager.getNotificationChannel(CHANNEL);
        return channel == null || channel.getImportance() != NotificationManager.IMPORTANCE_NONE;
        } catch (RuntimeException unavailable) { return false; }
    }

    /** Main-thread only. No background-start exemption, retry, alarm or boot path. */
    static boolean update(Context context, boolean busy, boolean foreground) {
        boolean allowed = busy && permitted(context);
        long now = SystemClock.elapsedRealtime();
        long next = LEASE.update(busy, foreground, allowed, now);
        if (!busy) {
            context.stopService(new Intent(context, TurnKeepAliveService.class));
            return true;
        }
        if (next == 0) {
            if (!allowed || !LEASE.active(now)) cancel(context);
            return false;
        }
        try {
            ComponentName started = context.startForegroundService(new Intent(context, TurnKeepAliveService.class)
                    .putExtra("lease", next));
            if (started != null) return true;
        } catch (RuntimeException denied) {
            // No exception messages: they can contain platform/application metadata.
        }
        LEASE.end(next);
        return false;
    }

    static void cancel(Context context) {
        LEASE.cancel();
        context.stopService(new Intent(context, TurnKeepAliveService.class));
    }

    static void checkLease(Context context) {
        if (!LEASE.active(SystemClock.elapsedRealtime()) || !permitted(context)) cancel(context);
    }

    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        long requested = intent == null ? 0 : intent.getLongExtra("lease", 0);
        if (!LEASE.valid(requested, SystemClock.elapsedRealtime()) || !permitted(this)) {
            finishLease();
            return START_NOT_STICKY;
        }
        token = requested;
        try {
            NotificationManager manager = getSystemService(NotificationManager.class);
            NotificationChannel channel = new NotificationChannel(CHANNEL, "Active turn runtime", NotificationManager.IMPORTANCE_LOW);
            channel.setDescription("Briefly keeps an active turn connected after leaving Jcode");
            channel.setShowBadge(false);
            channel.setLockscreenVisibility(Notification.VISIBILITY_PRIVATE);
            manager.createNotificationChannel(channel);
            Notification publicVersion = new Notification.Builder(this, CHANNEL)
                    .setSmallIcon(android.R.drawable.stat_notify_sync).setContentTitle("Jcode")
                    .setContentText("Active turn").build();
            Notification.Builder builder = new Notification.Builder(this, CHANNEL)
                    .setSmallIcon(android.R.drawable.stat_notify_sync).setContentTitle("Jcode turn in progress")
                    .setContentText("Brief background connection, up to 2½ minutes")
                    .setOngoing(true).setOnlyAlertOnce(true).setLocalOnly(true)
                    .setCategory(Notification.CATEGORY_SERVICE).setVisibility(Notification.VISIBILITY_PRIVATE)
                    .setPublicVersion(publicVersion).setShowWhen(false);
            if (Build.VERSION.SDK_INT >= 31) builder.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE);
            Notification notification = builder.build();
            if (Build.VERSION.SDK_INT >= 34) {
                startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE);
            } else {
                startForeground(NOTIFICATION, notification);
            }
            handler.removeCallbacks(expire);
            handler.postDelayed(expire, LEASE.remaining(token, SystemClock.elapsedRealtime()));
        } catch (RuntimeException denied) {
            finishLease();
        }
        return START_NOT_STICKY;
    }

    private void finishLease() {
        LEASE.end(token);
        handler.removeCallbacks(expire);
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
    }
    @Override public void onTimeout(int startId) { finishLease(); }
    @Override public void onTimeout(int startId, int fgsType) { finishLease(); }
    @Override public void onTaskRemoved(Intent rootIntent) { finishLease(); }
    @Override public void onDestroy() {
        LEASE.end(token);
        handler.removeCallbacks(expire);
        stopForeground(STOP_FOREGROUND_REMOVE);
        super.onDestroy();
    }
    @Override public IBinder onBind(Intent intent) { return null; }
}
