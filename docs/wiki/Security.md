# Security

Muster runs on your Mac for you alone. The rules below keep other local programs,
web pages and the agents themselves from doing more than they should.

## Connecting to the daemon

- The daemon listens on `127.0.0.1` only.
- A client must present the token stored in `$MUSTER_HOME/token` (readable only by
  you).
- A web page in your browser cannot connect: the daemon accepts only the app's own
  origin (and, for development, the local dev server on port 1420).
- Wrong tokens are answered slowly, to make guessing pointless. A correct token is
  never delayed.

## Remote access

Off by default. When you turn it on in Settings, Muster asks Tailscale to serve a second,
separate port on your tailnet over HTTPS (`https://<your Mac>.ts.net:8443`). Nothing is
opened to the internet or to your local network.

- Only devices you pair can connect. Pairing uses a one-time link (shown as a QR code)
  that expires after 10 minutes.
- Each device gets its own token. Unpairing it in Settings disconnects it at once.
- The main token never works from another device, and device tokens do not work on the
  Mac's own port.
- A paired device can do what the app can, except turn remote access on or off and pair
  or unpair devices.
- Anyone holding a paired device can run commands on your Mac through its terminals.
  Unpair a device you lose.

## What a card may do

Each running card gets its own limited token (`MUSTER_CARD_TOKEN`). With it the
`muster` CLI can only:

- read cards **linked** to its own card,
- move its own card and edit its own notes,
- send messages to linked cards,
- read and write comments on its own card and on linked cards.

Comments and notes edits are signed by Muster from the token, so an agent cannot write
as you or as another agent. Only you can delete comments.

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

The original review and the fixes it suggested are in
[`docs/REVIEW-daemon.md`](../REVIEW-daemon.md); the rules as implemented are in the
Security section of [`docs/PROTOCOL.md`](../PROTOCOL.md).
