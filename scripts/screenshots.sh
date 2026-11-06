#!/usr/bin/env bash
# Render every plugin's editor (or the ones named) to docs/screenshots/<plugin>.png, headlessly.
# Each plugin's examples/screenshot.rs runs its real DSP on a synthetic signal and draws the
# editor through crates/ui_preview, then stitches them into docs/screenshots/all.png. Usage: scripts/screenshots.sh [plugin ...]
set -euo pipefail
cd "$(dirname "$0")/.."

plugins=("$@")
if [ ${#plugins[@]} -eq 0 ]; then
  for dir in plugins/*/; do
    [ -f "$dir/examples/screenshot.rs" ] && plugins+=("$(basename "$dir")")
  done
fi

for plugin in "${plugins[@]}"; do
  cargo run -q -p "$plugin" --release --example screenshot
  echo "docs/screenshots/$plugin.png"
done

cargo run -q -p ui_preview --release --example stitch
