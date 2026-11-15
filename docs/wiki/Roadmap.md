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
| 8 | Remote access through `tailscale serve`, pairing, device revocation | built: the full board in a tablet or computer browser ([plan](../REMOTE-ACCESS.md)) |
| 9 | Web app polish: home-screen install, terminal key row, touch, phone layout | not started ([plan](../REMOTE-ACCESS.md)) |
| 10 | Native apps: Android tablet, then iPhone with notifications and Live Activity | planned ([plan](../MOBILE-APPS.md)); iPhone push needs a paid Apple developer account |

## Decisions already made

- Tauri 2 for the desktop app and, later, mobile.
- Being light on memory and CPU is a priority.
- Remote access uses the user's own Tailscale; Muster runs no service.
- Remote access works in any browser on the tailnet first; it stays as the fallback once native apps exist.
- Native mobile apps use Tauri 2 with native code only for notifications, Live Activities and background work.
- The Mac daemon sends push messages itself (APNs for the iPhone); no hosted relay.

## Not planned

Canvas views, a hosted relay, a browser server edition, SSH remote projects, team
presence, dictation, Windows and Linux.
