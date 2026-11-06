#!/usr/bin/env bash
# Build every plugin (or the ones named) and install the VST3 bundles for the current user.
# Usage: scripts/install.sh [plugin ...]
set -euo pipefail
cd "$(dirname "$0")/.."

dest="$HOME/Library/Audio/Plug-Ins/VST3"
mkdir -p "$dest"

plugins=("$@")
if [ ${#plugins[@]} -eq 0 ]; then
  for dir in plugins/*/; do plugins+=("$(basename "$dir")"); done
fi

args=()
for plugin in "${plugins[@]}"; do args+=(-p "$plugin"); done
cargo xtask bundle "${args[@]}" --release

for plugin in "${plugins[@]}"; do
  name=$(awk -v key="[$plugin]" '$0 == key { found = 1; next } found && /^name/ { gsub(/.*= *"|"$/, ""); print; exit }' bundler.toml)
  bundle="target/bundled/$name.vst3"
  [ -d "$bundle" ] || { echo "missing $bundle" >&2; exit 1; }
  [ -e "$dest/$name.vst3" ] && trash "$dest/$name.vst3"
  cp -R "$bundle" "$dest/"
  codesign -v "$dest/$name.vst3"
  echo "installed $name"

  # Factory presets as .vstpreset files, where REAPER and other hosts look for them.
  if [ -f "plugins/$plugin/examples/write_presets.rs" ]; then
    cargo run -q -p "$plugin" --release --example write_presets -- "$HOME/Library/Audio/Presets/Clowd/$name"
    echo "installed $name presets"
  fi
done
echo "Restart your DAW (or clear its plugin cache and re-scan) to load the new builds."
