package org.jcode.mobile.nativeservicestests;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.os.Bundle;
import java.io.File;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationTargetException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.security.KeyStore;
import java.util.Arrays;
import dalvik.system.DexClassLoader;

/** Executes installed, unmodified production Java methods under the TEST UID. */
public final class StorageInstrumentation extends Instrumentation {
    private static final String TEST = "org.jcode.mobile.nativeservicestests";
    private static final String TARGET = "org.jcode.mobile";
    private static final String ALIAS = "jcode.accounts.aes-gcm.v1";
    public static final class PrivateActivity extends Activity {
        PrivateActivity(Context context) { attachBaseContext(context); }
    }
    @Override public void onCreate(Bundle args) { super.onCreate(args); start(); }
    static void check(boolean value, String message) {
        if (!value) throw new AssertionError(message);
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        Object services = null;
        Class<?> type = null;
        Throwable failure = null;
        try {
            Context context = getTargetContext();
            check(TEST.equals(context.getPackageName()), "Refusing storage test outside test package");
            int targetUid = context.getPackageManager().getApplicationInfo(TARGET, 0).uid;
            check(android.os.Process.myUid() == context.getApplicationInfo().uid && targetUid != android.os.Process.myUid(),
                    "Refusing storage test: target and test UID must differ");
            File directory = context.getNoBackupFilesDir();
            check(directory.getCanonicalPath().startsWith(context.getDataDir().getCanonicalPath() + "/"), "Storage not test-private");
            KeyStore keys = KeyStore.getInstance("AndroidKeyStore"); keys.load(null);
            File file = new File(directory, "accounts.aesgcm");
            check(!keys.containsAlias(ALIAS) && !file.exists() && !new File(file + ".bak").exists(),
                    "Refusing to overwrite preexisting test storage. Uninstall test APK first.");
            String apk = context.getPackageManager().getApplicationInfo(TARGET, 0).sourceDir;
            DexClassLoader loader = new DexClassLoader(apk, context.getCodeCacheDir().getPath(), null, getClass().getClassLoader());
            type = loader.loadClass("org.jcode.mobile.AndroidServices");
            // Bypass only nativeInit registration. All storage methods come from the installed APK.
            Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
            Field singleton = unsafeClass.getDeclaredField("theUnsafe"); singleton.setAccessible(true);
            Object unsafe = singleton.get(null);
            services = unsafeClass.getMethod("allocateInstance", Class.class).invoke(unsafe, type);
            Field activity = type.getDeclaredField("activity"); activity.setAccessible(true);
            final PrivateActivity[] isolated = new PrivateActivity[1];
            runOnMainSync(() -> isolated[0] = new PrivateActivity(context));
            activity.set(services, isolated[0]);
            check(type.getMethod("accountRead").invoke(services) == null, "Fresh read must be absent");
            byte[] marker = ("native-service-secret-" + java.util.UUID.randomUUID()).getBytes(StandardCharsets.UTF_8);
            type.getMethod("accountWrite", byte[].class).invoke(services, (Object) marker);
            check(keys.containsAlias(ALIAS), "Real Android Keystore key absent");
            check(keys.getKey(ALIAS, null).getEncoded() == null, "Keystore key unexpectedly exportable");
            byte[] first = Files.readAllBytes(file.toPath());
            check(first.length == marker.length + 29 && first[0] == 1, "AES-GCM envelope malformed");
            check(Arrays.equals(marker, (byte[]) type.getMethod("accountRead").invoke(services)), "Decrypt roundtrip mismatch");
            assertNoPlaintext(directory, marker);
            type.getMethod("accountWrite", byte[].class).invoke(services, (Object) marker);
            byte[] second = Files.readAllBytes(file.toPath());
            check(!Arrays.equals(first, second), "GCM nonce/ciphertext reused");
            assertNoPlaintext(directory, marker);
            second[second.length - 1] ^= 1;
            Files.write(file.toPath(), second);
            try {
                type.getMethod("accountRead").invoke(services);
                throw new AssertionError("Corrupted GCM authentication tag accepted");
            } catch (InvocationTargetException expected) {
                check(expected.getCause() instanceof javax.crypto.AEADBadTagException,
                        "Corruption did not fail GCM authentication: " + expected.getCause().getClass().getName());
            }
            type.getMethod("accountClear").invoke(services);
            check(type.getMethod("accountRead").invoke(services) == null && !keys.containsAlias(ALIAS), "Clear left key/data");
            check(!file.exists() && !new File(file + ".bak").exists() && !new File(file + ".new").exists(), "Clear left account files");
            result.putString("storage", "PASS: installed production AES-GCM, non-exportable Keystore key, nonce uniqueness, no plaintext in isolated no-backup files, tag rejection, clear");
        } catch (Throwable error) {
            failure = error;
        } finally {
            if (services != null && type != null) try { type.getMethod("accountClear").invoke(services); }
            catch (Throwable cleanup) { if (failure == null) failure = cleanup; else failure.addSuppressed(cleanup); }
        }
        result.putString("result", failure == null ? "PASS" : "FAIL");
        if (failure != null) result.putString("failure", android.util.Log.getStackTraceString(failure));
        finish(failure == null ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
    }
    private static void assertNoPlaintext(File directory, byte[] marker) throws Exception {
        File[] files = directory.listFiles(); check(files != null, "Cannot inspect private files");
        for (File child : files) {
            if (child.isDirectory()) { assertNoPlaintext(child, marker); continue; }
            byte[] data = Files.readAllBytes(child.toPath());
            outer: for (int i = 0; i <= data.length - marker.length; i++) {
                for (int j = 0; j < marker.length; j++) if (data[i + j] != marker[j]) continue outer;
                throw new AssertionError("Plaintext marker persisted in test-private no-backup file");
            }
        }
    }
}
