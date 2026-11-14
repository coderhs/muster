# Troubleshooting

## The app shows an old behaviour after an update

The window is new but the daemon is still the old one. Quit Muster, run
`pkill -x musterd`, open Muster again. Sessions survive.

Check which daemon is running:

```sh
pgrep -fl musterd
```

There should be one for stable (inside `/Applications/Muster.app`) and, while
developing, one from `target/debug`.

## Prompt arrows or borders show as `_` or `-`

An old daemon without UTF-8 mode is running. Restart the daemon as above.

## A second Muster appears in Spotlight

A built copy was left in a build folder. `scripts/install-stable.sh` moves the built
app instead of copying it, and the build folders are marked as not indexed. Move any
stray `Muster.app` to the Trash.

## "tmux is not installed"

`brew install tmux`, then restart the daemon. `MUSTER_TMUX` can point at a specific
binary.

## Hooks do not arrive (status never changes)

The hook socket path must be shorter than 104 bytes. Keep `MUSTER_HOME` short. The
daemon reports this as a notice in the app.

## A card says the session was interrupted

The Mac restarted or the tmux server stopped. Use Restore on the card: it replays the
saved scrollback and resumes the agent's conversation.

## Resuming by session id fails

The card's Folder must be the folder the session was created in, and the session
must not be open somewhere else at the same time.

## Logs

- Daemon: `$MUSTER_HOME/musterd.log`
- Dev daemon: `~/.muster-dev/musterd.log`

## Starting clean in dev

Stop Muster Dev, then move `~/.muster-dev` to the Trash. Stable is not affected.
