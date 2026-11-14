# Development

## Tools

- [mise](https://mise.jdx.dev) pins Rust and Node (`mise.toml`). Run tools through
  `mise exec --`, or let mise activate in your shell.
- npm for the front end.
- tmux from Homebrew.
- Xcode Command Line Tools are enough; full Xcode is not needed.

## Repository layout

```
Cargo.toml            Rust workspace (crates/*)
crates/
  muster-proto/       wire types
  muster-core/        SQLite store, tmux backend, process stats
  muster-agents/      agent adapters and their test fixtures
  muster-hook/        hook forwarder binary
  musterd/            the daemon and its integration tests
  muster-cli/         the `muster` CLI
app/
  src/                Svelte 5 front end
  src-tauri/          Tauri 2 shell (its own Cargo project)
  mock/daemon.mjs     a fake daemon for front-end work
docs/                 plan, protocol, agent notes, security review, this wiki
scripts/              stable install and development runners
```

## Stable and dev

Two copies run side by side so Muster can be developed from inside Muster.

| | Stable | Dev |
|---|---|---|
| Start | open `/Applications/Muster.app` | `scripts/dev-app.sh` |
| Name | Muster | Muster Dev (process `muster-dev`) |
| Data | `~/Library/Application Support/Muster` | `~/.muster-dev` |
| tmux socket | `muster` | `muster-dev` |
| Build | release | debug |

Working loop:

1. Work in a card in **stable**.
2. Run `scripts/dev-app.sh`. The Muster Dev window opens.
3. Front-end changes (`app/src`) reload live. Shell changes (`app/src-tauri`) rebuild
   and reopen the window. Daemon or CLI changes (`crates/`) need the script run again;
   it restarts the dev daemon and dev sessions survive.
4. When dev is good, run `scripts/install-stable.sh`, quit Muster, run
   `pkill -x musterd`, and open Muster again. Sessions survive.

Only one Muster Dev runs at a time (the front-end dev server uses port 1420).

`scripts/dev.sh` runs only the dev daemon in the foreground;
`scripts/dev.sh muster <args>` runs the CLI against it.

## Tests

```sh
mise exec -- cargo test --workspace     # Rust: unit and daemon integration tests
cd app && mise exec -- npm run check    # type check
cd app && mise exec -- npm test         # front-end unit tests
```

The daemon integration tests start real daemons and tmux servers on sockets named
`muster-test-*` in temporary directories. They never touch the stable or dev data.
Agent tools are replaced by stand-in scripts; no real agent is run.

## Front end without a daemon

```sh
cd app && npm run mock     # fake daemon
cd app && npm run dev      # front end in a browser at localhost:1420
```

## Rules for this codebase

- Never edit a user's agent configuration or credentials.
- tmux sockets must be named `muster*`. Never run `tmux kill-server` without `-L`.
- Tests use a temporary `MUSTER_HOME`.
- No code or text is copied from other session managers.
- Changes to messages between daemon and clients are written into
  `docs/PROTOCOL.md` in the same change.
