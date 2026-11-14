package org.jcode.mobile;

import android.Manifest;
import android.app.*;
import android.content.*;
import android.content.pm.*;
import android.content.res.Configuration;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.graphics.*;
import android.media.*;
import android.net.Uri;
import android.os.*;
import android.provider.*;
import android.view.HapticFeedbackConstants;
import org.json.*;
import java.io.*;
import java.util.*;
import java.util.concurrent.*;
import java.security.KeyStore;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.AtomicFile;
import android.widget.ScrollView;
import android.widget.TextView;

/** Platform services only. All application widgets and rendering remain in GPUI. */
public final class AndroidServices {
    private final Activity activity;
    private static final int MEDIA = 4301, MIC = 4302, NOTIFICATIONS = 4303;
    private static final int MAX_BYTES = 12 * 1024 * 1024;
    private volatile boolean picking;
    private int photoLimit;
    private Uri cameraUri;
    private File cameraFile;
    private AudioRecord recorder;
    private volatile boolean foreground;
    private static final Object ACCOUNT_LOCK = new Object();
    private static final String ACCOUNT_KEY = "jcode.accounts.aes-gcm.v1";
    private static final int MAX_ACCOUNT = 1024 * 1024;
    public AndroidServices(Activity activity) { this.activity = activity; nativeInit(); }
    private native void nativeInit();
    private native void nativeMedia(int status, byte[] data);
    private native void nativeResume(String url);
    private native void nativeReply(String session, String text);

    private boolean ui(Callable<Boolean> action) {
        if (activity.isFinishing() || activity.isDestroyed()) return false;
        if (Looper.myLooper() == Looper.getMainLooper()) {
            try { return action.call(); } catch (Exception e) { android.util.Log.w("jcode", "Platform service", e); return false; }
        }
        FutureTask<Boolean> task = new FutureTask<>(action);
        activity.runOnUiThread(task);
        try { return task.get(5, TimeUnit.SECONDS); }
        catch (Exception e) { task.cancel(false); android.util.Log.w("jcode", "Platform service", e); return false; }
    }
    public void onResume(Intent intent) {
        foreground = true;
        Bundle reply = intent == null ? null : RemoteInput.getResultsFromIntent(intent);
        if (reply != null && reply.getCharSequence("reply") != null && intent.getData() != null) {
            nativeReply(intent.getData().getLastPathSegment(), reply.getCharSequence("reply").toString());
            nativeResume(null);
            intent.setClipData(null);
        } else nativeResume(intent != null && intent.getData() != null ? intent.getDataString() : null);
        // Consume launch/new Intents exactly once, not every foreground transition.
        if (intent != null) intent.setData(null);
    }
    public void onPause() { foreground = false; audioStop(); }
    public void onDestroy() { audioStop(); cleanupCamera(); }
    public boolean shareText(String text) {
        return ui(() -> {
            Intent send = new Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text);
            activity.startActivity(Intent.createChooser(send, "Share")); return true;
        });
    }
    public boolean selectText(String text) {
        return ui(() -> {
            TextView view = new TextView(activity);
            view.setText(text);
            view.setTextSize(16);
            view.setTextIsSelectable(true);
            view.setTypeface(android.graphics.Typeface.MONOSPACE);
            int padding = (int)(20 * activity.getResources().getDisplayMetrics().density);
            view.setPadding(padding, padding, padding, padding);
            ScrollView scroll = new ScrollView(activity);
            scroll.addView(view);
            new AlertDialog.Builder(activity).setTitle("Select text").setView(scroll).setPositiveButton("Done", (dialog, which) -> dialog.dismiss()).show();
            return true;
        });
    }

    private AtomicFile accountFile() throws IOException {
        File directory = activity.getNoBackupFilesDir();
        if (directory == null || !directory.isDirectory()) throw new IOException("Private encrypted account storage is unavailable");
        return new AtomicFile(new File(directory, "accounts.aesgcm"));
    }
    private SecretKey accountKey(boolean create) throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        if (store.containsAlias(ACCOUNT_KEY)) return ((KeyStore.SecretKeyEntry)store.getEntry(ACCOUNT_KEY, null)).getSecretKey();
        if (!create) throw new java.security.KeyStoreException("Account encryption key is missing");
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder(ACCOUNT_KEY, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setRandomizedEncryptionRequired(true).setKeySize(256).build());
        return generator.generateKey();
    }
    private byte[] accountAad() { return (activity.getPackageName() + ":accounts:v1").getBytes(java.nio.charset.StandardCharsets.UTF_8); }
    public byte[] accountRead() throws Exception {
        synchronized (ACCOUNT_LOCK) {
            AtomicFile file = accountFile();
            if (!file.getBaseFile().exists() && !new File(file.getBaseFile().getPath() + ".bak").exists()) return null;
            byte[] blob;
            try (FileInputStream in = file.openRead(); ByteArrayOutputStream out = new ByteArrayOutputStream()) {
                byte[] buffer = new byte[4096];
                for (int n; (n = in.read(buffer)) != -1;) {
                    if (out.size() + n > MAX_ACCOUNT + 64) throw new IOException("Encrypted account data exceeds limit");
                    out.write(buffer, 0, n);
                }
                blob = out.toByteArray();
            }
            if (blob.length < 29 || blob[0] != 1) throw new IOException("Invalid encrypted account format");
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.DECRYPT_MODE, accountKey(false), new GCMParameterSpec(128, Arrays.copyOfRange(blob, 1, 13)));
            cipher.updateAAD(accountAad());
            return cipher.doFinal(blob, 13, blob.length - 13);
        }
    }
    public void accountWrite(byte[] plain) throws Exception {
        if (plain == null || plain.length == 0 || plain.length > MAX_ACCOUNT) throw new IOException("Invalid account data size");
        synchronized (ACCOUNT_LOCK) {
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.ENCRYPT_MODE, accountKey(true));
            cipher.updateAAD(accountAad());
            byte[] encrypted = cipher.doFinal(plain), iv = cipher.getIV();
            if (iv.length != 12) throw new java.security.GeneralSecurityException("Unsupported GCM nonce length");
            AtomicFile file = accountFile();
            FileOutputStream out = null;
            try {
                out = file.startWrite();
                out.write(1); out.write(iv); out.write(encrypted);
                out.getFD().sync();
                file.finishWrite(out);
                out = null;
                if (!Arrays.equals(accountRead(), plain)) throw new IOException("Encrypted account write verification failed");
            } catch (Exception error) { if (out != null) file.failWrite(out); throw error; }
        }
    }
    public void accountClear() throws Exception {
        synchronized (ACCOUNT_LOCK) {
            KeyStore store = KeyStore.getInstance("AndroidKeyStore"); store.load(null);
            if (store.containsAlias(ACCOUNT_KEY)) store.deleteEntry(ACCOUNT_KEY);
            if (store.containsAlias(ACCOUNT_KEY)) throw new java.security.KeyStoreException("Cannot remove account encryption key");
            AtomicFile file = accountFile(); file.delete();
            if (file.getBaseFile().exists() || new File(file.getBaseFile().getPath() + ".bak").exists()) throw new IOException("Cannot remove encrypted account data");
        }
    }
    public boolean openUrl(String url) {
        return ui(() -> {
            Uri uri = Uri.parse(url);
            String scheme = uri.getScheme();
            if (!"https".equalsIgnoreCase(scheme) && !"http".equalsIgnoreCase(scheme)) return false;
            activity.startActivity(new Intent(Intent.ACTION_VIEW, uri)); return true;
        });
    }
    public void haptic(int kind) {
        activity.runOnUiThread(() -> {
            int value;
            if (Build.VERSION.SDK_INT >= 30 && kind >= 4) value = kind == 4 ? HapticFeedbackConstants.CONFIRM : HapticFeedbackConstants.REJECT;
            else value = kind == 3 ? HapticFeedbackConstants.CLOCK_TICK : kind <= 1 ? HapticFeedbackConstants.KEYBOARD_TAP : HapticFeedbackConstants.LONG_PRESS;
            activity.getWindow().getDecorView().performHapticFeedback(value);
        });
    }
    public float fontScale() { return activity.getResources().getConfiguration().fontScale; }
    public boolean isLight() { return (activity.getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK) != Configuration.UI_MODE_NIGHT_YES; }
    public boolean reduceMotion() { return Settings.Global.getFloat(activity.getContentResolver(), Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f; }

    public boolean pick(boolean camera, int limit) {
        return ui(() -> {
            if (picking) return false;
            Intent intent;
            if (camera) {
                intent = new Intent(MediaStore.ACTION_IMAGE_CAPTURE);
                if (intent.resolveActivity(activity.getPackageManager()) == null) return false;
                if (Build.VERSION.SDK_INT >= 29) {
                    ContentValues values = new ContentValues();
                    values.put(MediaStore.Images.Media.DISPLAY_NAME, "jcode-camera-" + System.currentTimeMillis() + ".jpg");
                    values.put(MediaStore.Images.Media.MIME_TYPE, "image/jpeg");
                    values.put(MediaStore.Images.Media.IS_PENDING, 1);
                    cameraUri = activity.getContentResolver().insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values);
                } else {
                    File cache = activity.getCacheDir();
                    if (cache == null) return false;
                    File directory = new File(cache, "camera");
                    if (!directory.isDirectory() && !directory.mkdirs()) return false;
                    cameraFile = File.createTempFile("camera-", ".jpg", directory);
                    cameraUri = new Uri.Builder().scheme("content").authority(activity.getPackageName() + ".attachments").appendPath(cameraFile.getName()).build();
                }
                if (cameraUri == null) return false;
                intent.putExtra(MediaStore.EXTRA_OUTPUT, cameraUri);
                intent.setClipData(ClipData.newRawUri("camera", cameraUri));
                intent.addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION | Intent.FLAG_GRANT_READ_URI_PERMISSION);
            } else {
                intent = new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("image/*").addCategory(Intent.CATEGORY_OPENABLE);
                intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, limit > 1);
            }
            photoLimit = Math.max(1, Math.min(limit, 20));
            picking = true;
            try { activity.startActivityForResult(intent, MEDIA); return true; }
            catch (RuntimeException e) { picking = false; cleanupCamera(); return false; }
        });
    }
    private void cleanupCamera() {
        Uri uri = cameraUri; cameraUri = null;
        File file = cameraFile; cameraFile = null;
        if (file != null) { file.delete(); }
        else if (uri != null) try { activity.getContentResolver().delete(uri, null, null); } catch (RuntimeException ignored) {}
    }
    public boolean onActivityResult(int request, int result, Intent data) {
        if (request != MEDIA) return false;
        if (result != Activity.RESULT_OK) { picking = false; cleanupCamera(); nativeMedia(2, null); return true; }
        ArrayList<Uri> uris = new ArrayList<>();
        if (cameraUri != null) uris.add(cameraUri);
        else if (data != null && data.getClipData() != null) {
            ClipData clip = data.getClipData();
            for (int i = 0; i < Math.min(photoLimit, clip.getItemCount()); i++) uris.add(clip.getItemAt(i).getUri());
        } else if (data != null && data.getData() != null) uris.add(data.getData());
        new Thread(() -> {
            boolean failed = uris.isEmpty();
            for (Uri uri : uris) {
                try { nativeMedia(0, jpeg(uri)); }
                catch (Exception | OutOfMemoryError e) { failed = true; android.util.Log.w("jcode", "Cannot decode photo", e); }
            }
            cleanupCamera(); picking = false;
            nativeMedia(failed ? 3 : 1, null);
        }, "jcode-photo").start();
        return true;
    }
    private byte[] jpeg(Uri uri) throws IOException {
        Bitmap bitmap;
        if (Build.VERSION.SDK_INT >= 28) {
            bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(activity.getContentResolver(), uri), (decoder, info, source) -> {
                int edge = Math.max(info.getSize().getWidth(), info.getSize().getHeight());
                if (edge > 2048) decoder.setTargetSampleSize((edge + 2047) / 2048);
                decoder.setAllocator(ImageDecoder.ALLOCATOR_SOFTWARE);
            });
        } else {
            BitmapFactory.Options options = new BitmapFactory.Options(); options.inJustDecodeBounds = true;
            try (InputStream in = activity.getContentResolver().openInputStream(uri)) { BitmapFactory.decodeStream(in, null, options); }
            if (options.outWidth <= 0 || options.outHeight <= 0) throw new IOException("Unreadable image");
            options.inSampleSize = Math.max(1, (Math.max(options.outWidth, options.outHeight) + 2047) / 2048);
            options.inJustDecodeBounds = false;
            try (InputStream in = activity.getContentResolver().openInputStream(uri)) { bitmap = BitmapFactory.decodeStream(in, null, options); }
            if (bitmap != null) {
                int orientation = 1;
                try (InputStream in = activity.getContentResolver().openInputStream(uri)) {
                    orientation = new android.media.ExifInterface(in).getAttributeInt(android.media.ExifInterface.TAG_ORIENTATION, 1);
                } catch (IOException ignored) { /* Legacy ExifInterface does not read PNG/WebP. */ }
                Matrix transform = new Matrix();
                switch (orientation) {
                    case 2: transform.setScale(-1, 1); break;
                    case 3: transform.setRotate(180); break;
                    case 4: transform.setScale(1, -1); break;
                    case 5: transform.setRotate(90); transform.postScale(-1, 1); break;
                    case 6: transform.setRotate(90); break;
                    case 7: transform.setRotate(-90); transform.postScale(-1, 1); break;
                    case 8: transform.setRotate(-90); break;
                    default: break;
                }
                if (!transform.isIdentity()) {
                    Bitmap upright = Bitmap.createBitmap(bitmap, 0, 0, bitmap.getWidth(), bitmap.getHeight(), transform, true);
                    if (upright != bitmap) bitmap.recycle();
                    bitmap = upright;
                }
            }
        }
        if (bitmap == null) throw new IOException("Unreadable image");
        int edge = Math.max(bitmap.getWidth(), bitmap.getHeight());
        if (edge > 2048) {
            Bitmap scaled = Bitmap.createScaledBitmap(bitmap, Math.max(1, bitmap.getWidth() * 2048 / edge), Math.max(1, bitmap.getHeight() * 2048 / edge), true);
            if (scaled != bitmap) bitmap.recycle();
            bitmap = scaled;
        }
        try {
            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
            if (!bitmap.compress(Bitmap.CompressFormat.JPEG, 88, bytes) || bytes.size() == 0 || bytes.size() > MAX_BYTES) throw new IOException("Image exceeds attachment limit");
            return bytes.toByteArray();
        } finally { bitmap.recycle(); }
    }

    public int audioStart() {
        if (activity.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            android.content.SharedPreferences prefs = activity.getSharedPreferences("permissions", Context.MODE_PRIVATE);
            if (prefs.getBoolean("microphone-requested", false) && !activity.shouldShowRequestPermissionRationale(Manifest.permission.RECORD_AUDIO)) return -2;
            boolean requested = ui(() -> {
                prefs.edit().putBoolean("microphone-requested", true).apply();
                activity.requestPermissions(new String[]{Manifest.permission.RECORD_AUDIO}, MIC); return true;
            });
            return requested ? 1 : -1;
        }
        synchronized (this) {
            if (recorder != null || !foreground) return -1;
            try {
                int min = AudioRecord.getMinBufferSize(16000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT);
                if (min <= 0) return -1;
                recorder = new AudioRecord(MediaRecorder.AudioSource.VOICE_RECOGNITION, 16000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, Math.max(min, 6400));
                if (recorder.getState() != AudioRecord.STATE_INITIALIZED) { audioStop(); return -1; }
                recorder.startRecording();
                if (recorder.getRecordingState() != AudioRecord.RECORDSTATE_RECORDING) { audioStop(); return -1; }
                return 0;
            } catch (RuntimeException e) { audioStop(); return -1; }
        }
    }
    public synchronized byte[] audioRead() throws IOException {
        if (recorder == null) throw new IOException("Microphone recording interrupted");
        byte[] bytes = new byte[1600];
        int n = recorder.read(bytes, 0, bytes.length, AudioRecord.READ_NON_BLOCKING);
        if (n < 0) throw new IOException("Microphone read failed: " + n);
        return Arrays.copyOf(bytes, n);
    }
    public synchronized void audioStop() {
        AudioRecord old = recorder; recorder = null;
        if (old != null) { try { old.stop(); } catch (RuntimeException ignored) {} old.release(); }
    }

    public boolean notificationPermission(String unused) {
        return ui(() -> {
            if (Build.VERSION.SDK_INT >= 33 && activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                activity.requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, NOTIFICATIONS); return false;
            }
            return ((NotificationManager)activity.getSystemService(Context.NOTIFICATION_SERVICE)).areNotificationsEnabled();
        });
    }
    public boolean clearNotifications(String unused) {
        NotificationManager manager = (NotificationManager)activity.getSystemService(Context.NOTIFICATION_SERVICE);
        for (android.service.notification.StatusBarNotification notification : manager.getActiveNotifications()) {
            if ("turns".equals(notification.getTag())) manager.cancel("turns", notification.getId());
        }
        return true;
    }
    public boolean syncLive(String headline) {
        return ui(() -> {
            NotificationManager manager = (NotificationManager)activity.getSystemService(Context.NOTIFICATION_SERVICE);
            if (headline.isEmpty()) { manager.cancel("agents", 1); return true; }
            if (!manager.areNotificationsEnabled()) return false;
            NotificationChannel channel = new NotificationChannel("jcode-agents", "Active agents", NotificationManager.IMPORTANCE_LOW);
            channel.setShowBadge(false);
            manager.createNotificationChannel(channel);
            Intent open = new Intent(activity, activity.getClass()).setAction(Intent.ACTION_MAIN).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP);
            PendingIntent pending = PendingIntent.getActivity(activity, 0, open, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
            Notification notification = new Notification.Builder(activity, "jcode-agents").setSmallIcon(android.R.drawable.stat_notify_sync).setContentTitle("jcode agents").setContentText(headline).setStyle(new Notification.BigTextStyle().bigText(headline)).setOngoing(true).setOnlyAlertOnce(true).setContentIntent(pending).build();
            manager.notify("agents", 1, notification); return true;
        });
    }
    public boolean postNotification(String json) {
        if (foreground) return false;
        return ui(() -> {
            NotificationManager manager = (NotificationManager)activity.getSystemService(Context.NOTIFICATION_SERVICE);
            if (!manager.areNotificationsEnabled()) return false;
            JSONObject info = new JSONObject(json);
            String session = info.optString("session_id", "");
            Intent open = new Intent(activity, activity.getClass()).setAction(Intent.ACTION_VIEW).setData(Uri.parse("jcode://session/" + Uri.encode(session))).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
            PendingIntent pending = PendingIntent.getActivity(activity, session.hashCode(), open, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
            manager.createNotificationChannel(new NotificationChannel("jcode-turns", "Session updates", NotificationManager.IMPORTANCE_DEFAULT));
            Intent reply = new Intent(open).setAction("sh.jcode.REPLY");
            int mutable = Build.VERSION.SDK_INT >= 31 ? PendingIntent.FLAG_MUTABLE : 0;
            PendingIntent replyPending = PendingIntent.getActivity(activity, session.hashCode(), reply, PendingIntent.FLAG_UPDATE_CURRENT | mutable);
            Notification.Action action = new Notification.Action.Builder(android.R.drawable.ic_menu_send, "Reply", replyPending).addRemoteInput(new RemoteInput.Builder("reply").setLabel("Message jcode").build()).build();
            Notification notification = new Notification.Builder(activity, "jcode-turns").setSmallIcon(android.R.drawable.stat_notify_chat).setContentTitle(info.optString("title", "jcode")).setContentText(info.optString("body")).setStyle(new Notification.BigTextStyle().bigText(info.optString("body"))).setContentIntent(pending).addAction(action).setAutoCancel(true).build();
            manager.notify("turns", session.hashCode(), notification); return true;
        });
    }
    public boolean indexSessions(String json) {
        return ui(() -> {
            ShortcutManager manager = activity.getSystemService(ShortcutManager.class);
            if (manager == null || manager.isRateLimitingActive()) return false;
            JSONArray rows = new JSONArray(json);
            ArrayList<ShortcutInfo> shortcuts = new ArrayList<>();
            int count = Math.min(rows.length(), manager.getMaxShortcutCountPerActivity());
            for (int i = 0; i < count; i++) {
                JSONArray row = rows.getJSONArray(i);
                String id = row.getString(0), title = row.getString(1);
                if (id.isEmpty() || title.trim().isEmpty()) continue;
                Intent intent = new Intent(activity, activity.getClass()).setAction(Intent.ACTION_VIEW).setData(Uri.parse("jcode://session/" + Uri.encode(id)));
                shortcuts.add(new ShortcutInfo.Builder(activity, "session-" + id).setShortLabel(title).setIntent(intent).build());
            }
            return manager.setDynamicShortcuts(shortcuts);
        });
    }

    /** Only private cache camera files, shared by an explicit temporary URI grant. */
    public static final class AttachmentProvider extends ContentProvider {
        public boolean onCreate() { return true; }
        private File file(Uri uri) throws FileNotFoundException {
            String name = uri.getLastPathSegment();
            if (uri.getPathSegments().size() != 1 || name == null || !name.matches("camera-[0-9]+\\.jpg")) throw new FileNotFoundException("Invalid camera URI");
            File cache = getContext().getCacheDir();
            if (cache == null) throw new FileNotFoundException("Camera cache unavailable");
            File directory = new File(cache, "camera");
            File result = new File(directory, name);
            try {
                if (!result.getCanonicalFile().getParentFile().equals(directory.getCanonicalFile())) throw new FileNotFoundException("Invalid camera path");
            } catch (IOException e) { throw new FileNotFoundException(e.getMessage()); }
            return result;
        }
        public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
            int flags = "r".equals(mode) ? ParcelFileDescriptor.MODE_READ_ONLY :
                ("w".equals(mode) || "wt".equals(mode)) ? ParcelFileDescriptor.MODE_WRITE_ONLY | ParcelFileDescriptor.MODE_TRUNCATE :
                "rw".equals(mode) ? ParcelFileDescriptor.MODE_READ_WRITE : -1;
            if (flags == -1) throw new FileNotFoundException("Unsupported camera access mode");
            return ParcelFileDescriptor.open(file(uri), flags);
        }
        public String getType(Uri uri) { return "image/jpeg"; }
        public Cursor query(Uri uri, String[] projection, String selection, String[] args, String sort) {
            try {
                File file = file(uri);
                String[] columns = projection == null ? new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE} : projection;
                MatrixCursor cursor = new MatrixCursor(columns, 1);
                Object[] values = new Object[columns.length];
                for (int i = 0; i < columns.length; i++) values[i] = OpenableColumns.DISPLAY_NAME.equals(columns[i]) ? file.getName() : OpenableColumns.SIZE.equals(columns[i]) ? file.length() : null;
                cursor.addRow(values); return cursor;
            } catch (FileNotFoundException e) { return null; }
        }
        public Uri insert(Uri uri, ContentValues values) { throw new UnsupportedOperationException(); }
        public int update(Uri uri, ContentValues values, String selection, String[] args) { throw new UnsupportedOperationException(); }
        public int delete(Uri uri, String selection, String[] args) { throw new UnsupportedOperationException(); }
    }
}
