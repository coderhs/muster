# Muster daemon protocol (v1)

One WebSocket per client at `ws://127.0.0.1:<port>/ws`. The port and token are found in
`$MUSTER_HOME/daemon.json` (`{"pid":..,"port":..}`) and `$MUSTER_HOME/token`.

`crates/muster-proto` is the source of truth; keep this file in step with it. Field names
are camelCase on the wire. Timestamps are unix milliseconds. Ids are opaque strings.

## Frames

Text frames are JSON:

```
request   {"t":"req","id":<u64>,"m":"<method>","p":{...}}
response  {"t":"res","id":<u64>,"ok":true,"r":{...}}
          {"t":"res","id":<u64>,"ok":false,"e":{"code":"<code>","msg":"..."}}
event     {"t":"ev","m":"<event>","p":{...}}
```

Binary frames carry terminal output only:

```
0x01 | u16 big-endian length of sid | sid (utf-8) | raw terminal bytes
```

`sid` is the card id. Error codes: `unauthorized`, `not_found`, `invalid`, `conflict`,
`unavailable`, `internal`.

The first request must be `hello`. Anything else before it gets `unauthorized` and the
socket is closed.

## Types

```
Project  {id, name, path, color, position}
Column   {id, projectId, title, color, category, position}
           category: "unstarted" | "started" | "done" | "closed"
Card     {id, projectId, columnId, rank, title, agentId, cwd, branch?, prompt?, notes?,
          priority?, dueAt?, agentSessionId?, status, statusSince, turnStartedAt?,
          lastActivityAt?, unread, hasSession, createdAt, updatedAt}
           status:   "draft" | "running" | "needs_you" | "done" | "idle" | "exited"
           priority: "low" | "medium" | "high" | "urgent"
Agent    {id, label, color, installed}
           id: "claude" | "codex" | "cursor" | "opencode" | "antigravity" | "gemini" |
               "copilot" | "grok" | "shell" | "custom:<id>"
```

`agentId: "shell"` is a plain login shell with no agent.

## Methods

| Method | Params | Result |
|---|---|---|
| `hello` | `{token, client: "desktop"\|"mobile"\|"cli", protocol: 1}` | `{protocol: 1, daemonVersion}` |
| `project.list` | `{}` | `{projects: Project[]}` |
| `project.add` | `{path, name?, color?}` | `{project}` (also creates the three default columns) |
| `project.update` | `{id, name?, color?, position?}` | `{project}` |
| `project.remove` | `{id}` | `{}` (refused with `conflict` while any card has a live session) |
| `board.get` | `{projectId}` | `{project, columns: Column[], cards: Card[]}` |
| `column.create` | `{projectId, title, color?, category?}` | `{column}` |
| `column.update` | `{id, title?, color?, category?, position?}` | `{column}` |
| `column.delete` | `{id, moveCardsTo}` | `{}` |
| `card.create` | `{projectId, columnId, title?, agentId, cwd?, prompt?, start?: bool, worktree?: {branch}, resumeSessionId?}` | `{card}` |
| `card.update` | `{id, title?, prompt?, notes?, priority?, dueAt?}` | `{card}` |
| `card.move` | `{id, columnId, before?: cardId, after?: cardId}` | `{card}` (daemon computes `rank`) |
| `card.delete` | `{id, killSession?: bool}` | `{}` |
| `card.start` | `{id}` | `{card}` (launches, or resumes if `agentSessionId` is set) |
| `card.markRead` | `{id}` | `{card}` |
| `session.attach` | `{cardId, cols, rows, watch?: bool}` | `{cols, rows}` then output arrives as binary frames, starting with a full repaint |
| `session.detach` | `{cardId}` | `{}` |
| `session.kill` | `{cardId}` | `{}` |
| `term.write` | `{cardId, data}` (utf-8 string) | `{}` |
| `term.resize` | `{cardId, cols, rows}` | `{cols, rows}` (the effective size) |
| `agent.list` | `{}` | `{agents: Agent[]}` |

Rules:

- `card.create` without `cwd` uses the project path. With `worktree`, the daemon creates a
  git worktree for that branch beside the project and uses it as `cwd`.
- `card.create` with `start: false` (the default) makes a draft card.
- `card.create` with `resumeSessionId` stores it as the card's `agentSessionId`, so the
  card resumes that existing agent session when started. It cannot be combined with
  `worktree`; `cwd` must be the folder the session was created in.
- A session's size is the smallest size among attached clients that are not `watch`.
  `watch` clients never affect the size.
- After `session.attach`, the first bytes a client receives must reproduce the current
  screen, so a client that starts from an empty terminal ends up correct.
- `term.write` is never retried by the daemon; a failure is reported to the caller.
- Moving a card between columns never starts or stops a session.

Clarifications (behaviour of the current daemon; none of these change a shape above):

- Optional fields that are unset are omitted from objects, never sent as `null`.
- `card.move`: `before: X` places the card immediately above card `X`; `after: Y` places it
  immediately below card `Y`. Both together must name two adjacent cards (`Y` then `X`).
  With neither, the card goes to the end of the column. Referenced cards must be in the
  target column (`invalid` otherwise).
- `card.update`: an absent key leaves a field unchanged; an explicit `null` clears
  `prompt`, `notes`, `priority` or `dueAt`.
- `card.delete` on a card with a live session is refused with `conflict` unless
  `killSession: true`.
- `card.start` on a card whose session is already live returns the card unchanged.
- `session.attach` on a card without a live session fails with `not_found`. Its response
  is always delivered before the first binary frame for that attach. Attaching again from
  the same connection replaces that connection's earlier subscription (and size).
- When only `watch` clients are attached, the session keeps its current size.
- `term.resize` requires the caller to be attached to that session (`invalid` otherwise).
- `term.write` to a session nobody is attached to is delivered directly to the session.
- A `session.size` event is sent whenever the effective size changes, including when the
  first client attaches.
- An unknown method gets `not_found`; malformed params get `invalid`.
- Binary frames sent by a client are ignored.
- A client that stops reading is disconnected once about 1024 frames are queued for it,
  rather than having terminal output silently dropped; reconnect and re-attach.

## Events

Sent to every client that has completed `hello`.

| Event | Payload |
|---|---|
| `project.changed` | `{project}` or `{removedId}` |
| `column.changed` | `{column}` or `{removedId, projectId}` |
| `card.changed` | `{card}` or `{removedId, projectId}` |
| `session.size` | `{cardId, cols, rows}` |
| `session.exit` | `{cardId, exitCode?}` |

Status changes arrive as `card.changed`. Later phases add methods and events (chat pages,
approvals, usage, resource stats, links); additions must be backwards compatible.

## Phase 3: agent status, approvals, chat, usage

Additive to v1: `protocol` stays `1`. Clients that ignore these fields and events keep
working; unknown events must be ignored.

### Card fields

```
Project  {..., worktreeSetup?}
Card     {..., needsYouReason?, pendingApproval?, context?, titleAuto, setup?}
PendingApproval {requestId?, tool, summary, since}
ContextInfo     {usedTokens, windowTokens?, percent?}       percent: 0..100
SetupInfo       {state: "running"|"failed", exitCode?}
```

- Status, `statusSince`, `turnStartedAt`, `lastActivityAt`, `unread` and `agentSessionId`
  are driven by the agent's hooks for agents that have them (Claude Code today). For
  other agents and shell cards `lastActivityAt` follows terminal output (refreshed on
  `board.get`, and at most every 10 s while someone is attached).
- `needsYouReason` is present only while `status` is `needs_you`:
  `permission` (see `pendingApproval`), `question`, `input`, `notification`,
  `startup_prompt` (the agent was launched but has reported nothing yet, e.g. Claude
  Code's folder-trust dialog is on screen), `terminal_prompt` (a prompt is open in the
  terminal that only the terminal can answer) or `setup_failed` (see worktree setup).
- `pendingApproval` is present only while a permission request waits. `requestId` is set
  when the request can be answered through the hook; when absent (the hook went away, or
  after a daemon restart) only the terminal-key fallback can answer it. `summary` is one
  line, e.g. `Bash: touch x.txt`; `since` is unix ms.
- `agentSessionId` is chosen by Muster at launch and can change while the agent runs (e.g.
  Claude Code's `/clear`); `card.start` on a card that has one resumes that session.

### Automatic titles

- `titleAuto` is `true` when the card was created without a `title`. While it is true the
  daemon maintains the title: at creation it is the cleaned first line of `prompt` if one
  was given, else a placeholder (the agent's label, or the cwd's folder name for shell
  cards); on the first user prompt of the session it becomes that prompt (first non-empty
  line, whitespace collapsed, at most 60 characters plus `…`; slash commands are
  skipped); when the agent reports its own session name (Claude Code's generated title)
  that wins. Changes arrive as `card.changed`.
- `card.update {title}` sets `titleAuto` to `false`; the daemon never overwrites a title
  the user set. `card.update {titleAuto: true}` hands the title back (it changes again on
  the next prompt or agent name). `card.update` params gain `titleAuto?: bool`.

### Worktree setup

- `project.update` params gain `worktreeSetup?: string | null` (`null` or `""` clears).
  The script is returned on `Project`.
- When a card created with `worktree` is first started (immediately with `start: true`,
  or by a later `card.start`) and its project has a script, the script runs inside the
  card's tmux session before the agent, so its output shows in the card's terminal. It
  is run by the user's login shell in the new worktree with `MUSTER_PROJECT_PATH` (the
  main checkout), `MUSTER_WORKTREE_PATH` and `MUSTER_BRANCH` set.
- While it runs: `status: "running"`, `setup: {state: "running"}` (show "setting up";
  the agent has not started). On exit 0 `setup` disappears and the agent starts. On
  failure the agent does not start, the session is left at a login shell in the worktree,
  and the card becomes `needs_you` with `needsYouReason: "setup_failed"` and
  `setup: {state: "failed", exitCode}`. `card.start` on such a card replaces the session
  and runs the setup again.

### Methods

| Method | Params | Result |
|---|---|---|
| `approval.answer` | `{cardId, requestId?, decision: "allow"\|"deny", remember?: bool, message?}` | `{via: "hook"\|"keys"}` |
| `chat.page` | `{cardId, before?, limit?}` (`limit` default 50, max 500) | `{messages: ChatMessage[], older?}` |
| `chat.unsubscribe` | `{cardId}` | `{}` |
| `usage.get` | `{}` | `{usage?: Usage}` |

```
ChatMessage {id, role: "user"|"assistant"|"tool", timestamp?, model?, parts: ChatPart[]}
ChatPart    {type: "text", text} | {type: "thinking"} | {type: "toolCall", id, name, summary}
          | {type: "toolResult", toolUseId, isError, summary} | {type: "image"}
          | {type: "interrupted"}
Usage       {fiveHour?: {usedPercent, resetsAt?}, sevenDay?: {usedPercent, resetsAt?},
             updatedAt}                       resetsAt and updatedAt: unix ms
```

- `approval.answer`: if `requestId` names a request still waiting on the hook, the
  decision goes through the hook (`via: "hook"`); `remember` applies the agent's own
  suggested "always allow" rules and `message` is shown to the agent on deny. Otherwise,
  if the card is `needs_you`, the agent's terminal keys are typed instead (`via: "keys"`;
  Claude Code: allow = `1`, deny = Escape, which also interrupts the turn; `remember` and
  `message` do not apply). Errors: `conflict` when the card is not waiting,
  `unavailable` when there is no reliable key for the decision (e.g. allow + remember
  without a hook request), `not_found` when the card has no live session.
- `chat.page` without `before` returns the newest messages, oldest first, and subscribes
  the connection to `chat.appended` for that card (until `chat.unsubscribe` or
  disconnect). Pass `older` as `before` to page backwards; absent `older` means the start
  of the conversation. A card without a transcript yet returns no messages (and still
  subscribes).

### Events

| Event | Payload | Sent to |
|---|---|---|
| `chat.appended` | `{cardId, messages: ChatMessage[]}` | connections subscribed by `chat.page` |
| `usage.changed` | `{usage: Usage}` | every client |

- `chat.appended` is pushed when the agent reports activity and the transcript has grown;
  no polling is needed. A message may arrive again with the same `id` when the agent
  extended it: replace the earlier copy. After a session id change the new conversation's
  messages follow.
- `usage.changed` is sent when the five-hour or seven-day numbers change. Limits are
  account-wide and only reported while some agent session is active; `usage.get` returns
  the last known values (kept across daemon restarts) with the time they were observed.

### Connection and other rules (settled for phase 3)

- **Origin**: the WebSocket upgrade is refused (HTTP 403) when an `Origin` header is
  present and is not `tauri://localhost`, `http://localhost[:port]` or
  `http://127.0.0.1[:port]`. Clients that send no `Origin` (the CLI) are accepted. Any web
  page can reach 127.0.0.1, so this check stands in front of the token.
- **Heartbeat**: the daemon sends a WebSocket ping every 30 s and closes a connection
  that has sent nothing (no pong, no frame) for a whole ping period after a ping. There
  is no application-level heartbeat; browsers and tungstenite answer pings themselves.
- **Terminal input**: `term.write` takes a UTF-8 string and that is sufficient. xterm.js
  only emits non-UTF-8 bytes (`onBinary`) for the legacy X10 mouse encoding; the tmux
  client in front of every session enables SGR mouse reporting (mode 1006), which is
  plain ASCII. No binary input field is defined.
- **Needs-you counts** per project are computed by clients from `card.changed`; there is
  no daemon method for them.
- **`card.markRead`** clears `unread`. On a `done` card it also moves the status to
  `idle`. It never changes a `needs_you` card's status.
- **`column.create`** without `category` creates an `unstarted` column.

## Phase 4 and 5: resources, keep-awake, restore, context sharing

Additive to v1 (`protocol` stays `1`).

### Card and board fields

```
Card    {..., restorable, queuedMessages?: Message[]}
Board   {project, columns, cards, links: Link[]}     (board.get result)
Link    {a, b}                                       undirected; a < b
Message {id, cardId, fromCardId?, text, createdAt, deliveredAt?}
Settings {keepAwake, autoRestore}                    defaults: true, false
ProcStats {memBytes, cpuPercent, processes}
```

- `restorable` is `true` when the card's session disappeared without exiting (machine
  reboot, tmux server died) rather than ending or being killed. `card.start` on such a
  card replays its saved scrollback into the new session's terminal, then resumes the
  agent with its stored `agentSessionId` (a shell card gets a new shell), and clears the
  flag.
- `queuedMessages` lists messages not yet delivered (absent when empty).

### Methods

| Method | Params | Result |
|---|---|---|
| `card.get` | `{id}` | `{card}` |
| `stats.subscribe` | `{intervalMs?}` (default 5000, minimum 2000) | `{intervalMs}` |
| `stats.unsubscribe` | `{}` | `{}` |
| `settings.get` | `{}` | `{settings}` |
| `settings.update` | `{keepAwake?, autoRestore?}` | `{settings}` |
| `link.add` | `{a, b}` (card ids) | `{}` |
| `link.remove` | `{a, b}` | `{}` |
| `context.list` | `{cardId}` | `{cards: Card[]}` (cards linked to `cardId`) |
| `context.read` | `{cardId, fromCardId?, kind: "transcript"\|"screen"\|"notes"\|"summary", limit?}` | `{text}` |
| `message.send` | `{cardId, fromCardId?, text}` | `{message}` |
| `message.cancel` | `{id}` | `{}` (`conflict` if already delivered) |
| `card.handoff` | `{id, agentId?}` | `{card}` |

### Events

| Event | Payload | Sent to |
|---|---|---|
| `stats.changed` | `{cards: [{cardId, memBytes, cpuPercent, processes}], total: ProcStats, daemon: ProcStats, at}` | subscribed connections |
| `settings.changed` | `{settings}` | every client |
| `link.changed` | `{link, linked: bool}` | every client |

### Rules

- **Stats** are sampled only while at least one connection is subscribed, at the
  smallest requested interval; a subscription ends with `stats.unsubscribe` or on
  disconnect. Per card: the session's whole process tree (the pane process, its
  descendants, and processes still in its session after re-parenting). `memBytes` is the
  macOS physical footprint (as Activity Monitor's "Memory"), `cpuPercent` is percent of
  one core since the previous sample (0 in the first), `total` sums all card sessions,
  `daemon` is musterd itself.
- **Keep-awake**: while any card is `running` and `keepAwake` is on, the daemon holds an
  idle-sleep assertion (the display may still sleep); it is released as soon as no card
  is running, when the setting is turned off, or on battery power below 10%.
- **Restore**: scrollback (the last 256 KB of history with colours) is saved under
  `$MUSTER_HOME/scrollback/` when the last viewer detaches, every minute for sessions with
  new output, and at shutdown. With `autoRestore` on, restorable cards are started
  automatically when the daemon starts.
- **Context**: with a card token (see "Security") the server reads as the token's card
  and only allows linked cards (or the card itself). With the master token,
  `fromCardId` is optional: when given, the cards must be linked (`invalid` otherwise);
  without it any card can be read. `transcript` is the newest `limit` messages (default 20, max 200) as
  plain text; `screen` is the current terminal screen (or the saved scrollback's tail when
  the session is gone); `summary` is title, agent, status, cwd, notes and the last user
  message and assistant reply. Text is capped at 64 KB.
- **Messages** are typed into the receiving session only when its card is `idle` or
  `done` and has a session; otherwise they wait in `queuedMessages` and the oldest is
  delivered at the next idle/done, one per turn. Delivered text starts with
  a header line `[Muster message from card <id> "<title>"; reply with: muster send <id>
  "..."]` (or `[Muster message from the user]`), then the text, all in one bracketed
  paste; see "Security" below for sanitising, size and rate limits. Cards without hook-driven status (shell and
  agents other than Claude Code) never become idle, so messages to them stay queued.
- **Handoff** creates a card in the same column and cwd (same branch), titled
  `Continue: <title>`, linked to the original, and starts it with a prompt that tells
  the agent to read the original through `muster context read`.
- Sessions get `MUSTER_HOME` and `MUSTER_CLI` (path of the `muster` CLI) in their
  environment, next to `MUSTER_CARD_ID`; the CLI uses `MUSTER_CARD_ID` as "me".
- Agents learn about the CLI per launch, without any write to their own config or the
  repo: the daemon generates `$MUSTER_HOME/agents/muster-instructions.md` (short) and a
  `muster` skill under `$MUSTER_HOME/agents/skill/`, and the adapter passes both on launch
  and resume (Claude Code: `--append-system-prompt-file` and `--add-dir`).

## Round 4: questions and plans, agent capabilities, coarse status, daemon status

Additive to v1 (`protocol` stays `1`).

### Questions and plans

```
PendingApproval {..., kind: "tool"|"question"|"plan", questions?: Question[], plan?, planFilePath?}
Question        {question, header?, options: [{label, description?}], multiSelect}
```

- `kind` is `tool` when absent (older data). `needsYouReason` is `question` or `plan` for
  those kinds.
- `approval.answer` params gain `answers?: {<question text>: [<label>...]}` and
  `acceptEdits?: bool`; `decision` gains `"answer"` (needs `answers`; several labels
  for multi-select; a value that is not a label is passed as free text) and
  `"approve_plan"` (with `acceptEdits` to continue in accept-edits mode). Sending a plan
  back is `decision: "deny"` with a `message`. Questions and plans have no terminal-key
  fallback: without a pending hook request they fail with `unavailable`.

### Agent capabilities

`agent.list` entries gain `capabilities?`:

```
{statusSource: "hooks"|"log"|"activity", approvals, questions, chat, context, usage, resume}
```

Hide controls an agent does not support. Today: Claude Code `hooks` (everything);
Cursor `hooks` (no approvals); Codex `log` (chat, context, usage, resume); opencode
`activity` (chat, context, resume); Gemini `activity` (resume); Antigravity `activity`
(resume not verified, so off); shell
and other commands `activity` (nothing else).

### Coarse status for cards without hooks

- Cards whose agent has no hook-driven status (shell, opencode, Gemini, Antigravity,
  other commands) are `running` while their terminal produces output and become `idle`
  after `silenceSeconds` of silence (a setting, default 5, 1..=600). `turnStartedAt`,
  `statusSince` and `lastActivityAt` follow. Hook-driven status always wins where an
  adapter provides it.
- Codex status comes from its session log, read when the terminal starts or stops
  producing output; Codex picks its own session id, which appears as `agentSessionId`
  shortly after launch.
- opencode is asked (`opencode api`) whether its session is busy at those edges, and,
  while a chat view is open (`chat.page` subscription), for new messages every few
  seconds.
- Queued messages are delivered when such a card is `idle`, except to `shell` cards
  (typing text into a shell would run it); those stay queued.

### Settings and usage

- `Settings` gains `silenceSeconds` (`settings.update {silenceSeconds}`; `invalid`
  outside 1..=600).
- `Usage` gains `windows?: [{name, windowMinutes?, usedPercent, resetsAt?}]` (every
  window the agent reported by its own name, e.g. Codex `primary`/`secondary`) and
  `agentId?` (which agent reported it). `fiveHour`/`sevenDay` are filled when the
  window sizes match.

### Daemon status

| Method | Params | Result |
|---|---|---|
| `daemon.status` | `{}` | `{version, pid, home, tmuxSocket, hooksAvailable, notices: Notice[]}` |

| Event | Payload |
|---|---|
| `daemon.status` | same as the method's result; sent when the notices change |

```
Notice {id, level: "warning"|"info", message}
```

Notice ids: `hook_socket` (hooks unavailable, e.g. `$MUSTER_HOME/hook.sock` is longer
than macOS's 104-byte limit or `muster-hook` is missing), `claude_permission_hooks` (the
user's own Claude Code settings define PermissionRequest hooks, so approvals from
Muster may be pre-empted by other tools; checked read-only at each Claude launch),
`tmux_missing` (no tmux binary; sessions cannot run), `hook_events_dropped` (hook events
rejected for a missing or wrong card secret).

## Security (round 5)

### Roles

`hello` accepts two kinds of token:

- the **master token** (`$MUSTER_HOME/token`): the app and the user's own CLI. Every
  method.
- a **card token**: generated for each session launch, given to the session only as
  `MUSTER_CARD_TOKEN` (with `MUSTER_DAEMON_FILE`, the path of `daemon.json`, so the CLI
  can find the port), stored by the daemon only as a SHA-256 hash, and revoked when the
  session ends. The caller's card is always the token's card, never a parameter. Allowed:
  - `context.list` (linked cards), `context.read` of linked cards or itself;
  - `message.send` to a linked card;
  - `card.get` of itself or a linked card; `board.get` of its own project, showing only
    itself and linked cards;
  - `card.move` and `card.handoff` of itself; `card.update` of its own `notes` only.

  Everything else (approvals, terminal and session methods, projects, columns, links,
  settings, stats, card creation and deletion, reading unlinked cards) is refused with
  `unauthorized`; the connection stays open. A card connection receives no events.
  Sessions no longer get `MUSTER_HOME`.

A card token is checked again on every call, so connections opened during a session
stop working when it ends.

This restrains well-behaved or prompt-injected agents with limited tools. It is not a
hard boundary: an agent that runs unsandboxed shell commands as the user can read the
master token file and the database (the CLI itself falls back to the master token at
the default `$MUSTER_HOME` when `MUSTER_CARD_TOKEN` is unset, and `MUSTER_DAEMON_FILE`
reveals where that is). A real boundary needs the agent's own sandbox to deny
`$MUSTER_HOME`.

### Other rules

- Hook events are accepted only with the per-launch hook secret of their card
  (`MUSTER_HOOK_SECRET` in the session; see docs/AGENTS.md), and transcript paths only
  when they resolve under the agent's own transcript directory. Rejected events show as
  a `hook_events_dropped` notice in `daemon.status`.
- Messages: control characters (including ESC and CR) are removed from the text and the
  sender's title, the text is limited to 8000 characters (`invalid` above), every
  delivery is one bracketed paste starting with the fixed header line, and each sender
  may send at most 10 messages a minute to the same card (`unavailable` beyond).
- Card processes run without `TMUX`/`TMUX_PANE`, so tmux commands inside a card cannot
  reach Muster's tmux server.
- `Settings.clipboardPassthrough` (default `false`): whether programs in cards may set
  the viewer's clipboard (OSC 52).
- Origins: `tauri://localhost` and the dev origins in `MUSTER_DEV_ORIGINS` (exact;
  default `http://localhost:1420` and `http://127.0.0.1:1420`). After 10 failed `hello`
  attempts within a minute, further bad attempts are answered slowly ("too many failed
  attempts") until the minute has passed; a valid token is always accepted.
- `card.handoff`: at most 3 per card per 10 minutes (`unavailable` beyond).
- Without tmux the daemon stays up, reports a `tmux_missing` notice and refuses session
  operations with `unavailable`.
- Before remote access (phase 8) the single master token must be replaced by per-device
  tokens that can be revoked; the loopback check means nothing behind `tailscale serve`.
