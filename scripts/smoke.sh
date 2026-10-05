#!/usr/bin/env bash
# Build BeeFile, open it on a temp folder, and require the window to stay up.
# Needs a running Wayland session (Omarchy / Hyprland).
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
  echo "WAYLAND_DISPLAY is not set. Run this from an Omarchy session." >&2
  exit 1
fi

cargo build

fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/docs"
printf 'hello\n' > "$fixture/notes.txt"
printf 'hidden\n' > "$fixture/.secret"

log="$(mktemp)"
bin="./target/debug/beefile"
# timeout exits 124 when the app is still running. Any other code is a crash or an early quit.
set +e
timeout --signal=TERM --kill-after=2s 8s "$bin" "$fixture" >"$log" 2>&1
code=$?
set -e

if [[ "$code" -eq 124 ]]; then
  echo "smoke ok: BeeFile stayed open on $fixture"
  exit 0
fi

echo "smoke failed: beefile exited $code" >&2
if [[ -s "$log" ]]; then
  echo "----- log -----" >&2
  cat "$log" >&2
fi
exit 1
