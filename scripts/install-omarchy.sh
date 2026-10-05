#!/usr/bin/env bash
# Build BeeFile and add it as an Omarchy shell plugin.
#
# Installs the release binary to ~/.local/bin/beefile and links
# omarchy-plugin/ to ~/.config/omarchy/plugins/bart.beefile, then enables
# the bar button on the right. Re-run after pulling to replace the binary.
# The plugin link is left in place.
#
#   scripts/install-omarchy.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
plugin_id=bart.beefile
plugin_src=$root/omarchy-plugin
plugins_dir=$HOME/.config/omarchy/plugins
link=$plugins_dir/$plugin_id
bin_dir=$HOME/.local/bin
bin=$bin_dir/beefile

fail() {
  echo "install-omarchy: $*" >&2
  exit 1
}

command -v cargo >/dev/null || fail "cargo is not on PATH"
command -v jq >/dev/null || fail "jq is not on PATH"
command -v omarchy >/dev/null || fail "omarchy is not on PATH"
[[ -f $plugin_src/manifest.json ]] || fail "missing $plugin_src/manifest.json"

echo "==> Checking the plugin"
omarchy plugin validate "$plugin_src"

echo "==> Release build"
(cd "$root" && cargo build --release)
[[ -x $root/target/release/beefile ]] || fail "release binary was not built"

echo "==> Installing $bin"
mkdir -p "$bin_dir"
cp "$root/target/release/beefile" "$bin.new"
chmod 755 "$bin.new"
mv -f "$bin.new" "$bin"

echo "==> Linking $link"
mkdir -p "$plugins_dir"
if [[ -L $link ]]; then
  current=$(readlink -f "$link")
  want=$(readlink -f "$plugin_src")
  [[ $current == "$want" ]] || fail "plugin link points at $current, not $want"
elif [[ -e $link ]]; then
  fail "$link exists and is not a symlink"
else
  ln -s "$plugin_src" "$link"
fi

echo "==> Enabling $plugin_id"
# rescanPlugins unloads every panel and widget on the shell's UI thread, then
# finishes the scan on the next turn. The default IPC budget is 2s. A reload
# that overruns it is reported as "omarchy-shell is not responding", and the
# follow-up call times out the same way because the reload is still running.
export OMARCHY_SHELL_IPC_TIMEOUT="${OMARCHY_SHELL_IPC_TIMEOUT:-15s}"
if ! omarchy-shell shell rescanPlugins >/dev/null; then
  fail "the shell did not rescan. When it is running: omarchy plugin enable $plugin_id --section right"
fi

plugin_list=""
read_plugin_list() {
  plugin_list=$(omarchy plugin list --json) || fail "the shell did not answer. Run: omarchy restart shell"
}

discovered=0
deadline=$((SECONDS + 20))
while (( SECONDS < deadline )); do
  read_plugin_list
  if jq -e --arg id "$plugin_id" 'any(.[]; .id == $id)' <<<"$plugin_list" >/dev/null; then
    discovered=1
    break
  fi
  sleep 0.2
done
((discovered)) || fail "plugin '$plugin_id' is not known. Run: omarchy-shell shell rescanPlugins"

read_plugin_list
if jq -e --arg id "$plugin_id" 'any(.[]; .id == $id and .enabled)' <<<"$plugin_list" >/dev/null; then
  echo "Already enabled"
else
  omarchy plugin enable "$plugin_id" --section right
fi

echo
echo "BeeFile is on the bar."
echo "  Binary   $bin"
echo "  Plugin   $link"
echo "  Click    focuses BeeFile, or starts it"
echo "  Again    omarchy-shell bart.beefile app"
echo
echo "A key can do the same, in ~/.config/hypr/bindings.lua:"
echo '  o.bind("SUPER + ALT + E", "BeeFile", "omarchy-shell bart.beefile app")'
echo
echo "Edits to the plugin on another drive may not hot-reload."
echo "Run: omarchy restart shell"
