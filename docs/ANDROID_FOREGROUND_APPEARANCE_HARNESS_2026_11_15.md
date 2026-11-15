# Foreground GPUI appearance harness, November 15, 2026

## Coordinator-only command

Reserve one disposable emulator exclusively. Have the installed production
`org.jcode.mobile/.JcodeActivity` already awake, resumed, on its **signed-out
onboarding**, with the keyboard hidden and no auth operation underway. Preparation
is outside the measured foreground-only window. Do not clear account/app data to
satisfy this precondition. The runner never installs, launches, force-stops, taps,
wakes, backgrounds, or resumes the app.

```sh
python3 scripts/android-foreground-appearance-acceptance.py \
  --run emulator-5554 \
  --out target/android/foreground-theme-api35-20261115
```

Use a new output directory for every attempt. No device operation happens without
an explicit `emulator-NUMBER` serial. The runner additionally verifies
`ro.kernel.qemu=1`, primary user 0 and awake/keyboard-hidden state. It requires
host `python3`, SDK `adb`, ImageMagick `magick` and `tesseract`. SDK path defaults to
`$ANDROID_HOME`, then `$ANDROID_SDK_ROOT`, then `~/Android/Sdk`.

Pure tests, no device/subprocess access:

```sh
python3 scripts/android-foreground-appearance-acceptance-test.py -v
```

Author validation: **23 pure tests passed**, Python compilation passed. The author
has not run adb or emulators. These are harness checks, not a runtime theme PASS.
Earlier isolated platform workflows were separately run by the coordinator on
API35/API36. They do not establish this foreground GPUI result.

## Actual evidence required for a PASS

1. Read `cmd uimode night`, selected `dumpsys uimode` service fields and only
   night-related secure-settings keys. Only explicit `yes` or `no`, unlocked and
   without forced override flags, are supported. Auto, scheduled/bedtime/custom,
   locked, ambiguous or unknown encodings fail closed **before mutation**. The
   raw snapshot distinguishes missing settings from present numeric values.
2. Identify the exact top-resumed `JcodeActivity` ActivityRecord token, task,
   ProcessRecord token/PID and launch count. Reject historical entries, other
   components, multiple top-resumed displays or non-resumed/finishing state.
   Hash the actual installed APK bytes with device `sha256sum`, not a guessed
   workspace APK or revision.
3. Require an actual retained target-PID `wm_on_resume_called` or
   `am_on_resume_called` event before proceeding. This establishes that the OS
   lifecycle event path is observable. Keep a unique eight-line events-buffer
   tail as a continuity anchor. A missing/cleared/rolled-over/ambiguous anchor,
   unparseable event or `chatty` suppression in the measured delta fails closed.
   The harness never clears any log buffer or changes log levels.
4. Capture the baseline screen. Actual OCR must contain both **Sign in to Jcode
   Cloud** and **Continue with Google** and must not show authentication underway.
   This is a visible signed-out precondition, not a secure-account-storage audit
   or proof of app-wide network inactivity. The harness issues no auth, model or
   network request and never enters email/text or presses an action.
5. Classify actual screenshot pixels from the central 60% by 60% rectangle,
   excluding system status/navigation bars. At least 70% of these actual pixels
   must be near the shared GPUI background, `#000000` for dark or `#f6f6f6` for
   light, with at most 5% near the opposing background. The tolerance is eight
   levels per RGB channel. This checks content pixels, not just a system setting,
   a changed screenshot hash, OEM chrome, average brightness, or an expected
   fixture. A black/white blank screen cannot pass the simultaneous OCR gate.
6. Execute exactly two changes using `cmd uimode night`: opposite mode, then
   original mode. Each is queried back, then the default 1.5-second settle is
   followed by actual screenshot/OCR/color verification and before/after Activity
   checks. `--settle-seconds` is bounded to 0.5 through 3 seconds. There is no
   resume/relaunch fallback and no retry that could hide a missing invalidation.
7. Across the complete measured window, require unchanged Activity/process/task
   identity and launch count. Reject every observed target create/start/restart/
   pause/resume/stop/destroy/relaunch/top-resumed-loss/gain or process start/death
   lifecycle event. End-state `RESUMED` alone is not sufficient. This combines
   sampled server state with retained continuous OS event-buffer evidence, not
   an app-provided assertion or synthetic callback.
8. In `finally`, including normal exceptions, Ctrl-C and SIGTERM, restore the
   original command mode and exact captured night-related secure key/value set.
   Delete only a night key newly created by the campaign, never arbitrary
   settings. Continue attempting other key restorations if one fails. Re-query
   selected service state and raw settings, requiring exact equality, and check
   Activity/event continuity again. Restoration failure makes the whole run fail,
   even if all rendered transitions passed. No final PASS is printed until this
   verification succeeds.

Each adb subprocess has a 20-second timeout, image conversion/OCR each 15 seconds,
and the number of commands/settings keys is bounded. No retry or polling loop is
unbounded. Global night mode can redraw **other** emulator apps, which is why
exclusive disposable-emulator ownership is required. It must not run concurrently
with audio, screenshot, accessibility, settings or other runtime acceptance work.

Evidence is local: `original-settings.json`, `restored-settings.json`, original
and restored service dumps, command transcript, PNG/OCR pairs, actual color
fractions/PNG hashes, full Activity dumps, events-buffer snapshots and
`result.json`. The output directory is created mode 0700 and cannot already exist.
No screenshot is uploaded by this harness. Existing input-method/service dumps are
not accepted as substitutes for actual theme pixels.

## Hard limits and remaining gates

* A PASS is **foreground shared GPUI theme only** on the recorded installed APK
  and emulator. `font_scale_accepted`, `reduce_motion_accepted`, `full_parity` and
  `release_accepted` remain false in every result. APK build provenance must be
  joined to the recorded installed digest before making a release-profile claim.
* Lifecycle proof is OS event-log and identity evidence, not an instrumented
  native callback counter. If target lifecycle logging or buffer continuity is
  unavailable, the harness refuses to claim same-Activity acceptance. There is no
  silent downgrade to PID-only or screenshot-only evidence.
* `fontScale` is not in the current Activity's consumed `configChanges`, so an
  OS-driven font-scale change can legitimately recreate the Activity. Requiring
  no recreation would be a different contract, not evidence that scaling is
  broken. A separate test should measure actual OCR glyph geometry/layout across
  that supported lifecycle, restore the exact original scale and retained draft,
  and distinguish that from foreground theme invalidation. This harness does not
  mutate font scale.
* Reduced motion needs genuine application-animation evidence, not a readback of
  `animator_duration_scale`, disappearance of system transitions, or an unchanged
  screenshot hash. The signed-out donut uses the shared Reduce Motion setting,
  so a future bounded visual campaign could locate its actual rendered bounds,
  establish baseline movement, demonstrate frozen orientation after a live
  setting change, then restore and demonstrate renewed movement while proving
  content/lifecycle liveness. That sensitivity/recovery control is not yet
  implemented here. This harness does not change any animator setting or claim
  Reduce Motion observation/observer teardown acceptance.
* An app-wide no-network guarantee requires independently arranged isolation or
  additional instrumentation. Visible signed-out onboarding plus no harness
  networking is deliberately not represented as such a guarantee.
* SIGKILL, host death, adb disconnection or device shutdown can prevent cleanup.
  Use the durable original snapshot and command log for coordinator-owned recovery
  and do not claim a pass. Never repair an acceptance failure by resetting app
  data, settings databases, notification state or the whole emulator.
