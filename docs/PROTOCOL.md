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
| `project.add` | `{path, name?, color?}` | `{project}` (also creates the default columns: To Do, In Progress, Review, Done) |
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
Card     {..., needsYouReason?, pendingApproval?, context?, titleAuto, setup?, model?}
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
- `model` is a short name of the model the agent last reported ("Opus 5.5", "gpt-5.5",
  "big-pickle"): Claude Code's status line and `SessionStart` hook, Codex's per-turn
  `turn_context`, Cursor's `sessionStart` hook, opencode's session. Absent until reported
  (always for Gemini and Antigravity, which report nothing); kept after the session ends.

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
| `usage.get` | `{}` | `{usage?: Usage, accounts: Usage[]}` |

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
- `usage.changed` is sent when one agent's numbers change; `usage.agentId` says which.
  Limits are account-wide and only reported while some agent session is active;
  `usage.get` returns the last known values (kept across daemon restarts) in `accounts`,
  one entry per agent, and in `usage` Claude Code's entry (or the newest) for older
  clients. `updatedAt` is when the agent last reported.
- Reports from every card of an agent are merged, not replaced: within one window (same
  `resetsAt`, give or take 30 minutes) the larger `usedPercent` stands, a later `resetsAt`
  starts a new window, and a report that leaves a window out keeps the known one. This
  matters for Claude Code, whose status line repeats the limits of that session's own last
  API response: an idle session would otherwise roll the numbers back. A window whose
  `resetsAt` has passed means nothing is used in the next one yet; clients show it as 0%.

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
  plugin with the `muster` skill under `$MUSTER_HOME/agents/plugin/`, and the adapter passes
  both on launch and resume (Claude Code: `--append-system-prompt-file` and `--plugin-dir`).

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

### Agent accounts

| Method | Params | Result |
|---|---|---|
| `agent.accounts` | `{refresh?: bool}` | `{accounts: AgentAccount[]}` |

```
AgentAccount {agentId, installed, signedIn?, method?, detail?, defaultModel?, usageNote?, checkedAt}
```

- One entry per agent CLI (not the shell). `signedIn` is absent when Muster cannot tell
  (Antigravity, Copilot, Grok). `method` is how it is signed in ("Claude account", "API
  key", "ChatGPT", "Cursor account", "Google account", "opencode account"); `detail` is
  extra text such as opencode's providers or "free models only"; `defaultModel` is the
  model the CLI's own settings start with; `usageNote` says where limits come from or why
  there are none.
- Read from local state only: Claude Code's `.claude.json` (`oauthAccount`) and
  `.credentials.json` presence, `codex login status`, cursor-agent's `cli-config.json`
  (`authInfo`), Gemini CLI's settings and `oauth_creds.json` presence, the provider ids
  in opencode's database. No credential value is read out and nothing goes over the
  network. Answers are cached for a minute; `refresh` checks again.

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
| `daemon.status` | `{}` | `{version, commit?, commitDate?, pid, home, tmuxSocket, hooksAvailable, notices: Notice[]}` |

`commit` is the short git commit the daemon was built from, with `+` appended when it was built with uncommitted changes; `commitDate` is that commit's date (ISO 8601).


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
  - `card.move` and `card.handoff` of itself; `card.update` of its own `notes` only;
  - `comment.list` and `comment.add` on itself or a linked card (round 6).

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

## Comments and notes authorship (round 6)

A card has a list of comments, written by the user and by agents. The daemon decides who
the author is; a client never does.

| Method | Params | Result |
|---|---|---|
| `comment.list` | `{cardId}` | `{comments: Comment[]}`, oldest first |
| `comment.add` | `{cardId, text, fromCardId?}` | `{comment}` |
| `comment.delete` | `{id}` | `{}` |

| Event | Payload |
|---|---|
| `comment.changed` | `{comment}` (added) or `{removedId, cardId}` |

```
Comment {id, cardId, author: Author, text, createdAt}
Author  {kind: "user"|"agent", cardId?, agentId?, cardTitle?}
Card    {..., commentCount, notesEditedBy?: Author, notesEditedAt?}
```

- **Author.** With the master token the author is the user, unless `fromCardId` names a
  card (the CLI does this when `MUSTER_CARD_ID` is set). With a card token the daemon
  sets `fromCardId` to the token's card whatever the caller sent, so an agent always
  signs as itself. An agent author carries its card id, that card's agent id and the
  card's title at the time of writing.
- **Card tokens** may list and add comments on their own card and on linked cards
  (`unauthorized` otherwise) and may not delete comments.
- **Text.** Control characters other than newline and tab are removed (agents read
  comments in a terminal). Empty text and text over 8000 characters are `invalid`. An
  agent may add at most 20 comments a minute to the same card (`unavailable` beyond).
  A card keeps its newest 500 comments.
- **Unread.** A comment by an agent sets the card's `unread`, so it shows on the board
  and in the tracker; `card.markRead` clears it. A comment by the user does not.
- `comment.add` and `comment.delete` are followed by `card.changed` for the card
  (`commentCount`, `unread`). Deleting a card deletes its comments.
- **Notes.** When `card.update` changes `notes`, the daemon records `notesEditedBy` and
  `notesEditedAt` with the same author rule (`card.update` accepts `fromCardId` for
  this; a card token's is always its own card). An unchanged value records nothing.
- `context.read` accepts `kind: "comments"`: the card's comments as text, each headed by
  its time (UTC) and author, e.g. `agent claude (card ab12cd "Fix login")` or `user`.
  The summary kind mentions the comment count.

## Round 8: stopping idle sessions

`Settings` gains `idleStopMinutes` (default `0`, never; at most 525600), set with
`settings.update`. `Card` gains `keepRunning?: bool`, `idleStoppedAt?: ms` and
`idleKept?: string`; `card.update` accepts `keepRunning`.

With a non-zero setting the daemon looks at the sessions a minute after it starts and
then every five minutes, and stops
one when all of this holds for the whole period:

- its agent can resume a conversation and the card has an `agentSessionId` (shell cards
  are never stopped);
- the card is `idle` or `done`: not working, not waiting for the user;
- no client is attached to it;
- the agent has reported nothing (`statusSince`, `lastActivityAt`), and no client has
  attached to, detached from or written to the session. Terminal output alone does not
  count: a terminal repaints for reasons that are nobody's use of it;
- the card's `keepRunning` is not set.

Before stopping, the daemon checks for work that would end with the session. If it finds
any, or cannot tell, the session stays and `idleKept` says why in words for the user:

- a process started more than two minutes after the session itself and still running
  (processes an agent starts with itself, such as MCP servers, do not count);
- for Claude Code, a job made with `CronCreate` and not deleted or expired, or a
  `ScheduleWakeup` that has not come due. These live only in the session's memory, so
  they are read from the transcript, counting only calls made since the running
  process started;
- for Claude Code, a non-empty `.claude/scheduled_tasks.json` in the card's folder when
  no other live Claude session shares that folder.

A stopped session is asked to quit (Ctrl+C twice) and then removed. The card becomes
`exited` with `idleStoppedAt` set, keeps its `agentSessionId`, and is not `restorable`;
`card.start` resumes the conversation and clears `idleStoppedAt`. `idleKept` is cleared
when the card is no longer idle, when it is stopped, and when `keepRunning` is set.

## Round 9: project documents

For the Docs tab. The methods are for the user's client only; a card's connection is
refused.

| Method | Params | Result |
|---|---|---|
| `docs.list` | `{projectId}` | `{docs: DocEntry[], truncated?: bool}` |
| `docs.read` | `{projectId, path}` | `{path, content, modifiedAt, truncated?: bool}` |
| `docs.write` | `{projectId, path, content, create?: bool, expectedModifiedAt?: ms}` | `{doc: DocEntry}` |

```
DocEntry {path, kind, size, modifiedAt}
           kind: "claude" | "agents" | "doc"
```

`docs.list` reads the project's folder when asked; nothing is watched or cached. It
lists Markdown files (`.md`, `.markdown`):

- at the top of the project, where `CLAUDE.md` and `CLAUDE.local.md` are `claude`,
  `AGENTS.md` is `agents`, and everything else is `doc`;
- `.claude/CLAUDE.md` and `.claude/CLAUDE.local.md`, as `claude`;
- under `docs/` (or `doc/`), five folders deep, skipping hidden entries and linked
  folders.

`path` is relative to the project's folder with `/` separators. The order is instruction
files, then the README, then the rest by path. At most 300 files are listed;
`truncated` says there were more.

`docs.read` takes a `path` from the list. It returns `invalid` for a path that is
absolute, contains `.` or `..`, or does not name a Markdown file, and `not_found` for a
file that does not exist or that a link takes outside the project's folder. At most
512 KB is returned, as UTF-8 with invalid bytes replaced; `truncated` says the file is
longer.

`docs.write` saves a file of at most 512 KB. Its `path` must be one the list would
show: a Markdown file at the top of the project, `CLAUDE.md` or `CLAUDE.local.md` in
`.claude/`, or a file under `docs/` (or `doc/`) no deeper than the list reaches, with no
hidden folder on the way; anything else is `invalid`.

- With `create: true` the file must not exist (`conflict` if it does). Missing folders
  are made, unless a folder on the way is a link that leaves the project.
- Otherwise the file must exist (`not_found`). With `expectedModifiedAt` (the
  `modifiedAt` from `docs.read`) the save is refused with `conflict` when the file has
  changed since, so that an agent's edit is not overwritten unseen. The text is written
  beside the file and moved into place; a link to another file of the project is
  written through and stays a link.

## Round 10: raw terminal input

`term.write` accepts `raw?: bool`. Input normally goes through the session's terminal
client, which reads it as keys and re-encodes them for the program. A key the program's
terminal type cannot express is then delivered as its plain equivalent: Ctrl+Enter sent
as `ESC [ 13 ; 5 u` arrives as Enter. With `raw: true` the daemon hands the bytes to the
program unchanged. The app uses it for Ctrl+Enter in Claude Code cards ("send now").

## Round 11: renaming a card with AI

| Method | Params | Result |
|---|---|---|
| `card.aiRename` | `{id}` | `{card}` |

Asks the card's agent to name its own session from the conversation so far, and takes
that name as the card's title. It works for agents whose `capabilities.aiRename` is true
(`agent.list`); today that is Claude Code, whose `/rename` command with no argument
generates a name.

The daemon sets `titleAuto` to true (also on a card the user had named), types the
command and Enter into the session, and returns the card at once. Claude Code does not
refresh its status line after `/rename`, so the daemon watches the transcript for the
`custom-title` line the command appends (for up to 20 seconds) and then sets the title,
announced with `card.changed`. Generated names are kebab-case (`fix-login-redirect`);
such session names, here and from the status line, are shown as words
("Fix login redirect").

- `invalid`: the agent has no rename command (other agents, shell cards).
- `conflict`: the card has no live session, is not between turns (status other than
  `idle` or `done`), or has no conversation yet. Nothing is ever typed mid-turn.
- A request for a card whose rename is still under way types nothing and returns the
  card.
- A title set with `card.update` afterwards stays, as before (`titleAuto: false`).
- The method is for the user's client only; a card's connection is refused.

## Round 12: project summary

For the Summary tab and `muster summary`: what was worked on in a project during a
time range, built from the activity log. The numbers involve no model.

| Method | Params | Result |
|---|---|---|
| `project.summary` | `{projectId, from, to, splits?: ms[]}` | `ProjectSummary` |

```
ProjectSummary {from, to, cards: SummaryCard[], totals, days?: SummaryDay[], active: SummaryActive[]}
SummaryCard    {id, title, agentId, model?, columnId, columnTitle, status, hasSession,
                firstAt, lastAt, workingMs, runs, outcome, sessionEnd?, created?, summary?}
                outcome:    "working" | "needs_you" | "finished" | "idle" | "not_started"
                sessionEnd: "exited" | "idle_stopped"
SummaryTotals  {cards, workingMs, finished, created}
SummaryDay     {from, to, cards: SummaryCard[], totals}
SummaryActive  {id, title, agentId, model?, columnTitle, status, statusSince, hasSession}
```

`from` is inclusive and `to` exclusive, in unix ms; `to` must be greater (`invalid`
otherwise). The client chooses the bounds, so "today" is the client's local day. A
range that reaches into the future is cut at the present.

**What the log holds.** One row when a card is created, one for every status change
(`from` and `to`), one when a card changes column, and one (`idle_stop`) when the daemon
stops an idle session. Nothing else: no prompts, no tool calls, no titles. The newest
1000 rows of each card are kept. Rows go when their card is deleted, so a deleted card
is not in any summary.

**Working time** is the time a card spent `running` inside the range, taken from
consecutive status rows. A card that was already running when the range begins counts
from `from`; one still running at its end counts up to `to` (or the present). The daemon
reads each card's last row before `from` to know the state it started in. What `running`
means depends on the agent: a turn for agents with hooks or a session log, a burst of
terminal output for the others (see `statusSource`). A session start counts as running
until the agent first reports idle.

**Runs** are the stretches of `running` that overlap the range. Going to `needs_you`
and back continues the same run.

**A card is listed** when it had a run in the range or was created in it. Reading a
finished card, moving it, or its session ending is not work and does not list it.
`firstAt` and `lastAt` are the first and last moment of running (or the creation)
inside the range. `title`, `model`, `columnTitle`, `status` and `hasSession` are the
card's values now. Cards are ordered by `lastAt`, newest first.

**`outcome`** is how the last run stood at the end of the range: `working` (still
running), `needs_you`, `finished` (the agent reported the turn done), `idle` (the run
stopped otherwise: interrupted, or an agent that only reports output and silence), or
`not_started` (created in the range, never run). **`sessionEnd`** is present when the
session ended inside the range after that run: `idle_stopped` when the daemon stopped it
for being idle, `exited` otherwise. `summary` is a written account of the work, present
once it has been asked for (Round 13).

**`totals`**: `cards` and `workingMs` count cards with at least one run; `finished`
those whose `outcome` is `finished`; `created` the cards created in the range.

**`splits`** (at most 366 times strictly inside the range, any order) cut the range into
parts, and `days` then holds one entry per part that has cards, newest first, each
summarised as a range of its own. The app sends its local midnights. A run that crosses
a split counts once in each part. Without `splits`, `days` is absent.

**`active`** lists the project's cards that are `running` or `needs_you` now, whatever
the range: waiting cards first, the longest wait on top.

A card's connection is refused (`unauthorized`): the summary names every card of the
project, and a card may only see itself and the cards linked to it.

## Round 13: written summaries

A few sentences per card and a paragraph per project for a range, written by a small
model through the user's own Claude Code. Nothing is written until a client asks.

| Method | Params | Result |
|---|---|---|
| `project.summarize` | `{projectId, from, to}` | `{started: bool}` |

| Event | Payload |
|---|---|
| `summary.changed` | `{projectId, from, to, done, total, running, error?}` |

`project.summary` (Round 12) returns what has been written for exactly the range asked
for:

```
SummaryCard    + summary?, summaryAt?, summaryStale?, summaryNone?
ProjectSummary + text?, textAt?, textStale?, summarizing?: {done, total}
```

- `summary` and `summaryAt` are on the cards of the whole range (`cards`), not on those
  inside `days`. `summaryNone` says the card had no conversation in the range that
  could be read (a shell card, an agent without a readable transcript, a session that
  said nothing); `summaryAt` is then when that was found.
- `summaryStale`: the card ran after its conversation was last read for the text. This
  comes from the activity log (`lastAt` against the time of the reading), so reading a
  summary never opens a transcript.
- `text` is the paragraph. `textStale`: a card with a text is stale or has a newer text
  than the paragraph, or a card worked on has none yet.
- `summarizing` is present while a job for this range runs.

**`project.summarize`** starts a job and answers at once. `started` is `false` when a
job for the project is already running (one per project; the request is then a no-op).
Errors: `invalid` for an empty range, `not_found` for the project, `unavailable` when
Claude Code is not installed. For every card with a run in the range, the job:

1. reads the card's conversation inside the range from the agent's transcript: the
   user's prompts and the assistant's text, without tool calls, tool output or
   reasoning. Agents whose transcript has no times (Cursor) are taken whole. No
   transcript, or nothing in the range, stores `summaryNone`.
2. keeps the stored text when the newest message is the one it was written from;
3. otherwise sends an excerpt of at most about 12,000 characters (when the conversation
   is longer: the first prompt and the newest messages) and stores the answer as one
   plain line of at most 600 characters.

Then it writes the paragraph (at most 900 characters) from the card texts, if any text
is newer than the paragraph. Two requests run at a time. `summary.changed` is sent when
the job starts (`done: 0`), after each card (`done` counts cards looked at, rewritten or
not) and at the end (`running: false`); clients read the texts with `project.summary`.
The first failed request ends the job: its last event carries `error` (Claude Code not
signed in, no answer within 60 seconds, ...), and what was written before stays.

Each text is one process:

```
claude -p --model haiku --safe-mode --tools "" --strict-mcp-config
       --no-session-persistence --disable-slash-commands
       --system-prompt <fixed instructions> --output-format json
```

with the excerpt on stdin, in an empty folder of its own (`<home>/summarize`), and
without the `MUSTER_*` and agent-session variables of the daemon's environment. It
loads no project instructions, settings, hooks, skills, plugins or MCP servers, has no
tools and saves no session. The instructions say that the excerpt is material to
describe, not instructions; a closing tag inside the excerpt is defused. The answer is
stripped of control characters before it is stored.

The texts are kept per card and range (and per project and range for the paragraph),
the newest 12 ranges each, and go with their card or project.

Both the method and the texts are for the user's client: a card's connection is refused.

## Round 14: remote access

Phones, tablets and other computers reach the daemon through `tailscale serve`. Code:
`crates/musterd/src/remote.rs`.

| Method | Params | Result |
|---|---|---|
| `remote.status` | `{}` | `RemoteStatus` |
| `remote.set` | `{enabled}` | `RemoteStatus` |
| `remote.pair` | `{}` | `{url, expiresAt}` |
| `remote.revoke` | `{id}` | `RemoteStatus` |

| Event | Payload |
|---|---|
| `remote.changed` | `RemoteStatus`, sent when it is switched, a device pairs or one is revoked |

```
RemoteStatus {enabled, port, httpsPort, url?, webUi, error?, devices: Device[],
              tailscale: {found, running, dnsName?, https}}
Device {id, name, createdAt, lastSeenAt?}
```

- **Switching on** starts a second listener on `127.0.0.1:<port>` (default 7318) and runs
  `tailscale serve --bg --https=<httpsPort> http://127.0.0.1:<port>` (default 8443).
  Devices open `url`, `https://<tailnet name>:<httpsPort>/`. A Tailscale problem (not
  installed, logged out, HTTPS certificates off for the tailnet, Serve not enabled) does
  not fail the call; it is reported in `error`. The setting survives restarts. Switching
  off stops the listener, closes device connections and runs
  `tailscale serve --https=<httpsPort> off`.
- **The remote listener** serves the built front end (`Contents/Resources/web` in the app
  bundle, `MUSTER_WEB_DIR`, or `app/dist` next to a development build) with `GET`,
  `POST /pair`, and the WebSocket at `/ws`. A WebSocket upgrade and `/pair` need the
  `Origin` to be exactly the tailnet page's origin. `hello` there accepts device tokens
  only, never the master or a card token, and device tokens do not work on the local
  port.
- **Pairing.** `remote.pair` returns `<url>#pair=<code>`, valid for 10 minutes and for one
  use. The page posts `{code, name}` to `/pair` and gets `{token, deviceId}`. The token
  is 32 random bytes, stored hashed. Failed codes count toward the same lockout as bad
  `hello` tokens.
- **A device connection** may call everything the app can, except `remote.set`,
  `remote.pair` and `remote.revoke` (`unauthorized`): remote access is managed on the
  Mac. Its token is checked again on every call. `remote.revoke` closes the device's
  open connections at once.
- Only the user's own clients (master token) may call `remote.*`; card connections are
  refused.

## Round 15: the last reply

Cards gain `lastMessage?: string`: the start of the agent's reply from when its latest
turn ended, on one line, without markdown marks, cut at a word after about 240
characters. It is kept until the next turn ends. Claude Code and Codex report the reply
with the turn's end; for other agents it is read from the transcript. The menu-bar
item uses it for the body of a "Finished" notification.

## Round 16: a card's changes

For the card's Changes tab. The methods are for the user's client only; a card's
connection is refused. Both read git when asked; nothing is watched or cached.

| Method | Params | Result |
|---|---|---|
| `card.changes` | `{id, base?: string, scope?: Scope}` | `CardChanges` |
| `card.diff` | `{id, path, base?: string, scope?: Scope}` | `{path, patch, truncated?: bool}` |

```
Scope: "all" (default) | "committed" | "uncommitted"
CardChanges {root, branch?, base, defaultBase, branches: string[], scope, baseCommit, commits,
             uncommitted, files: ChangedFile[], truncated?: bool}
ChangedFile {path, oldPath?, kind, additions, deletions, binary?: bool}
             kind: "added" | "modified" | "deleted" | "renamed" | "untracked"
```

The changes are those of the repository the card's folder is in, measured from where
the card's branch left its base, and include uncommitted and untracked files. The base
is the branch checked out in the project's folder when that is the same repository,
else `origin/HEAD`, else the first of `main`, `master`, `trunk`, `develop` that exists.
`baseCommit` is the merge base of the base and `HEAD`. When the card works on the base
branch itself, or no base is found, `base` is `"HEAD"` and only uncommitted changes are
listed. `defaultBase` is that automatic choice.

`base` picks another branch to compare against: a local branch, or a remote one such as
`origin/main` (`invalid` when there is no such branch or it shares no history with
`HEAD`). `branches` lists the local branches, most recently changed first (at most 200),
with the automatic base added when it is a remote branch.

`scope` picks which changes: `all` compares `baseCommit` with the files on disk,
`committed` compares `baseCommit` with `HEAD`, and `uncommitted` compares `HEAD` with the
files on disk. Untracked files are in `all` and `uncommitted`. `commits` counts the
commits since `baseCommit`. Paths are relative to `root`,
the repository's top folder. At most 500 files are listed; `truncated` says there were
more. A folder that is not in a git repository is `invalid`.

`card.diff` returns the unified diff of one file, as `git diff` prints it. `path` must
be one `card.changes` lists with the same `base` and `scope` (`not_found` otherwise). At most 512 KB is returned, cut at
a line; `truncated` says there was more. Git runs with external diff, text conversion
and fsmonitor off.

Review comments are sent with `message.send`; there is no separate method.
