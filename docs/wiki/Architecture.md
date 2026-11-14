# Architecture

```
 ┌────────────────────┐        WebSocket (127.0.0.1, token)
 │  Muster.app        │◄──────────────────────────────────┐
 │  Tauri shell       │                                   │
 │  Svelte front end  │                                   ▼
 └────────────────────┘                          ┌────────────────┐
                                                 │    musterd     │
 ┌────────────────────┐   Unix socket            │  (the daemon)  │
 │ agent CLI in a     │──────────────────────────►                │
 │ card (via          │   muster-hook            │  SQLite board  │
 │ muster-hook)       │                          │  hook receiver │
 └────────▲───────────┘                          └───────┬────────┘
          │ runs inside                                  │ controls
          │                                              ▼
 ┌────────┴─────────────────────────────────────────────────────┐
 │ tmux server (socket "muster"), one tmux session per card     │
 └──────────────────────────────────────────────────────────────┘
```

## The pieces

| Piece | Path | Job |
|---|---|---|
| Wire types | `crates/muster-proto` | The message shapes shared by daemon and clients. |
| Core | `crates/muster-core` | SQLite store, tmux backend, process statistics, keep-awake. |
| Agent adapters | `crates/muster-agents` | Per-agent launch and resume commands, hook setup, transcript and usage readers. |
| Hook binary | `crates/muster-hook` | A tiny program agents call on events; forwards them to the daemon. |
| Daemon | `crates/musterd` | WebSocket API, hook receiver, status tracking, context sharing. |
| CLI | `crates/muster-cli` | `muster`, for people and for agents inside cards. |
| App | `app/` | Tauri 2 shell (`app/src-tauri`) and Svelte 5 front end (`app/src`). |

## Why a daemon

- Sessions and the board outlive the window. Closing or updating the app does not
  touch running agents.
- Every client is equal. The desktop window and a future phone client use the same
  API, described in [`docs/PROTOCOL.md`](../PROTOCOL.md).
- The app stays small. The front end talks to the daemon directly; the Tauri shell
  only finds the daemon, starts it when needed, and provides the menu-bar icon and
  notifications.

## Where data lives

The data directory is `$MUSTER_HOME`, by default `~/Library/Application Support/Muster`.

| File | Contents |
|---|---|
| `muster.db` | The board: projects, columns, cards, links, activity. |
| `token` | The secret a client needs to connect. Readable only by you. |
| `daemon.json` | The running daemon's process id and port. |
| `hook.sock` | The Unix socket agents report events to. |
| `scrollback/` | Saved terminal history, used to restore a card after a reboot. |
| `agents/` | Generated per-agent files: settings, instructions, the `muster` skill. |
| `tmux.conf` | Generated tmux configuration. Overwritten at start. |
| `musterd.log` | The daemon's log. |

## How a status reaches the board

1. The daemon starts the agent with flags that register `muster-hook` for the
   agent's events. The user's own agent configuration is never edited.
2. When something happens (a turn starts, a tool needs approval, the turn ends), the
   agent runs `muster-hook`, which sends the event over `hook.sock`.
3. The daemon updates the card's status in SQLite and pushes an event to every
   connected client.
4. The window updates the card. Nothing polls.

Agents without hooks fall back to terminal activity: tmux tells the daemon when a
session has gone quiet or produced output.

## Lightness

Low memory and CPU is a requirement.

- One live terminal at a time; others are plain cards.
- Events instead of polling wherever the agent or tmux can push.
- Statistics are sampled only while a client is showing them.
- Scrollback is kept by tmux and on disk, not in the app's memory.
