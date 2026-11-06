#!/bin/sh
# Re-applies the bundle onto the launcher's source into a scratch stage (compile-check loop).
set -e
S=${SCRATCH:-$HOME/.hermes/cache/scratch}
HERE=$(cd "$(dirname "$0")" && pwd)
rm -rf "$S/seedmap-mods" "$S/seedmap-stage"
mkdir -p "$S/seedmap-mods/seedmap"
cp "$HERE"/horizon-mixin.toml "$HERE"/model.rs "$HERE"/worker.rs "$HERE"/runtime.rs "$HERE"/h_*.rs "$S/seedmap-mods/seedmap/"
$HOME/.local/share/horizon-launcher/native/tools/0d6b103c7063/bin/horizon-mixins apply \
  --workspace $HOME/.local/share/horizon-launcher/native/source/66f3981f522b24c250b21523a70080965b940424 \
  --mods "$S/seedmap-mods" --out "$S/seedmap-stage"
