#!/bin/sh
# Run the development build of the whole app ("Muster Dev") next to the stable one.
#
#   scripts/dev-app.sh
#
# It has its own data directory (~/.muster-dev), tmux server (muster-dev), app name and
# settings, so it never touches the stable app's board or sessions. Safe to run from a
# card inside stable Muster.
#
# Front-end changes reload live. Changes to the app shell (app/src-tauri) rebuild and
# relaunch the window. Changes to the daemon or CLI crates need this script run again:
# it rebuilds them and restarts the dev daemon. Dev sessions live in tmux and survive.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
# Drop anything inherited from a stable card, so the dev daemon starts clean.
for v in $(env | sed -n 's/^\(MUSTER_[A-Z_]*\)=.*/\1/p'); do
  case "$v" in MUSTER_DEV_HOME|MUSTER_DEV_SOCKET) ;; *) unset "$v" ;; esac
done
unset TMUX TMUX_PANE || true
export MUSTER_HOME="${MUSTER_DEV_HOME:-$HOME/.muster-dev}"
export MUSTER_TMUX_SOCKET="${MUSTER_DEV_SOCKET:-muster-dev}"
export MUSTER_BIN_DIR="$root/target/debug"
case "$MUSTER_TMUX_SOCKET" in muster*) ;; *) echo "MUSTER_DEV_SOCKET must start with 'muster'" >&2; exit 1 ;; esac
mkdir -p "$MUSTER_HOME"
cd "$root"
mise exec -- cargo build -p musterd -p muster-hook -p muster-cli

# Stop the previous dev daemon so the window starts the freshly built one.
if [ -f "$MUSTER_HOME/daemon.json" ]; then
  pid=$(sed -n 's/.*"pid"[^0-9]*\([0-9][0-9]*\).*/\1/p' "$MUSTER_HOME/daemon.json" | head -1)
  if [ -n "$pid" ] && [ "$(ps -o comm= -p "$pid" 2>/dev/null | sed 's|.*/||')" = musterd ]; then
    kill "$pid" 2>/dev/null || true
    i=0
    while kill -0 "$pid" 2>/dev/null && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
  fi
fi

echo "Muster Dev: MUSTER_HOME=$MUSTER_HOME MUSTER_TMUX_SOCKET=$MUSTER_TMUX_SOCKET" >&2
cd "$root/app"
[ -d node_modules ] || mise exec -- npm install
exec mise exec -- npx tauri dev --config src-tauri/tauri.dev.conf.json -f dev-bin -- --bin muster-dev
