#!/bin/sh
# Run a development daemon that never touches the stable install:
# its own data directory, tmux server and port file.
#
#   scripts/dev.sh            build (debug) and run musterd in the foreground
#   scripts/dev.sh muster ... run the dev CLI against the dev daemon
#
# Override with MUSTER_DEV_HOME / MUSTER_DEV_SOCKET. Keep the home path short: the
# hook socket ($MUSTER_HOME/hook.sock) must stay under macOS's 104-byte limit.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
export MUSTER_HOME="${MUSTER_DEV_HOME:-$HOME/.muster-dev}"
export MUSTER_TMUX_SOCKET="${MUSTER_DEV_SOCKET:-muster-dev}"
case "$MUSTER_TMUX_SOCKET" in muster*) ;; *) echo "MUSTER_DEV_SOCKET must start with 'muster'" >&2; exit 1 ;; esac
mkdir -p "$MUSTER_HOME"
cd "$root"
mise exec -- cargo build -q -p musterd -p muster-hook -p muster-cli
if [ "${1:-}" = muster ]; then
  shift
  exec "$root/target/debug/muster" "$@"
fi
echo "dev daemon: MUSTER_HOME=$MUSTER_HOME MUSTER_TMUX_SOCKET=$MUSTER_TMUX_SOCKET" >&2
exec "$root/target/debug/musterd" run
