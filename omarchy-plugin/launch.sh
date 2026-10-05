#!/usr/bin/env bash
# Focus an existing BeeFile window, or start one.
# Match the Wayland class exactly. A title search would also hit editors
# whose window title contains the project name.
set -euo pipefail

binary=${1:-"$HOME/.local/bin/beefile"}
shift || true

address=$(hyprctl clients -j | jq -r 'first(.[] | select(.class == "BeeFile") | .address) // empty')
if [[ -n $address ]]; then
  hyprctl dispatch "hl.dsp.focus({ window = \"address:$address\" })" >/dev/null 2>&1 \
    || hyprctl dispatch focuswindow "address:$address"
  exit 0
fi

exec setsid uwsm-app -- "$binary" "$@"
