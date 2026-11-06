# Jcode Mobile

A native iPhone client for Jcode, built in Rust with GPUI. Follow coding sessions, send messages, and work with your agents from your phone.

The same app also runs in a phone-sized desktop window for development. The client connects to a Jcode backend, it does not run the coding agent locally on your iPhone.

> **Status:** Active development. Platform tooling, account setup, and iOS signing are not yet a one-command onboarding experience.

## What's here

- Chat transcripts with Markdown rendering and tool-call summaries.
- Multiple sessions, session tabs, and a sidebar.
- Model and provider selection.
- Cloud account and SSH connection support, plus paired-server connections.
- Mobile session synchronization and continuation.
- iOS integrations for notifications, sharing, deep links, and extensions.
- A desktop preview for iterating on the mobile interface.

Some capabilities require a configured account, a compatible Jcode server, or platform-specific provisioning. See the focused guides below for setup details and limitations.

## Development

### Requirements

- A Rust toolchain with Rust 2024 edition support.
- The native build dependencies required by the pinned GPUI fork for your platform.
- `justrust` for the development commands and helper scripts below.
- A graphical desktop session for the preview. The touch-preview helper specifically requires Linux and an existing Wayland session.

Dependencies on GPUI and the Jcode SDK are pinned in the Cargo manifests. A separate GPUI checkout is optional for the pinned-dependency workflows below.

### Desktop preview

```sh
git clone https://github.com/1jehuang/jcode-mobile.git
cd jcode-mobile
justrust run
```

The desktop app opens a phone-sized window and can connect to a real backend. Launching it is not an offline test.

For the native Wayland touch-preview workflow using pinned dependencies:

```sh
GPUI_LOCAL=0 ./scripts/touch-preview.sh
```

The helper defaults to a local `~/gpui` checkout when `GPUI_LOCAL=0` is omitted. See [Touch preview](docs/TOUCH_PREVIEW.md) for configuration and input limitations.

### Checks and tests

```sh
justrust check
justrust test -p jcode-mobile-core
GPUI_LOCAL=0 ./scripts/mobile-fuzz.sh
```

The mobile fuzz launcher runs the app's widget tests headlessly and serially rather than launching the application. Its dependency resolution is offline, so the required dependencies must already be cached.

To exercise the theme, motion, and text-scale matrix:

```sh
GPUI_LOCAL=0 ./scripts/mobile-fuzz.sh --matrix
```

See [Mobile fuzzing](docs/MOBILE_FUZZING.md) for replay controls, environment requirements, and coverage limits.

### iPhone builds

`scripts/ios-bundle.sh` builds and packages an iPhone app into `target/ios/JcodeMobile.ipa`.

This is a specialized workflow, not a standalone bootstrap script. It expects Apple SDK and cross-compilation tooling on Linux, or Xcode tooling on macOS. Device installation also needs signing and provisioning configuration. The Linux path currently selects Rust `1.98.1` and expects an SDK under `~/.swiftpm/swift-sdks/darwin.artifactbundle`.

```sh
CARGO=justrust ./scripts/ios-bundle.sh
```

For simulator packaging, the script supports `SIM=1`. The device development helper, `scripts/ios-dev.sh`, additionally expects a local `~/gpui` checkout, `xtool`, and `pymobiledevice3`.

## Repository layout

| Path | Purpose |
| --- | --- |
| `src/` | GPUI application, mobile interface, and platform integration |
| `crates/jcode-mobile-core/` | UI-independent connection, account, session, and synchronization logic |
| `assets/` | Bundled fonts and icons |
| `ios-res/` | iOS resources and privacy manifest |
| `ios-ext/` | iOS extension sources |
| `scripts/` | Preview, testing, packaging, and device tooling |
| `docs/` | Focused development and integration guides |
| `appstore/` | App Store assets and metadata |

## Further reading

- [Touch preview](docs/TOUCH_PREVIEW.md)
- [Mobile fuzzing](docs/MOBILE_FUZZING.md)
- [Synced continuation](docs/SYNCED_CONTINUATION.md)
- [Push notifications](docs/PUSH.md)
- [iOS extensions](docs/IOS_EXTENSIONS.md)
- [Connection latency](docs/CONNECTION_LATENCY.md)

## Contributing

Small, focused changes are welcome. Include reproduction steps for bugs, describe any backend or device prerequisites, and run the relevant checks before submitting a pull request. Never commit account tokens, SSH private keys, signing credentials, or provisioning secrets.
