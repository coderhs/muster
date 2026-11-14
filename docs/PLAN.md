# Muster — plan

Working name: **Muster**. A macOS app that runs coding-agent CLIs (Claude Code, Codex,
Cursor, opencode, Antigravity, ...) as cards on a kanban board, with a tracker that shows
which sessions need attention. Later, a phone client reaches the same sessions over the
user's own Tailscale network.

## Principles

- **Light.** Low memory and CPU is a product requirement, not a nice-to-have. Events over
  polling, one live terminal at a time, nothing held in memory that tmux or SQLite can hold.
- **Daemon and clients.** One daemon owns all state. The desktop window and the phone are
  both clients of the same WebSocket API (`docs/PROTOCOL.md`).
- **No service of ours.** Nothing depends on infrastructure we run. Remote access is the
  user's own Tailscale.
- **Non-invasive.** Prefer per-launch flags and environment variables over editing an
  agent CLI's config files. Where a config edit is unavoidable it needs consent and a clean
  uninstall.
- **Original work only.** No code or text is copied from other session managers.

## Components

| Path | What |
|---|---|
| `crates/muster-proto` | Shared wire types (serde). Source of truth for `docs/PROTOCOL.md`. |
| `crates/muster-core` | SQLite store, tmux session backend, board logic. |
| `crates/muster-agents` | Agent adapters: launch/resume commands, hook setup, event normalisation, transcript and usage readers. |
| `crates/muster-hook` | Tiny binary the agent CLIs invoke on hook events; forwards to the daemon. |
| `crates/musterd` | The daemon: WebSocket API, hook receiver, wiring. |
| `crates/muster-cli` | `muster` CLI for people and for agents (read linked context, move own card). |
| `app/` | Tauri 2 shell (`app/src-tauri`) and Svelte front end (`app/src`). |

Toolchain is pinned in `mise.toml`; run tools through `mise exec --`.

## Runtime layout

- Data directory: `$MUSTER_HOME`, default `~/Library/Application Support/Muster`.
  Contains `muster.db`, `token` (0600), `daemon.json` (pid, port), `hook.sock`, `scrollback/`.
- tmux: dedicated server socket `-L muster` (overridable with `MUSTER_TMUX_SOCKET`), one
  tmux session per card named `m-<cardId>`, generated config, status bar off.
- Every agent process gets `MUSTER_CARD_ID`, `MUSTER_AGENT_ID` and `MUSTER_HOOK_SOCK` in
  its environment. The hook binary does nothing when these are absent.

## Phases

Desktop first; mobile starts only after phase 7.

| # | Phase | Done when |
|---|---|---|
| 0 | Spikes | tmux session survives a daemon restart; xterm.js re-attaches with a correct screen; a Claude Code hook event reaches the daemon; usage-limit data source chosen; memory budgets set from measurements |
| 1 | Daemon core | Create, attach, resize, kill sessions over the API; board persists; daemon starts at login |
| 2 | Board | Columns, drag and drop, card modal with live terminal, draft cards that start an agent with a prompt, start in a new git worktree, multiple projects |
| 3 | Claude Code integration | Status badges; resume after reboot; approvals and questions answered from the card; context meter; chat view |
| 4 | Tracker | Needs you / Finished / Running / Idle with wait and idle times; menu-bar count; notifications; keep-awake; usage-limit strip; per-session and total RAM/CPU |
| 5 | Context sharing | Card links; linked agent reads another card's transcript, summary, screen, notes; messages and @mentions delivered only when the receiver is idle; handoff |
| 6 | More agents | Codex, opencode, Cursor with status and resume; Antigravity, Gemini, Copilot, Grok with launch, resume, basic status; custom commands |
| 7 | Hardening | Daily use; memory in budget; crash, reboot, upgrade paths tested |
| 8 | Remote access | Reachable through `tailscale serve`; QR pairing with a token; device revocation |
| 9 | Home-screen web app | Tracker, chat, approvals, board, terminal on iPhone via Safari "Add to Home Screen"; ntfy alerts |
| 10 | Native iOS shell | Optional; same front end in Tauri iOS |

## Data model (SQLite)

- `projects(id, name, path, color, position, created_at)`
- `columns(id, project_id, title, color, category, position)` — category:
  `unstarted | started | done | closed`. New projects get To Do / In Progress / Done.
- `cards(id, project_id, column_id, rank, title, agent_id, cwd, branch, prompt, notes,
  priority, due_at, agent_session_id, status, status_since, turn_started_at,
  last_activity_at, unread, created_at, updated_at)`
  - `rank` is a fractional-index string, so a move rewrites one row.
  - `status`: `draft | running | needs_you | done | idle | exited`.
  - A draft card has no session; `card.start` launches the agent.
- `links(card_a, card_b)` — context links.
- `activity(id, card_id, ts, kind, payload)` — comments, moves, status changes.

## Status model

`draft` → (start) → `running` ⇄ `needs_you` → `done` (turn finished, unread) → `idle`
(read) → `running` ... ; `exited` when the session is gone. Each change records
`status_since`; `turn_started_at` and `last_activity_at` feed the tracker's timers. For
agents without hooks, `last_activity_at` falls back to the terminal's last output time.

## Out of scope

Canvas and free-form node types, any hosted relay, browser server edition, SSH remote
projects, team presence, dictation, Windows and Linux.
