# Headless mobile interaction fuzzing

`scripts/mobile-fuzz.sh` runs the real `jcode-mobile` binary's Rust tests through
`justrust test`, not the application's `main`. It needs Linux, Bash, justrust,
and the usual project build dependencies. It does not need Wayland, X11, a phone,
a compositor, or a logged-in account. Start with:

```sh
scripts/mobile-fuzz.sh --dry-run
scripts/mobile-fuzz.sh
scripts/mobile-fuzz.sh --release
scripts/mobile-fuzz.sh seeded_unicode_handler_sequences -- --nocapture
scripts/mobile-fuzz.sh --matrix
```

The default is the **entire binary test suite**, including deterministic widget
and model tests, not just tests matching `fuzz`. Cargo/justrust arguments and test
filters go before `--`. Libtest arguments go after it. `--dry-run` can appear
anywhere before `--`, validates GPUI paths, and only prints shell-escaped commands.
It does not require justrust to be installed or invoke a build. Commands stop at
the first failing process and retain its exit code.

## Local GPUI and headless execution

By default the runner uses `${GPUI_DIR:-$HOME/gpui}`. It applies command-line Cargo
patches for the complete local `gpui`/`gpui_*` crate family, crates.io GPUI, and
vendored `cgl`, following `touch-preview.sh` without changing that script. This
keeps GPUI types shared with platform crates and the thinking-orbs dependency.
The checkout must include the listed platform crates and `vendor/cgl`.

```sh
GPUI_DIR=/path/to/gpui scripts/mobile-fuzz.sh --dry-run
GPUI_LOCAL=0 scripts/mobile-fuzz.sh --dry-run
```

`GPUI_LOCAL=0` uses the pinned repository dependencies instead. This is an opt-out,
not a promise that every pinned revision has the same test backend behavior.
The runner sets `ZED_HEADLESS=1` and removes `DISPLAY`/`WAYLAND_DISPLAY` only for
child processes. It does not edit manifests, Cargo configuration, shell profiles,
credentials, or HOME, and does not change the caller's environment. Normal build
caches and build artifacts can still be written by justrust/Cargo.

Tests are forced to `--test-threads=1` with `RUST_TEST_THREADS=1`. Explicit thread
overrides are rejected. Some existing tests mutate process-global accessibility
state, so parallel test execution is not currently safe. Serial execution is a
containment measure, not proof that all global state is restored perfectly.

## Matrix and deterministic replay

The baseline always uses dark appearance, Reduce Motion off, and text scale 1.0.
`--matrix` runs eight independent processes: light/dark × Reduce Motion off/on ×
text scale 1.0/1.6. Each invocation prints its configuration. Settings are supplied
as `JCODE_MOBILE_APPEARANCE`, `JCODE_MOBILE_REDUCE_MOTION`, and
`JCODE_MOBILE_TEXT_SCALE`. Tests that explicitly install their own theme or reset
global metrics may not exercise every matrix value. A matrix pass is therefore
not a visual or accessibility certification.

Phone dimensions are varied by the raw interaction harness itself, rather than
by launching desktop windows at different sizes. The raw harness campaign budget
is configurable with `JCODE_MOBILE_FUZZ_STEPS` and `JCODE_MOBILE_FUZZ_SEEDS`.
The defaults are 48 steps per seed and two seeds, bounded to 1..512 steps and
1..8 seeds. Invalid values use defaults. `JCODE_MOBILE_FUZZ_SEED` sets the
starting action seed, with subsequent campaigns incrementing it:

```sh
JCODE_MOBILE_FUZZ_SEED=42 JCODE_MOBILE_FUZZ_STEPS=128 JCODE_MOBILE_FUZZ_SEEDS=4 \
  scripts/mobile-fuzz.sh --matrix -- --nocapture
```

These controls apply to the raw harness, not every deterministic test. Individual
property-style tests may intentionally use a fixed seed corpus and step budget.
Read a failure's seed, step, action, surface, and dimensions before reducing a
case. Keep those diagnostics with the exact revision and printed matrix settings.

GPUI also has scheduler replay controls `SEED` and `ITERATIONS`, separate from the
input-action RNG. For example:

```sh
SEED=42 ITERATIONS=1 scripts/mobile-fuzz.sh seeded_unicode_handler_sequences -- --nocapture
```

Use the scheduler seed reported by GPUI when replaying a scheduling failure.
Increasing `ITERATIONS` increases scheduler repetitions, not the raw action step
budget. For exact replay retain both RNG configurations. The runner preserves
these inherited controls, and never writes environment variables inside tests.

## Safety boundary

Offline test constructors guard credential loading, network startup, browser
launches, and account persistence. The interaction harness must use those
constructors and synthetic fixtures rather than production login/startup paths.
The runner itself invokes no account APIs, browser, simulator service, or real
application launch. It is not a network namespace or a security sandbox: adding
an unguarded test can violate the offline contract. Keep new tests on the guarded
paths and review any new side effects.

Actual API/platform services are deliberately excluded: authentication providers,
account and session synchronization endpoints, push registration/delivery,
Keychain, Spotlight services, and server-side behavior are not validated by this
campaign. No real credentials or service requests are needed. justrust build
infrastructure is separate from application network traffic. The runner passes
Cargo's `--offline`, so dependencies must already be cached. This does not disable
hosted justrust transport. Do not bypass justrust billing or
isolation. The runner never reconfigures HOME or wipes user state to simulate a
clean account.

## Targeted interaction regressions

The raw-touch suite covers onboarding focus dismissal, safe offline drafts,
overlay recovery, cancelled drawer and tab pans, auxiliary field keyboard
routing, long-press tab menus and rename recovery, cancelled armed pull-to-refresh,
and screen changes while an edge gesture is captured. The seeded campaigns
cycle eight surfaces at 320×568, 390×844, 844×390, and 568×320, with multi-contact
sequences, cancellation, out-of-bounds positions, and mid-contact resizing.

Initial runs reproduced a drawer cancellation that could still idle-commit and
auxiliary text fields that neither opened nor received the simulated keyboard.
Independent Unicode input modeling also exposed composition-relative UTF-16
selection offsets. Fixed regressions additionally cover stale edge captures after
navigation and cancelled tab/pull gestures. A fully settled keyboard-layout
regression then exposed rename actions at y=270.5..310.5 beneath the keyboard
starting at y=232 on a 320×568 phone. The rename card now sizes within its available
width, centers in the keyboard-reduced viewport, and scrolls when necessary.
These checks assert interaction state and recovery, not rendered pixels or the
complete iOS event stack.

### Recorded stress run

On November 5, 2026 (Pacific time), this expanded campaign passed:

```sh
JCODE_MOBILE_FUZZ_SEEDS=8 JCODE_MOBILE_FUZZ_STEPS=256 \
  scripts/mobile-fuzz.sh deterministic_raw_touch_campaigns -- --nocapture
```

Seeds 362108240712 through 362108240719 completed **2,048 cases, 3,056
contacts, and 9,192 raw touch events** across eight surfaces and four dimensions.
The justrust receipt is `20261105-223207311-3528788` (93.81 seconds of tests).
Separately, all eight raw interaction tests passed in each of the eight
appearance/Reduce Motion/text-scale configurations (**64 test executions**):

```sh
scripts/mobile-fuzz.sh --matrix app::interaction_fuzz -- --nocapture
```

This matrix includes the fully settled keyboard rename workflow, not just the
pre-animation layout. Its final process receipt is
`20261105-223927784-3585469`. Two older swipe assertions were also corrected to
expect immediate settling under Reduce Motion, rather than requiring animation.
The complete binary suite also passed
`scripts/mobile-fuzz.sh --matrix -- --nocapture`: 97 tests in each dark configuration and 98 in each light
configuration, **780 passing test executions** in total. A concurrent drawer
footer regression was added between the two appearance halves, accounting for
the count difference. The final process receipt is
`20261105-224115802-3596545`; the eight-process run took 164.82 seconds.

Receipts describe finite test coverage, not an absence-of-bugs guarantee.

## Reviewed surface inventory and stateful journeys

`docs/MOBILE_INTERACTION_COVERAGE.md` is the human checklist and
`docs/MOBILE_INTERACTION_COVERAGE.json` maps discovered renderer/event/control
owners to named regression tests, explicit gaps, or native-device requirements.
The normal suite includes `production_event_inventory_matches_reviewed_coverage_ledger`,
which detects structural drift and stale test references. This is not semantic
coverage measurement: an unchanged callback signature can still change behavior.
Use the checklist notes rather than interpreting a mapped row as exhaustive coverage.

The expanded offline suites cover authentication callback races, account linking,
settings/logout, directory/model pickers, drawer/tab actions, composer send/queue/stop,
transcript controls, message selection/retry/rename, and mocked Git, voice, deep-link,
and push lifecycles. Real provider, transport, permission, and OS behavior remains
explicitly excluded where fixtures inject outcomes. To run individual families:

```sh
scripts/mobile-fuzz.sh account_coverage
scripts/mobile-fuzz.sh navigation_coverage
scripts/mobile-fuzz.sh composer_coverage
scripts/mobile-fuzz.sh overlay_coverage
scripts/mobile-fuzz.sh coverage_inventory
scripts/mobile-fuzz.sh journey_fuzz -- --nocapture
scripts/mobile-fuzz.sh dump_interaction_coverage_inventory -- --ignored --nocapture
```

The stateful journey test runs four fixed seeds through 256 handler actions each,
without resetting application state between actions. Its independent tab/draft/pending
prompt model checks Unicode edits, tab creation/switch/close, queued prompts, local sends,
logout and stale authentication, cancellation and responsive layouts. These are direct
handler/model journeys with rendered frames, not 1,024 additional raw-touch gestures.
The separate raw campaign and focused hit-tested workflows cover event routing.

On November 6, 2026 (Pacific time), an independent raw corpus also passed:

```sh
JCODE_MOBILE_FUZZ_SEED=42 JCODE_MOBILE_FUZZ_SEEDS=8 JCODE_MOBILE_FUZZ_STEPS=256 \
  scripts/mobile-fuzz.sh deterministic_raw_touch_campaigns -- --nocapture
```

Seeds 42 through 49 completed another **2,048 cases, 3,056 contacts, and 9,192
raw touch events**. Receipt: `20261106-012249698-362106`, 108.62 seconds of tests.
This corpus exercises the raw campaign's eight surfaces, not every newly inventoried
service/overlay path. The focused regression suites cover those additional paths.

### Expanded full-suite acceptance

After integrating the surface ledger and fixes, the complete binary suite passed
all eight configurations on November 6, 2026 (Pacific time):

```sh
scripts/mobile-fuzz.sh --matrix -- --nocapture
```

Each process passed **209 tests**, with only the intentionally opt-in inventory
dump ignored. That is **1,672 passing test executions**, with the reviewed
production-inventory guard enabled and no test filters/skips. The run took
336.4 seconds including competing build load. First/final receipts:
`20261106-014044509-494349` and `20261106-014504189-545062`. Final Clippy of
the binary and its tests also passed: `20261106-014128835-499822` (warnings remain).
Validation used local GPUI overrides, not a claim about the unpatched pinned backend.

This pass caught an actual model-sheet click-through at text scale 1.6: a model
row's center was inside both its actual scroll clip and the account pill underneath.
Picking the row closed and immediately reopened the sheet via the underlying control.
The modal now occludes that control. The regression asserts the overlap and successful
selection. A local-session See More test was also corrected to scroll its real clipped
overview before tapping, rather than treating offscreen debug bounds as reachability.

A separate Settings test failure reproduced at scheduler seed 47 was caused by
its seeded entrance animation still moving. Only seeded geometry was settled,
with the real touch/Chat/tab assertions retained. The corrected test passed
`SEED=42 ITERATIONS=128`, seeds 42 through 169, receipt
`20261106-013158332-426759`. This is fixture stabilization, not a production fix
or evidence of native animation timing.

## What a passing run means

The suite exercises Rust widget handlers, raw synthetic touch/event routing,
selection and editing state, UTF-8/UTF-16 conversion, IME composition, Unicode
and grapheme edits, multiline drafts, and modeled mobile keyboard call sequences.
Assertions compare independent simple models where practical and check state and
boundary invariants. Finite, deterministic campaigns can find regressions, not
prove the absence of bugs or cover every gesture/action interleaving.

### Native acceptance still required

Use an isolated test account on an iPhone or supported iOS simulator, never a
production account for destructive actions. Keep screenshots, device/OS version,
configuration, and failure steps with the test revision. The remaining checklist is:

- Complete and cancel each supported email/Apple/browser login flow, including
  switching flows while a response is pending and relaunching after sign-out.
- Type with the actual software keyboard, Unicode IMEs, autocorrection, hardware
  keys and selection handles. Rotate during editing and confirm sheet actions
  remain reachable with the keyboard and large accessibility text.
- Exercise VoiceOver activation/focus and native copy/share sheets. Verify safe
  areas, clipped content, animations and actual haptic feedback visually/on hardware.
- Deny then grant microphone permission, start/release/lock/cancel a recording,
  interrupt/background it, and verify successful transcription cannot overwrite a
  newly typed draft or arrive in another tab. Verify the recording time limit.
- Use an authorized test service to disconnect/reconnect during streaming, send,
  stop, retry, model/directory changes, remote fork and session continuation.
  Verify server acknowledgements, not just local optimistic intent.
- Deliver actual cold/warm deep links and APNs notifications/replies, switch tabs
  during connection, and check exactly-once routing without lost drafts.
- Verify real Git inspection/cloud inventory errors and timeout recovery, native
  badge/Live Activity updates, persistence, and an explicitly authorized disposable
  account deletion. Offline fixtures intentionally do not perform these operations.

These are **pending manual/integration checks**, not recorded passes. The macOS
iOS monkey workflow and scripts can supply additional native random input, but
random taps alone do not establish the semantic outcomes above.

This is **not actual iOS acceptance testing**. UIKit text interaction, OS keyboard
and autocorrect behavior, real IME integration, haptic hardware, accessibility
services, device timing, GPU rendering, and screenshot appearance still require
platform/device testing. A headless layout or bounds assertion is not a visual
screenshot comparison. Keep targeted regressions for minimized failures, and use
`touch-preview.sh` or real iOS testing for the platform and visual gaps.
