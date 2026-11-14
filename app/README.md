# Muster desktop app

Tauri 2 shell (`src-tauri/`) around a Svelte 5 + TypeScript front end (`src/`). The front
end talks to the daemon over the WebSocket in `docs/PROTOCOL.md`; the only host-specific
code is `src/lib/connection/` (Tauri: read `$MUSTER_HOME/daemon.json` and `token`;
browser: `?daemon=` and `#token=` on the page URL, saved values, or a prompt).

Run every tool through mise: `mise exec -- npm ...`, `mise exec -- cargo ...`.

## Develop against the mock daemon

```sh
npm install
npm run mock          # ws://127.0.0.1:7317/ws, token "dev", seeded board
npm run dev           # http://127.0.0.1:1420 connects to the mock with no setup
```

`node mock/daemon.mjs --empty` starts with no projects, `--verbose` logs every request,
`--port`/`--token` change the defaults. With `MUSTER_HOME` set, the mock writes
`daemon.json` and `token` there so the Tauri shell finds it like the real daemon. Its
fake terminal echoes lines and understands `colors`, `need`, `done`, `size`, `flood`, `exit`.

## Against the real daemon

```sh
export MUSTER_HOME=$(mktemp -d) MUSTER_TMUX_SOCKET=muster-test-$RANDOM
../target/debug/musterd run &
# Desktop shell (reads MUSTER_HOME):
npm run tauri dev
# or a browser tab:
open "http://127.0.0.1:1420/?daemon=ws://127.0.0.1:$(jq .port $MUSTER_HOME/daemon.json)/ws#token=$(cat $MUSTER_HOME/token)"
```

## Daemon start and the menu-bar item

On launch (and whenever the page asks for the daemon URL) the shell checks
`$MUSTER_HOME/daemon.json`: if its pid is not a running `musterd`, it starts `musterd run`
detached (own process group, stderr appended to `$MUSTER_HOME/musterd.log`). It never
stops the daemon; quitting the app leaves it running. The binary is looked up in
`$MUSTER_BIN_DIR`, then beside the app executable (the bundled copy), then, in debug
builds, the repository's `target/debug` and `target/release`. Set
`MUSTER_NO_DAEMON_START=1` to disable this (needed when pointing the app at the mock).

Closing the window keeps the app in the menu bar: the window's webview is destroyed and
the tray keeps its own small WebSocket connection to show the needs-you count and list.
"Show Muster" or a card in the tray menu recreates the window; Cmd+Q or "Quit Muster" quits.

Release bundle with the daemon inside:

```sh
cargo build --release -p musterd -p muster-hook   # from the repo root
npm run bundle        # copies them to src-tauri/binaries/ and builds Muster.app
```

## Notifications

Posted by the shell through the UserNotifications framework (also with only the
menu-bar item running), permission asked on first need, click opens the card. They need
the app bundle: the bundle is ad-hoc signed (`bundle.macOS.signingIdentity: "-"`) with the
identifier `dev.muster.desktop`; a bare `muster-app` binary reports notifications as
unavailable. Settings shows the permission state.

What only a person can verify is listed in `HANDCHECK.md`.

## Checks

```sh
npm run check         # svelte-check + tsc
npm test              # vitest: protocol client and board reducer
npm run build         # front-end bundle in dist/
npx tauri build --debug   # Muster.app in src-tauri/target/debug/bundle/macos/
```

## Keys

Cmd+B hides or shows the projects sidebar; in an open card, Cmd+I hides or shows the
details column (Details and Comments tabs; Cmd+Enter posts a comment).
Cmd+T toggles board and tracker (all projects; J/K or arrows, Enter opens).
Board: arrows or H/J/K/L move the selection, Shift+arrows (or Shift+H/J/K/L) move the
selected card, Enter opens, N starts a new session (pick an agent: Enter or 1..9; the last
one used in the project comes first; M for more options), Shift+N opens the full new-card
dialog (title, prompt, draft, folder, worktree), S starts a draft, Cmd+1..9 switch project.
Project settings (sidebar project menu) hold the worktree setup script.
Columns reorder by dragging their header.
Card modal: Esc (outside the terminal) or Cmd+W closes, Ctrl+` toggles focus between the
terminal and the side panel. In the terminal, Cmd+C/Cmd+V copy and paste, Cmd+K clears,
Cmd+Left/Right go to line start/end, Option+Left/Right move by word, Cmd+Backspace
deletes the line, Shift+Enter inserts a newline for agent CLIs.
New card: Cmd+Enter starts, Shift+Cmd+Enter saves a draft.
