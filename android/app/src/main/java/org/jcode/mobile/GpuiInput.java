package org.jcode.mobile;

import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.text.Editable;
import android.text.Selection;
import android.text.InputType;
import android.text.SpannableStringBuilder;
import android.view.KeyEvent;
import android.view.View;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

/** Non-drawing Android IME endpoint. GPUI owns all application text and painting.
 * The Editable below is only Android input-method composition bookkeeping.
 */
final class GpuiInput extends View {
    private static final int COMMIT = 0, COMPOSE = 1, FINISH = 2, DELETE = 3, ENTER = 4;
    private static native void nativeEdit(int kind, String text, int before, int after);
    // Retain IME bookkeeping across a configuration-driven connection restart.
    // This is not an application model. GPUI continues to own the field value.
    private final Editable editable = new SpannableStringBuilder();
    private int inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE
            | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
    private int imeOptions = EditorInfo.IME_ACTION_UNSPECIFIED | EditorInfo.IME_FLAG_NO_EXTRACT_UI
            | EditorInfo.IME_FLAG_NO_FULLSCREEN;
    private String autofillHint = "";
    private boolean configurationDirty;
    private boolean configurationRefreshPosted;

    GpuiInput(Context context) {
        super(context);
        setWillNotDraw(true);
        setFocusable(true);
        setFocusableInTouchMode(true);
        setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        setContentDescription(null);
        Selection.setSelection(editable, 0);
    }

    void setTextInputConfiguration(int inputType, int imeOptions, String autofillHint) {
        if (autofillHint == null) autofillHint = "";
        if (this.inputType == inputType && this.imeOptions == imeOptions
                && this.autofillHint.equals(autofillHint)) return;
        this.inputType = inputType;
        this.imeOptions = imeOptions;
        this.autofillHint = autofillHint;
        setAutofillHints(autofillHint.isEmpty() ? null : new String[] { autofillHint });
        configurationDirty = true;
        scheduleConfigurationRefresh();
    }

    private boolean isComposing() {
        int start = BaseInputConnection.getComposingSpanStart(editable);
        return start >= 0 && BaseInputConnection.getComposingSpanEnd(editable) > start;
    }

    private void scheduleConfigurationRefresh() {
        if (!configurationDirty || configurationRefreshPosted) return;
        configurationRefreshPosted = true;
        // Never restart inside an InputConnection edit callback or while a
        // composition is active. Finish/commit will retry the pending change.
        post(() -> {
            configurationRefreshPosted = false;
            if (!configurationDirty || !hasFocus() || isComposing()) return;
            InputMethodManager manager = (InputMethodManager)getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
            if (manager != null && manager.isActive(this)) {
                configurationDirty = false;
                manager.restartInput(this);
            }
        });
    }

    void setKeyboardVisible(boolean visible) {
        InputMethodManager manager = (InputMethodManager)getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
        if (manager == null) return;
        if (visible) {
            requestFocus();
            scheduleConfigurationRefresh();
            manager.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT);
        } else {
            manager.hideSoftInputFromWindow(getWindowToken(), 0);
            clearFocus();
        }
    }

    @Override public boolean onCheckIsTextEditor() { return true; }

    @Override public InputConnection onCreateInputConnection(EditorInfo info) {
        info.inputType = inputType;
        info.imeOptions = imeOptions;
        info.initialSelStart = Math.max(0, Selection.getSelectionStart(editable));
        info.initialSelEnd = Math.max(info.initialSelStart, Selection.getSelectionEnd(editable));
        configurationDirty = false;
        return new Connection();
    }

    private boolean key(KeyEvent event) {
        int code = event.getKeyCode();
        if (code == KeyEvent.KEYCODE_DEL || code == KeyEvent.KEYCODE_FORWARD_DEL || code == KeyEvent.KEYCODE_ENTER) {
            if (event.getAction() == KeyEvent.ACTION_DOWN) {
                if (code == KeyEvent.KEYCODE_ENTER) nativeEdit(ENTER, null, 0, 0);
                else nativeEdit(DELETE, null, code == KeyEvent.KEYCODE_DEL ? 1 : 0,
                                code == KeyEvent.KEYCODE_FORWARD_DEL ? 1 : 0);
            }
            return true;
        }
        if (!event.isCtrlPressed() && !event.isAltPressed() && !event.isMetaPressed()) {
            int character = event.getUnicodeChar();
            if (character > 0 && Character.isValidCodePoint(character)) {
                if (event.getAction() == KeyEvent.ACTION_DOWN) {
                    nativeEdit(COMMIT, new String(Character.toChars(character)), 0, 0);
                }
                return true;
            }
        }
        return false;
    }

    @Override public boolean onKeyDown(int code, KeyEvent event) {
        return key(event) || super.onKeyDown(code, event);
    }
    @Override public boolean onKeyUp(int code, KeyEvent event) {
        return key(event) || super.onKeyUp(code, event);
    }

    private final class Connection extends BaseInputConnection {
        Connection() { super(GpuiInput.this, true); }
        @Override public Editable getEditable() { return editable; }

        @Override public boolean commitText(CharSequence text, int position) {
            if (text == null) return false;
            nativeEdit(COMMIT, text.toString(), 0, 0);
            boolean committed = super.commitText(text, position);
            scheduleConfigurationRefresh();
            return committed;
        }
        @Override public boolean setComposingText(CharSequence text, int position) {
            if (text == null) return false;
            nativeEdit(COMPOSE, text.toString(), 0, 0);
            boolean composed = super.setComposingText(text, position);
            scheduleConfigurationRefresh();
            return composed;
        }
        @Override public boolean finishComposingText() {
            nativeEdit(FINISH, null, 0, 0);
            boolean finished = super.finishComposingText();
            scheduleConfigurationRefresh();
            return finished;
        }
        @Override public boolean deleteSurroundingText(int before, int after) {
            if (before < 0 || after < 0) return false;
            nativeEdit(DELETE, null, before, after);
            boolean deleted = super.deleteSurroundingText(before, after);
            scheduleConfigurationRefresh();
            return deleted;
        }
        @Override public boolean deleteSurroundingTextInCodePoints(int before, int after) {
            if (before < 0 || after < 0) return false;
            Editable editable = getEditable();
            int start = Math.max(0, Selection.getSelectionStart(editable));
            int end = Math.max(start, Selection.getSelectionEnd(editable));
            int left = start, right = end;
            for (int n = 0; n < before && left > 0; n++) left -= Character.charCount(Character.codePointBefore(editable, left));
            for (int n = 0; n < after && right < editable.length(); n++) right += Character.charCount(Character.codePointAt(editable, right));
            return deleteSurroundingText(start - left, right - end);
        }
        @Override public boolean performEditorAction(int action) {
            nativeEdit(ENTER, null, 0, 0);
            return true;
        }
        @Override public boolean sendKeyEvent(KeyEvent event) {
            return key(event) || super.sendKeyEvent(event);
        }
        @Override public boolean performContextMenuAction(int id) {
            if (id == android.R.id.paste || id == android.R.id.pasteAsPlainText) {
                ClipboardManager manager = (ClipboardManager)getContext().getSystemService(Context.CLIPBOARD_SERVICE);
                ClipData clip = manager == null ? null : manager.getPrimaryClip();
                if (clip == null || clip.getItemCount() == 0) return false;
                return commitText(clip.getItemAt(0).coerceToText(getContext()), 1);
            }
            return super.performContextMenuAction(id);
        }
        @Override public void closeConnection() {
            nativeEdit(FINISH, null, 0, 0);
            super.closeConnection();
        }
    }
}
