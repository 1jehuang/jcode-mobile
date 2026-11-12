# Default directory for new sessions

New sessions started from mobile can open in a chosen cloud directory instead
of the cloud machine's home directory.

## Setting it

Settings → **New sessions start in**:

- Type an absolute path (`/home/you/project`) or a home-relative one
  (`~/project`) and tap **Save** or the keyboard's Done key.
- **Use current** saves the current session's directory. It appears only when
  that differs from the saved default.
- **Reset** clears the default, so new sessions start in the server default
  (the cloud machine's home) again.

Relative paths such as `src/app` are rejected, because the server resolves
them against an unspecified working directory.

## What it applies to

The default is used only when a session is created without an explicit
directory: the header **+**, the Settings **New session** button, swipe to a
new tab, the session map, deep links and voice. It never overrides:

- a directory picked in the composer's directory sheet or the drawer,
- a resumed session (it keeps its original directory),
- a forked session (it inherits the source session's directory).

## Storage

`<platform data dir>/jcode-mobile/prefs.json`, beside `account.json`:

```json
{ "default_dir": "/home/jeremy/jcode-mobile" }
```

It is not secret, so it stays out of the Keychain. Missing, corrupt or invalid
values fall back to the server default. Unknown fields are ignored, so later
preferences can be added to the same file. Signing out does not clear it.

## Tests

- `crates/jcode-mobile-core/src/prefs.rs`: persistence, normalization, corrupt
  and invalid files.
- `src/app/default_dir.rs`: precedence (explicit, resume, fork, default),
  invalid input, and real taps on Use current, Reset and Save in Settings.
