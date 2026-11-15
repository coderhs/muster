#!/bin/sh
# Build a Muster.app to give to someone else, as a zip with first-run instructions.
#
#   scripts/package.sh
#
# The result is target/package/Muster-<commit>-macos-arm64.zip. It runs on Apple
# silicon Macs (M1 and later) with macOS 12 or newer. It does not touch the installed
# stable app or its data.
#
# The app is signed ad hoc, not with an Apple Developer ID, and is not notarized, so
# macOS asks the recipient to allow it once; the instructions in the zip say how.
#
# Unlike scripts/install-stable.sh, the build rewrites source paths recorded in the
# binaries, which would otherwise carry this machine's home folder name.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
[ "$(uname -m)" = arm64 ] || { echo "package.sh builds for Apple silicon; run it on one" >&2; exit 1; }
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "commit or stash your changes first: a shared build should match a commit" >&2
  exit 1
fi
commit=$(git rev-parse --short=9 HEAD)
export RUSTFLAGS="--remap-path-prefix=$HOME=/build --remap-path-prefix=$root=/build/muster"

mise exec -- cargo build --release -p musterd -p muster-hook -p muster-cli
cd "$root/app"
[ -d node_modules ] || mise exec -- npm install
MUSTER_BIN_DIR="$root/target/release" mise exec -- node scripts/prepare-sidecars.mjs
mise exec -- npx tauri build --config src-tauri/tauri.bundle.conf.json --bundles app
built="$root/app/src-tauri/target/release/bundle/macos/Muster.app"
[ -d "$built" ] || { echo "build produced no Muster.app" >&2; exit 1; }

for b in "$built"/Contents/MacOS/*; do
  if LC_ALL=C grep -a -q "$HOME" "$b"; then
    echo "$(basename "$b") still contains $HOME; not packaging" >&2
    exit 1
  fi
done

out="$root/target/package"
stage="$out/stage-$commit"
mkdir -p "$stage"
# Move, not copy: a second Muster.app left in the build folder would show up in Spotlight.
[ ! -e "$stage/Muster.app" ] || mv "$stage/Muster.app" "$HOME/.Trash/Muster-package-$(date +%Y%m%d-%H%M%S).app"
mv "$built" "$stage/Muster.app"
cat > "$stage/READ ME FIRST.txt" <<TXT
Muster (build $commit)

Needs: an Apple silicon Mac (M1 or later), macOS 12 or newer.

1. Install tmux, which Muster runs its sessions in:
       brew install tmux
   Install the agent CLIs you want to use (Claude Code, Codex, Cursor, ...).

2. Drag Muster.app into Applications.

3. Allow it once. This build is not signed with an Apple Developer ID, so macOS
   blocks it the first time. In Terminal:
       xattr -dr com.apple.quarantine /Applications/Muster.app
   Or: open Muster, dismiss the warning, then System Settings > Privacy & Security
   > "Open Anyway".

4. Open Muster and add a project folder.

Your data stays on your Mac, in ~/Library/Application Support/Muster.
Settings > About shows which build this is.
TXT
zip="$out/Muster-$commit-macos-arm64.zip"
[ ! -e "$zip" ] || mv "$zip" "$HOME/.Trash/$(basename "$zip" .zip)-$(date +%Y%m%d-%H%M%S).zip"
# ditto keeps the code signature and extended attributes intact.
(cd "$stage" && ditto -c -k --sequesterRsrc . "$zip")
echo "$zip"
