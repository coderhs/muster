# Sessions and tmux

## One tmux session per card

Muster runs its own tmux server, separate from any tmux you use yourself:

- socket name `muster` (so `tmux -L muster ...` reaches it)
- one tmux session per card, named `m-<cardId>`
- a small extra session `muster-ctl` that the daemon uses to receive notifications

The status bar and the prefix key are turned off, so inside a card tmux is invisible.

## Looking at a card

When a card is opened, the daemon starts a tmux client for that session and streams
its output to the window, where xterm.js draws it. Typing goes the other way. The
session is sized to the window; if two clients look at the same card, the smaller
size wins.

Closing the card detaches the client. The agent keeps running.

## What survives what

| Event | Sessions |
|---|---|
| Close the window, quit the app | Keep running. |
| Daemon stops or crashes | Keep running; the next daemon finds them again. |
| App update | Keep running. |
| Mac restarts | Processes are gone. Cards are marked restorable: Restore replays the saved scrollback and resumes the agent's conversation by its session id. |

## Environment inside a card

Every session gets:

| Variable | Purpose |
|---|---|
| `MUSTER_CARD_ID` | Which card this is. |
| `MUSTER_AGENT_ID` | Which agent runs here. |
| `MUSTER_HOOK_SOCK` | Where `muster-hook` sends events. |
| `MUSTER_CARD_TOKEN` | A limited token for the `muster` CLI (see [Security](Security.md)). |
| `MUSTER_CLI` | Path to the `muster` CLI. |

`TMUX`, `TMUX_PANE` and `MUSTER_HOME` are removed, so a tmux command typed inside a
card cannot reach Muster's own server.

## Text encoding

tmux is always started in UTF-8 mode and sessions get a UTF-8 locale. Without this,
an app started from the Finder has no locale, and tmux replaces every non-ASCII
character (prompt arrows, box borders) with `_` or `-`.

## Scrolling

Scrollback is held by tmux. The mouse wheel moves one line per step.

## Useful commands

```sh
tmux -L muster list-sessions          # every card session
tmux -L muster attach -t '=m-<id>'    # look at one directly (detach: close the tab)
muster attach <card>                  # the same through the CLI; Ctrl-] detaches
```

Never run `tmux kill-server` without `-L`: that would stop your own tmux server.
