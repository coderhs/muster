# Remote access: phone and tablet

Status: step 1 built (2026-11-15). Steps 2 to 4 not started. This is the roadmap's phases 8
and 9 for a single user with several devices.

## Goal

Leave the Mac running and use Muster from a phone or tablet, at home or away. Every
session keeps running on the Mac; the other device only draws the board and terminals.

- **Tablet:** the same UI as the desktop app, in the browser, installed to the home
  screen.
- **Phone:** a layout made for a small screen: which cards need you, then one card at a
  time (terminal, chat, approvals).

## Decisions

- **Tailscale, not plain LAN.** The daemon is a shell on the Mac. `tailscale serve`
  gives HTTPS with a real certificate, works away from home, and opens nothing to the
  internet. Muster runs no service of its own.
- **A web app, not a native app.** The daemon serves the same front end the desktop app
  uses. Installed to the home screen it opens like an app. No App Store and no paid Apple
  developer account.
- **Per-device tokens.** Each device is paired with a one-time link (QR code) and gets
  its own token, which can be revoked on its own. The master token never leaves the Mac.
- **A separate listener.** Remote connections arrive on their own loopback port, so the
  daemon knows a connection is remote without trusting addresses (everything `tailscale
  serve` proxies comes from loopback).

## Steps

1. **Serve the app over Tailscale, with pairing. Built.**
   - Settings → Remote access: the switch, the address, "Pair a device" (QR code and
     link), the list of paired devices with Unpair.
   - The daemon's second listener (`127.0.0.1:7318`) serves the web app, `/pair` and
     `/ws`. Turning it on runs `tailscale serve --bg --https=8443 …`; turning it off
     removes that again.
   - The page pairs from `#pair=<code>` and keeps its device token. An unpaired device
     sees how to pair instead of a token prompt.
   - The app bundle carries the web app in `Contents/Resources/web`.
   - Tests: `crates/musterd/tests/remote.rs` (with a stand-in `tailscale`). It was also
     checked once against the real Tailscale on this Mac: page and assets over HTTPS,
     pairing, a device session over `wss://`, revocation, and switching off.
2. **Tablet polish.** A web app manifest and icons, so it installs to the home screen.
   A key row above the on-screen keyboard for terminals (Esc, Ctrl, Tab, arrows, Ctrl-C).
   Touch dragging of cards. Hide what only works on the Mac, such as the folder picker.
3. **Phone layout.** Below a set width: a list of cards grouped by status (needs you,
   working, done), then a full-screen card with the terminal or chat, approve and deny
   buttons, and the key row.
4. **Alerts on the phone** through ntfy: a card needs you, a card is done.

## Trying it

1. In Muster on the Mac, open Settings → Remote access and turn it on. The first time,
   Tailscale may need HTTPS certificates turned on for the tailnet (admin console, DNS
   page); Settings says so if they are off.
2. Click "Pair a device…" and scan the QR code with the tablet or phone camera. The
   device must be logged in to the same tailnet.
3. On iPhone or iPad: Share → Add to Home Screen. On Android: menu → Install app (or Add
   to Home screen).

## Limits to expect

- **The Mac must stay awake.** A closed MacBook sleeps unless it is on power with an
  external display, or sleep is disabled (`sudo pmset -a disablesleep 1`). "Keep the Mac
  awake while agents are running" only covers running cards.
- **Only what Muster shows is available.** Sessions, terminals, notes and comments work.
  An editor or a browser preview of the project does not, except through a terminal card.
- **Terminal size is shared.** With the same card open on two devices the terminal takes
  the smaller size.
- **Ports are fixed** (7318 on the Mac, 8443 on the tailnet), so only one Muster on the
  Mac can have remote access on at a time (stable or dev, not both).
