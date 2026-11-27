#!/bin/sh
# Build the Debian package of Muster, on Debian or Ubuntu.
#
#   scripts/package-deb.sh [VERSION]
#
# The result is target/package/muster_<version>_<amd64|arm64>.deb. It is built for the
# machine it runs on, and needs that distribution's glibc or newer: build on the oldest
# release you mean to support (the release workflow uses Debian 12).
#
# Build needs: libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
# libssl-dev libxdo-dev build-essential file, plus the Rust and Node in mise.toml.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
[ "$(uname -s)" = Linux ] || { echo "package-deb.sh builds on Linux" >&2; exit 1; }
version=${1:-$(sed -n 's/^ *"version": "\(.*\)",$/\1/p' app/src-tauri/tauri.conf.json)}
version=${version#v}
# Keep this machine's folder names out of the binaries.
export RUSTFLAGS="--remap-path-prefix=$HOME=/build --remap-path-prefix=$root=/build/muster"

cargo build --release -p musterd -p muster-hook -p muster-cli
target=${CARGO_TARGET_DIR:-$root/target}
cd "$root/app"
[ -d node_modules/.bin ] || npm ci
MUSTER_BIN_DIR="$target/release" node scripts/prepare-sidecars.mjs
app_target=${CARGO_TARGET_DIR:-$root/app/src-tauri/target}
npx tauri build --config src-tauri/tauri.bundle.conf.json --config "{\"version\":\"$version\"}" --bundles deb

out="$root/target/package"
mkdir -p "$out"
arch=$(dpkg --print-architecture)
cp "$app_target/release/bundle/deb/Muster_${version}_${arch}.deb" "$out/muster_${version}_${arch}.deb"
echo "$out/muster_${version}_${arch}.deb"
