package org.jcode.mobile;

import android.content.ClipData;
import android.content.Intent;
import java.io.UnsupportedEncodingException;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;

/** Bounded incoming text adapter only. The shared router owns account/UI state. */
public final class IncomingShare {
    public static final int MAX_UTF8_BYTES = 4096;
    private static final String CONSUMED = "org.jcode.mobile.IncomingShare.CONSUMED";
    private static final int URI_GRANTS = Intent.FLAG_GRANT_READ_URI_PERMISSION
            | Intent.FLAG_GRANT_WRITE_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
            | Intent.FLAG_GRANT_PREFIX_URI_PERMISSION;

    private IncomingShare() {}

    /**
     * Consume an ACTION_SEND once on this Intent, including rejected payloads.
     * Returns a draft-only shared route, never an auto-send prompt. Non-SEND
     * intents are untouched. Caller must not fall back to incoming data when a
     * SEND was rejected. Invoke on the UI thread before the usual nativeResume.
     *
     * Requires shared router support for draft=1 before enabling its integration.
     */
    public static String consume(Intent intent) {
        if (intent == null || !Intent.ACTION_SEND.equals(intent.getAction())) return null;
        try {
            // Change the action before reading/unparceling untrusted extras.
            // Neither resume nor Activity recreation can replay this delivery.
            intent.setAction(CONSUMED);
            if (!"text/plain".equals(intent.getType()) || intent.getData() != null
                    || intent.hasExtra(Intent.EXTRA_STREAM) || (intent.getFlags() & URI_GRANTS) != 0) {
                return null;
            }
            CharSequence value = intent.getCharSequenceExtra(Intent.EXTRA_TEXT);
            // Only plain strings, not arbitrary Parcelable/Spanned payloads.
            if (!(value instanceof String)) return null;
            String text = (String)value;
            ClipData clip = intent.getClipData();
            if (clip != null) {
                // A redundant text clip is harmless. Never dereference a URI,
                // embedded Intent, multiple items, HTML, or file attachment.
                if (clip.getItemCount() != 1) return null;
                ClipData.Item item = clip.getItemAt(0);
                if (item.getUri() != null || item.getIntent() != null || item.getHtmlText() != null
                        || !(item.getText() instanceof String) || !text.equals(item.getText())) return null;
            }
            return route(text);
        } catch (RuntimeException malformed) {
            // BadParcelableException, type mismatches and invalid extras fail
            // closed without exposing the private shared text to logs.
            return null;
        } finally {
            // Remove every untrusted payload, not just EXTRA_TEXT. Do not retain
            // subject/HTML/stream extras or URI grants on the Activity Intent.
            intent.replaceExtras((android.os.Bundle)null);
            intent.setClipData(null);
            intent.setDataAndType(null, null);
            intent.setFlags(intent.getFlags() & ~URI_GRANTS);
        }
    }

    /** Pure bounded conversion, exposed package-locally for host-side tests. */
    static String route(String text) {
        if (text == null || text.length() == 0 || text.length() > MAX_UTF8_BYTES) return null;
        boolean nonWhitespace = false;
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (Character.isHighSurrogate(c)) {
                if (++i >= text.length() || !Character.isLowSurrogate(text.charAt(i))) return null;
                nonWhitespace = true;
                continue;
            }
            if (Character.isLowSurrogate(c) || (c < 0x20 && c != '\n' && c != '\t')
                    || (c >= 0x7f && c <= 0x9f) || c == 0x061c || c == 0x200e || c == 0x200f
                    || (c >= 0x202a && c <= 0x202e) || (c >= 0x2066 && c <= 0x2069)
                    || c == 0xfffe || c == 0xffff) return null;
            if (!Character.isWhitespace(c) && !Character.isSpaceChar(c)) nonWhitespace = true;
        }
        if (!nonWhitespace || text.getBytes(StandardCharsets.UTF_8).length > MAX_UTF8_BYTES) return null;
        try {
            // Form encoding is understood by the existing shared URL parser;
            // percent-escape '&', '#', '+', '%' so text cannot inject options.
            return "jcode://new?text=" + URLEncoder.encode(text, "UTF-8") + "&draft=1";
        } catch (UnsupportedEncodingException impossible) {
            throw new AssertionError(impossible);
        }
    }
}
