package org.jcode.mobile;

/** The Rust library owns networking and returns a complete JSON state snapshot. */
public final class NativeBridge {
    private static final String LOAD_ERROR;
    static {
        String error = null;
        try { System.loadLibrary("jcode_mobile_android"); }
        catch (UnsatisfiedLinkError | SecurityException e) {
            error = e.getMessage() == null ? e.getClass().getSimpleName() : e.getMessage();
        }
        LOAD_ERROR = error;
    }
    private NativeBridge() {}
    public static boolean isAvailable() { return LOAD_ERROR == null; }
    public static String loadError() { return LOAD_ERROR; }
    public static native String request(String json);
}
