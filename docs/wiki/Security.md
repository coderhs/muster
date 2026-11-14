# Security

Muster runs on your Mac for you alone. The rules below keep other local programs,
web pages and the agents themselves from doing more than they should.

## Connecting to the daemon

- The daemon listens on `127.0.0.1` only.
- A client must present the token stored in `$MUSTER_HOME/token` (readable only by
  you).
- A browser page cannot connect: the daemon accepts only the app's own origin.
- Wrong tokens are answered slowly, to make guessing pointless. A correct token is
  never delayed.

## What a card may do

Each running card gets its own limited token (`MUSTER_CARD_TOKEN`). With it the
`muster` CLI can only:

- read cards **linked** to its own card,
- move its own card and edit its own notes,
- send messages to other cards.

It cannot read unlinked cards, start or kill sessions, or see the main token. The
token stops working when the session ends.

## Hooks

Each launch gets a secret, so only the agent Muster started can report events for
that card. Transcript paths reported by hooks are checked before they are read.

## Messages between agents

Delivered as pasted text between turns, with control characters removed, so a
message cannot act as keystrokes.

## Clipboard

Programs in cards cannot write to the Mac clipboard unless you turn that on in
Settings.

## Your agent configuration

Muster does not edit `~/.claude`, `~/.codex` or any other agent's configuration or
credentials. Integration uses per-launch flags and generated files under
`$MUSTER_HOME/agents`.

The full review, with each finding and its fix, is in
[`docs/REVIEW-daemon.md`](../REVIEW-daemon.md).
