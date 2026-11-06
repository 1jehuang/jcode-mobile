# Touchscreen preview on Linux

Run from a terminal inside an existing **Wayland** desktop session:

```bash
scripts/touch-preview.sh
# Optional optimized build, or inspect the command without building/launching:
scripts/touch-preview.sh --release
scripts/touch-preview.sh --dry-run
```

The launcher is non-interactive and executes `justrust run --package jcode-mobile
--bin jcode-mobile`. It does not start a compositor, switch sessions, install
packages, or change login settings. You need Linux, `justrust` on PATH, normal
GPUI Linux build dependencies, and a working Wayland compositor. For physical
touch testing the compositor must expose the touchscreen to the application.
`WAYLAND_DISPLAY` must name an existing socket, relative to `XDG_RUNTIME_DIR`
or absolute. The launcher rejects `ZED_HEADLESS` and removes `DISPLAY` only in
its own process, preventing an accidental X11/XWayland fallback. A socket check
is not proof of compositor connectivity or touchscreen support.

## Local GPUI and native touch status

By default `GPUI_LOCAL=1` uses `$HOME/gpui`. Select another checkout with
`GPUI_DIR=/absolute/path/to/gpui`. The command-line Cargo overrides follow the
pattern in `scripts/ios-bundle.sh`, without editing Cargo.toml or persistent
Cargo configuration. They patch the GPUI git source for `gpui`, `gpui_platform`,
`gpui_linux`, `gpui_wgpu`, `gpui_macros`, shared string/util crates, and the other
GPUI platform crates present in that checkout. The crates.io `gpui` override is
also redirected locally so `gpui-thinking-orbs` uses the same GPUI type identity.
Both source overrides for the repository's vendored `cgl` are kept local too.
Normal dependency resolution when you actually run may update Cargo.lock.

```bash
GPUI_DIR=/home/jeremy/gpui scripts/touch-preview.sh
GPUI_LOCAL=0 scripts/touch-preview.sh # pinned revision, not local touch work
```

**Native finger input depends on GPUI's Wayland `wl_touch` implementation.**
The backend initially has no `wl_touch` handling. That support is being developed
separately in `/home/jeremy/gpui`; this launcher alone does not implement it or
prove it works. Use the checkout containing that work, and verify actual taps,
drags, scrolling, cancellation, and long presses on hardware. The pinned revision
is not a substitute for the local backend changes. XWayland pointer events are
not evidence that native Wayland touch works. Multitouch and gesture fidelity
must not be assumed from a successful build or mouse-only test.

The app already enables **mouse-to-touch emulation** on desktop: mouse presses
and drags pass through GPUI's touch recognizer. This remains useful without a
physical touchscreen, but is not hardware-touch validation. It also already has
a clickable simulated phone keyboard. This is not UIKit's keyboard, a compositor
keyboard integration, or an iOS simulator. Safe areas, keyboard insets, platform
services, haptics, and system interaction still need real iPhone testing. The
preview connects to real services and uses real stored accounts, not mock data.

## Existing preview environment options

All app environment settings are inherited unchanged. Defaults remain app-owned:

| Variable | Behavior |
| --- | --- |
| `JCODE_MOBILE_SIZE=375x667` | Phone content size in logical pixels, default `390x844`. Safe areas are selected from height. |
| `JCODE_MOBILE_FRAMELESS=1` | Hide the simulated device frame and its extra padding. |
| `JCODE_MOBILE_MOUSE=1` | Disable mouse-to-touch emulation and restore ordinary mouse input. Leave unset for emulation. |
| `JCODE_MOBILE_KEYBOARD=0` | Disable the simulated keyboard. Unset shows it when an input is focused. `1` forces it visible, even without focus. Any other set value except `0` also forces it. |
| `JCODE_MOBILE_TEXT_SCALE=1.3` | Override text scale. |
| `JCODE_MOBILE_REDUCE_MOTION=1` | Reduce motion (`0` leaves it off). |
| `JCODE_MOBILE_APPEARANCE=light` | Light appearance, otherwise dark. |
| `JCODE_MOBILE_USE_CLI_ACCOUNT=1` | Reuse the jcode CLI login when available. This opts into a real account. |
| `JCODE_MOBILE_DRAFT=text` | Prefill the composer. |
| `JCODE_MOBILE_DRAWER=1` | Start with the session drawer open. |
| `JCODE_MOBILE_SEARCH=text` | Prefill drawer search. |
| `JCODE_MOBILE_TABS=3` | Request initial session tabs. |
| `JCODE_MOBILE_ERROR=text` | Show a sign-in error banner. |
| `JCODE_MOBILE_OPEN_URL='jcode://new?text=hi'` | Exercise deep-link handling at launch. |
| `JCODE_MOBILE_SEND=text` | Automatically send a message once connected. Use deliberately, it can create real server work. |
| `JCODE_MOBILE_HAPTICS_LOG=1` | Log desktop haptic requests, which do not generate physical phone haptics. |

`FRAMELESS`, `MOUSE`, `DRAWER`, `USE_CLI_ACCOUNT`, and `HAPTICS_LOG` are
presence-based flags: setting them to `0` still enables their behavior. Unset
them to disable it. Do not share account credentials or Apple sign-in token
variables in screenshots, shell history, or bug reports.

```bash
JCODE_MOBILE_SIZE=390x844 JCODE_MOBILE_KEYBOARD=1 \
  scripts/touch-preview.sh
```

For launcher-only validation, `bash -n scripts/touch-preview.sh` checks syntax.
`--help` needs no compositor. `--dry-run` checks prerequisites and prints the
shell-escaped command, but does not compile, launch a GUI, or test touch input.
