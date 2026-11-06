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

## What a passing run means

The suite exercises Rust widget handlers, raw synthetic touch/event routing,
selection and editing state, UTF-8/UTF-16 conversion, IME composition, Unicode
and grapheme edits, multiline drafts, and modeled mobile keyboard call sequences.
Assertions compare independent simple models where practical and check state and
boundary invariants. Finite, deterministic campaigns can find regressions, not
prove the absence of bugs or cover every gesture/action interleaving.

This is **not actual iOS acceptance testing**. UIKit text interaction, OS keyboard
and autocorrect behavior, real IME integration, haptic hardware, accessibility
services, device timing, GPU rendering, and screenshot appearance still require
platform/device testing. A headless layout or bounds assertion is not a visual
screenshot comparison. Keep targeted regressions for minimized failures, and use
`touch-preview.sh` or real iOS testing for the platform and visual gaps.
