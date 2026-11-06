#!/usr/bin/env bash
# App-menu entry point for the native Linux phone preview.
set -euo pipefail
ROOT="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/.." && pwd -P)"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

case "${1:-}" in
  --help|-h)
    printf '%s\n' 'Usage: scripts/mobile-simulator.sh [--install | --dry-run | preview options...]' \
      'Launches the desktop phone preview using touch-preview.sh.' \
      '--install adds an app-menu entry, Desktop shortcut and command.' \
      'Other options, such as --release, are forwarded to touch-preview.sh.'
    exit 0
    ;;
  --install)
    applications="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    desktop="$(xdg-user-dir DESKTOP 2>/dev/null || printf '%s/Desktop' "$HOME")"
    mkdir -p "$applications" "$HOME/.local/bin" "$desktop"
    launcher="$ROOT/scripts/mobile-simulator.sh"
    # Desktop Exec quoting must protect shell-independent reserved characters.
    escaped="${launcher//\\/\\\\}"
    escaped="${escaped//\"/\\\"}"
    escaped="${escaped//\$/\\\$}"
    escaped="${escaped//\`/\\\`}"
    escaped="${escaped//%/%%}"
    entry="$applications/jcode-mobile-simulator.desktop"
    cat > "$entry" <<ENTRY
[Desktop Entry]
Version=1.0
Type=Application
Name=Jcode Mobile Simulator
GenericName=Mobile App Preview
Comment=Open the Jcode phone preview with touch emulation
Exec="$escaped"
Icon=jcode
Terminal=false
Categories=Development;
Keywords=jcode;mobile;simulator;preview;iphone;touch;
StartupNotify=false
ENTRY
    chmod +x "$launcher" "$entry"
    ln -sfn "$launcher" "$HOME/.local/bin/jcode-mobile-simulator"
    cp "$entry" "$desktop/Jcode Mobile Simulator.desktop"
    chmod +x "$desktop/Jcode Mobile Simulator.desktop"
    if command -v gio >/dev/null 2>&1; then
      gio set "$desktop/Jcode Mobile Simulator.desktop" metadata::trusted true 2>/dev/null || true
    fi
    if command -v update-desktop-database >/dev/null 2>&1; then
      update-desktop-database "$applications"
    fi
    printf 'Installed app-menu entry, Desktop shortcut, and command: jcode-mobile-simulator\n'
    exit 0
    ;;
  --dry-run)
    exec "$ROOT/scripts/touch-preview.sh" "$@"
    ;;
esac

state="${XDG_STATE_HOME:-$HOME/.local/state}/jcode-mobile"
mkdir -p "$state"
log="$state/simulator.log"
notify() {
  if command -v notify-send >/dev/null 2>&1; then
    notify-send --app-name='Jcode Mobile Simulator' "$@" >/dev/null 2>&1 || true
  fi
}
notify 'Starting Jcode Mobile Simulator' 'Building the desktop preview if needed…'
# GUI launchers have no terminal. Keep build/runtime failures discoverable.
if "$ROOT/scripts/touch-preview.sh" "$@" > "$log" 2>&1; then
  exit 0
else
  status=$?
  notify --urgency=critical 'Jcode Mobile Simulator could not start' "See $log"
  printf 'Simulator exited with status %s. Log: %s\n' "$status" "$log" >&2
  exit "$status"
fi
