# API36 Gboard non-docked accessory toolbar diagnosis

Date: November 15, 2026. Text-only analysis of root-captured evidence. No device operations or image reads were performed for this diagnosis. No production code or acceptance parser changes are proposed.

## Finding and scope

The captured API36 Gboard service owns a visible, non-docked accessory toolbar rather than a full bottom keyboard. Its own content inset consumes the entire InputMethod frame. This explains why `mInputShown=true` and `mIsInputViewShown=true` coexist with zero docked keyboard content. The strict release check correctly rejects that state. These observations do not establish whether a Gboard preference, cached accessory mode, stylus-device classification, or an IME/framework defect triggered the mode.

There is no evidence here that Jcode app insets clipped a full keyboard, or that the draft lost its prefix. Do not fix app layout or weaken visible-keyboard acceptance to accommodate this capture.

## Reproducible evidence

Two separate root captures exhibit the same geometry:

- `target/android/release-runtime-aab-api36-visible-ime/`: `result.json` reports `No visible InputMethod frame found in window dump`, `passed=false`, and `release_accepted=false`. This is the release-derived artifact, not the later debug artifact.
- `target/android/input-3331b05-viewport-diagnostic/`: the later debug viewport diagnosis. `semantic.xml` contains the complete focused native editable value `paritydraft sample` at bounds `[52,970][1028,1086]`. `home.txt` OCR has only the suffix. OCR clipping is not equivalent to native input loss.

In both captures, the InputMethod window in `keyboard-windows.txt` or `windows.txt`, lines 299-319, declares:

```text
mGivenContentInsets=[0,1857][0,0]
frame=[0,63][1080,1920]
```

Thus declared docked content height is `1920 - (63 + 1857) = 0`. The window is drawn and visible. The zero-height result comes from IME-owned content insets, not the app's content frame.

In `ime-shown.txt` or `input-method.txt`, lines 1375-1403, the IME service itself declares:

```text
mIsInputViewShown=true
contentTopInsets=1857 visibleTopInsets=1857 touchableInsets=3
  touchableRegion=SkRegion((21,560,169,1250))
mSettingsObserver=SettingsObserver{mShowImeWithHardKeyboard=1}
```

The window dump's corresponding screen-space toolbar touch region is `[21,623][169,1313]`. It overlaps the left side of the native email field, consistent with the clipped OCR prefix while native text remains complete. Geometry establishes possible occlusion, not a screenshot observation by this worker.

The service's AccessPoints dump, lines 2278-2294, contains `close_accessory_floating_keyboard` and controls including `widget_more_access_points`, `widget_keyboard`, `widget_change_widget_orientation`, and `widget_access_point_stylus_gestures`. These support identifying the non-docked input UI as the accessory toolbar. They do not prove every cached control is currently visible or clickable.

The explicit hard-keyboard setting was already recorded as enabled by the service, so repeating `show_ime_with_hard_keyboard=1` is not a demonstrated solution.

### Trigger remains unproven

The device dump lists eleven stylus-capable virtio multitouch devices and a physical qwerty keyboard. App/client EditorInfo reports `initialToolType=1`, `isStylusHandwritingEnabled=false`, and inputType `0x80021`. The earlier release service EditorInfo reports tool type 2, but the later 3331 service reports tool type 0 while retaining the same toolbar. Therefore a single stylus event is not established as the necessary cause. The app handwriting opt-out does not prove which Gboard-owned accessory preference or cached UI mode is active.

The API35 reference `target/android/release-runtime-aab-api35/ime-shown.txt`, lines 1297-1300, reports positive visible content geometry (`contentTopInsets=985` within a taller frame) rather than this all-frame inset. That is a useful contrast, not proof that API36 itself has a platform regression.

## Safe next root action

Root alone operates the device, using the explicit serial. Prefer the actual Gboard accessory toolbar's menu/overflow and an observed, labeled **Show on-screen keyboard** / keyboard control corresponding to `widget_keyboard`. Inspect the real current UI before selecting. Do not guess coordinates, interpret the IME action key as a mode toggle, or submit/send the draft.

If that control is unavailable, open the already captured Gboard settings activity, without clearing data or replacing the IME:

```sh
adb -s emulator-5556 shell am start -n com.google.android.inputmethod.latin/com.google.android.apps.inputmethod.latin.preference.SettingsActivity
```

Inspect the current Physical keyboard / toolbar or Write in text fields settings. Record the previous value before changing any preference and restore it after the bounded diagnostic run, including failure paths. Menu labels are suggestions to verify against the actual installed UI, not asserted selectors. Do not grant permissions, clear Gboard data, reinstall, or silently change the global default IME.

After explicitly switching to the full keyboard, root should recapture the IME service and window state and rerun the existing strict visible-keyboard check unchanged. Require positive declared keyboard content plus the existing real visible-key evidence. A settings tap, `mInputShown=true`, or the standalone diagnostic below is not acceptance. If mode switching leaves the same geometry, retain failure and investigate the IME service rather than patching app insets speculatively.

## Read-only diagnostic and tests

`android/tests/ime-toolbar-diagnosis.py` is an isolated evidence classifier, not imported by any acceptance script. It emits no draft text, performs no device access, requires agreement between window and service insets, and does not claim that positive geometry alone proves visible keys. Missing or ambiguous evidence remains undetermined.

```sh
python3 -B android/tests/ime-toolbar-diagnosis.py --self-test
python3 -B android/tests/ime-toolbar-diagnosis.py \
  --windows target/android/input-3331b05-viewport-diagnostic/windows.txt \
  --input-method target/android/input-3331b05-viewport-diagnostic/input-method.txt
```

Local validation: four pure test groups pass. Both captured API36 datasets classify as `IME_NON_DOCKED_ACCESSORY_TOOLBAR`, with `docked_content_height=0` and `input_view_shown=true`. No device-side mode-switch success, release acceptance, or rendering fix is claimed by this diagnosis.
