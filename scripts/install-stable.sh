#!/bin/sh
# Build a release Muster.app from this checkout and install it as the stable app.
#
#   scripts/install-stable.sh
#
# Installs to ${MUSTER_APP_DIR:-/Applications}/Muster.app (falls back to ~/Applications
# when that is not writable). The previous copy is moved to the Trash. The stable app
# uses the default data directory (~/Library/Application Support/Muster) and tmux
# socket `muster`; scripts/dev-app.sh uses ~/.muster-dev and `muster-dev`, so both can
# run at once.
#
# Sessions run in tmux and survive the update. To switch to the new version, quit
# Muster, run `pkill -x musterd`, and open it again.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
dest="${MUSTER_APP_DIR:-/Applications}"
[ -w "$dest" ] || dest="$HOME/Applications"
mkdir -p "$dest"
cd "$root"
mise exec -- cargo build --release -p musterd -p muster-hook -p muster-cli
cd "$root/app"
[ -d node_modules ] || mise exec -- npm install
MUSTER_BIN_DIR="$root/target/release" mise exec -- node scripts/prepare-sidecars.mjs
mise exec -- npx tauri build --config src-tauri/tauri.bundle.conf.json --bundles app
built="$root/app/src-tauri/target/release/bundle/macos/Muster.app"
[ -d "$built" ] || { echo "build produced no Muster.app" >&2; exit 1; }
if [ -e "$dest/Muster.app" ]; then
  mv "$dest/Muster.app" "$HOME/.Trash/Muster-$(date +%Y%m%d-%H%M%S).app"
fi
# Move, not copy: a second Muster.app left in the build folder would show up in Spotlight.
mv "$built" "$dest/Muster.app"
echo "installed $dest/Muster.app"
