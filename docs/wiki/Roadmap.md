# Roadmap

The full plan with its reasoning is in [`docs/PLAN.md`](../PLAN.md).

| # | Phase | State |
|---|---|---|
| 0 | Spikes: tmux survival, re-attach, hooks, usage source | done |
| 1 | Daemon core: sessions over the API, persistent board | done |
| 2 | Board: columns, drag and drop, card terminal, drafts, worktrees, projects | done |
| 3 | Claude Code integration: status, resume, approvals, questions, context meter, chat | done; approvals from the card not yet confirmed against a real session |
| 4 | Tracker: wait and idle times, menu-bar count, notifications, keep-awake, usage limits, memory and CPU | done; notifications unconfirmed with ad-hoc signing |
| 5 | Context sharing: links, reads, messages, handoff | done |
| 6 | More agents: Codex, opencode, Cursor, Gemini, Antigravity | built; only Claude Code and Cursor run for real |
| 7 | Hardening: daily use, memory budget, crash, reboot and upgrade paths | in progress |
| 8 | Remote access through `tailscale serve`, pairing, device revocation | not started |
| 9 | Home-screen web app for iPhone, ntfy alerts | not started |
| 10 | Native iOS shell | optional |

## Decisions already made

- Tauri 2 for the desktop app and, later, mobile.
- Being light on memory and CPU is a priority.
- Remote access uses the user's own Tailscale; Muster runs no service.
- Mobile starts as a home-screen web app, so no paid Apple developer account is needed.
- Alerts to the phone start with ntfy.

## Not planned

Canvas views, a hosted relay, a browser server edition, SSH remote projects, team
presence, dictation, Windows and Linux.
