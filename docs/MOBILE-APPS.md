# Native mobile apps: Android tablet and iPhone

Status: planned, not started. Recorded 2026-11-15. Builds on remote access
([`REMOTE-ACCESS.md`](REMOTE-ACCESS.md)), which stays as the fallback: any device on the
tailnet can still use Muster in a browser.

## Goal

Apps that feel native on the devices in use:

- **Android tablet:** the full board as on the desktop, made for touch, with
  notifications that have Approve and Deny buttons.
- **iPhone:** a phone layout, notifications, and a Live Activity on the Lock Screen and in
  the Dynamic Island showing which cards are working and which need you.
- **Both:** the last known state stays visible when the Mac cannot be reached.

## The deciding constraint: Apple's paid developer account

Live Activities, the Dynamic Island and notifications while the app is closed all need
Apple's push service (APNs). A free Apple ID cannot use push.

| | Free Apple ID | Paid account ($99/year) |
|---|---|---|
| Install on the iPhone | Yes, but it expires every 7 days and must be reinstalled from Xcode | A year, or through TestFlight |
| Notifications while the app is closed | No | Yes |
| Live Activity / Dynamic Island | Only while the app is open | Yes, updated by the Mac |
| Approve / Deny on a notification | No | Yes |

Android has no such cost: the app installs as an APK, with no fee and no expiry.

**Open decision:** pay for the account, borrow one (the app and its push key then belong
to that account), or ship the iPhone app without push and Live Activities (ntfy could
still deliver plain alerts).

## Approach

**Tauri 2 mobile, with native code only where needed.** It keeps the earlier decision
(Tauri for desktop and mobile) and reuses the Svelte front end and protocol client.

- **iPhone (Swift):** a widget extension for the Live Activity and Dynamic Island,
  notification actions, the device token in the Keychain.
- **Android (Kotlin):** a background service that keeps the connection open over
  Tailscale, ongoing notifications with Approve and Deny, the device token in the
  Keystore.
- **The Mac daemon** sends push messages itself, directly to APNs. Muster still runs no
  service of its own.

Alternative: fully native apps (SwiftUI and Kotlin, no shared UI). Slightly more native,
but three UIs to maintain and about twice the work. Worth it only if the Tauri web view
feels wrong in use.

### Offline

Each app keeps the last board, card states and recent chat or terminal lines on the
device. Without a connection it shows them read-only, with the time of the last sync.
Actions are not queued: an approval delivered hours later, to a session that has moved
on, is dangerous.

### Daemon changes

- Register a paired device for push: platform, push token, Live Activity token.
- Send APNs requests (alerts, notification actions, Live Activity updates) when a card
  needs you or finishes. Needs the account's `.p8` key, stored on the Mac only.
- Per-device notification preferences.
- An approval answered from a notification goes through the same `approval.answer` path,
  with the device's token.

## Phases and effort

Focused working days. Testing on the real devices is the main limit: simulators can be
driven automatically, the phone and tablet are checked by hand.

| Phase | Work | Effort |
|---|---|---|
| 0. Trial builds | Tauri iOS and Android builds running; the app reaches the Mac over Tailscale; a Live Activity proof of concept in the simulator | 1–2 days |
| 1. Shared mobile base | Pairing by scanning the QR code in the app, secure token storage, offline copy of the board, reconnecting, push registration in the daemon | 3–5 days |
| 2. Android tablet | Touch-friendly board, terminal key row, hardware keyboard, background connection, notifications with Approve and Deny | 4–6 days (+2 for a home-screen widget) |
| 3. iPhone | Phone layout (cards by status, one card full screen), APNs from the daemon, notification actions, Live Activity and Dynamic Island | 7–10 days |
| 4. Hardening | Battery and background behaviour, signing and build scripts, tests, docs | 3–5 days |

Total: about 3½ to 5 weeks. Phases 0 to 2 give a native Android tablet app with
notifications in under two weeks, independent of the Apple account decision.

**Order:** Android tablet first (no account needed, proves the shared base), then iPhone.

## Limits to expect

- A Live Activity lasts at most 8 hours; Muster has to start a new one for longer work.
- Samsung's "live notification" style depends on the tablet's One UI version; ordinary
  ongoing notifications work everywhere.
- Both devices need the Tailscale app connected.
- A background connection on the tablet costs some battery; it can be limited to while
  charging or while a card is running.
