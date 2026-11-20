# Using Muster

## Projects and the board

A **project** is a folder on disk. Each project has its own board. A new project gets
four columns: To Do, In Progress, Review and Done. Review is for work an agent has
finished and you have not looked at yet. Columns can be added, renamed, recoloured
and reordered. Cards are moved by dragging. A project made before the Review column
existed keeps its columns; add one from the board if you want it.

Moving a card between columns never starts or stops its session. Columns describe
your work; the card's **status** describes the agent.

## Creating a card

Press the new-card button (or its shortcut) and pick an agent. Everything else is
optional:

| Field | What it does |
|---|---|
| Title | Leave empty and the card is named from the first prompt. |
| Prompt | Sent to the agent when it starts. Leave empty to type in the terminal. |
| Folder | Where the agent runs. Empty means the project folder. |
| New git worktree | Creates a worktree for a new branch and runs the agent there. |
| Resume an existing session | Continues an agent session by its id instead of starting a new one. |

**Start now** launches the agent. **Save draft** keeps the card without a session.

### Worktrees and the setup script

With "New git worktree" ticked, Muster creates a git worktree beside the project for
the branch you name. If the project has a **worktree setup script** (Project
settings), it runs in the new worktree before the agent starts, for example to install
dependencies or copy an env file. If the script fails, the card says so and offers to
run it again.

### Resuming a session

Tick "Resume an existing session" and paste the agent's session id. The Folder must
be the one the session was started in, because agents store sessions per folder. The
prompt is not sent when resuming.

## Card status

| Status | Meaning |
|---|---|
| Draft | Not started. |
| Running | The agent is working on a turn. |
| Needs you | The agent asked a question or wants an approval. |
| Done | The turn finished and you have not looked yet. |
| Idle | Finished and seen. |
| Exited | The session ended. |

## Inside a card

- **Terminal**: the agent's own interface. Only one terminal is live at a time, to
  keep memory low.
- **Chat**: the conversation as readable messages, for agents whose transcript Muster
  can read.
- **Changes**: what the agent changed in the card's folder, to review without leaving
  Muster. See [Reviewing changes](#reviewing-changes).
- **Approvals and questions**: for Claude Code, a permission request, a question or a
  plan can be answered from the card without opening the terminal.
- **Details**: column, priority, due date, notes and links. Links connect cards for
  [context sharing](Context-Sharing.md). The notes show who last edited them.
- **Comments**: the second tab of the same column. You and agents both write here, and
  every comment says who wrote it.
- The details column can be hidden with its button in the card's title bar or ⌘I,
  which gives the terminal the full width.
- **Rename with AI**, next to the title and in the card's right-click menu, asks
  Claude Code to name the session from the conversation so far (it types `/rename`
  into the session) and uses that name as the card's title. It is available on Claude
  Code cards with a running session that is between turns; type a title yourself at
  any time to keep your own.

## Looking at a file

When an agent names a file, such as `crates/muster-core/src/store.rs:932`, ⌘-click it
in the terminal, or click it in the Chat tab, to see the file at that line without
leaving Muster. Paths with `:line`, `:line:col`, `#L12` or `(line 12)` jump to the line;
paths are looked up in the card's folder first, then the project's.

The viewer has syntax colours, ⌘F to find in the file, and Esc to close. **Open in
editor** (⌘O) opens the file at the same line in the editor chosen in Settings →
Files (Cursor, VS Code, Zed, Sublime Text, Xcode, JetBrains IDEs and more, the default
app, or a command of your own with `{file}`, `{line}` and `{col}`). **Reveal in Finder**
and **Copy path** are next to it.

Only files in the card's folder and its project open, unless **Show any file on this
Mac** is switched on in Settings → Files. A paired phone or tablet can always only see
files in the card's folder and project, and has no editor button.

## Reviewing changes

The **Changes** tab of a card (next to Terminal and Chat; ⌃⇥ steps through them) lists
the files the card's folder changed and shows the diff of the one you pick.

- For a worktree card the changes are measured against the branch checked out in the
  project folder (usually `main`), from where the card's branch left it: the agent's
  commits plus anything not committed yet, including new files. Changes made on the
  base branch since then are not shown.
- For a card that works in the project folder on the base branch itself, only the
  uncommitted changes are shown.
- **Compare to** picks another branch to measure against; Automatic is the choice
  above. **All / Committed / Uncommitted** shows the commits and the uncommitted work
  together, only the branch's commits, or only what is not committed yet (new files
  included). Both choices are remembered per card.
- The header names the card's branch, how many commits it has, whether anything is
  uncommitted, and the total lines added and removed.
- An image (PNG, JPEG, GIF, WebP, AVIF, BMP, ICO or SVG) is shown as pictures, before and
  after side by side, with each one's size in pixels and bytes. An SVG also shows its
  text diff below. Pictures larger than 4 MB are not shown, only their size. The list refreshes when the tab opens, when the agent
  finishes a turn, and with the refresh button.

Click a line of the diff to comment on it (⌘↩ saves, Esc cancels); click a comment to
change or delete it. A note for the whole review can go in the box at the bottom.
**Send to agent** sends the note and every comment, each with its file, line and the
line's text, as one message. Like other messages it waits until the agent is between
turns. Unsent comments are kept, even when the card or the window is closed, until you
send or discard them. Shell cards have no agent to send a review to.

Known limitation: a comment's line number refers to the version you were looking at. In
**Committed** that is the committed version, which can differ from the file on disk
when there are uncommitted edits on top. Each comment also quotes the line's text, so
the agent can still find the spot. To be revisited if it causes trouble.

## Sidebar

The projects sidebar collapses to a narrow rail with the button at the left of the toolbar or ⌘B. The rail shows one tile per project in its colour, labelled with the first letter of its name, or the first two when another project starts with the same letter.
⌘1 to ⌘9 keep switching projects while it is hidden.

## Docs

The Docs tab shows the selected project's own documentation, read from its folder:

- **Agent instructions**: `CLAUDE.md` (also `.claude/CLAUDE.md` and `CLAUDE.local.md`)
  and `AGENTS.md`. A file the project does not have is listed as "not added"; choosing
  it shows what the file is for and how to write a good one. For a file that exists,
  the Tips button shows the same advice, and a note appears above the text when the
  file is long, empty, or when one agent would miss what the other file says.
- **Documents**: Markdown files at the top of the project and under `docs/`, shown as
  folders that open and close.

Links between documents open in the tab; links to the web open in the browser. The
files are read when the tab opens and when you press Refresh.

**Edit** opens the file's text; **Preview** shows how it will read, and **Save** (⌘S)
writes it. Text you have not saved is kept while you look at other documents, tabs or
projects, and the file is marked with a dot in the list. If an agent changed the file
after you opened it, the save is refused: press Refresh to see the new text; your own
is still there to save over it or discard.

The **+** beside Documents makes a new file at the top of the project, and the **+**
on a folder makes one inside it. A name may include folders (`guides/Setup`); `.md` is
added for you. A missing `CLAUDE.md` or `AGENTS.md` is made from its tips page.

## Summary

The Summary tab answers "what did we work on?" for the selected project. Pick a range:
Today, Yesterday, Last 7 days or Last 30 days (whole days in your local time).

- The header counts the cards worked on, the time agents spent working, the cards whose
  last run finished, and the cards created.
- **Active now** lists the cards that are running or waiting for you at this moment,
  whatever the range. It stays current while the tab is open.
- Below are the cards worked on in the range, most recent first, with the agent and
  model, the column, when the work happened, the working time, the number of runs and
  how it ended (Finished, Needs you, Still working, Idle, Exited, Stopped when idle).
  A range of several days is shown day by day. Click a row to open the card.
- Cards that were created in the range and never started are folded away at the bottom.

Working time is the time a card was Running. For Claude Code, Cursor and Codex a run is
a turn; for the other agents and for shell cards it is a burst of terminal output, so
their run counts are higher and their times rougher. The numbers come from Muster's own
log of status changes. Deleted cards are not included, and only the newest 1000 status
changes of a card are kept.

### Written summaries

**Summarise** writes what was done, in words: two or three sentences under each card
(what was asked, what was changed or found, how it ended) and one paragraph for the
whole range at the top. It happens only when you press the button, never by itself.

- The texts are kept. The next time you open the range they are there at once, and the
  button reads **Update**: it rewrites only the cards whose conversation has gone on
  since, and the paragraph if a card's text changed. "work has gone on since" under a
  text says it is behind.
- While it runs the button shows how far it is, and texts appear as they arrive.
- Cards without a conversation Muster can read (shell cards, opencode, Gemini,
  Antigravity) say "No conversation to summarise".
- In a range of several days a card's text covers the whole range and is shown under
  the most recent day the card was worked on.

What it costs and where the text goes: each card text is one request to a small model
(Haiku) through your own Claude Code, plus one request for the paragraph, so the first
Summarise of a range with 7 cards makes 8 requests and counts against your Claude plan
like any other Claude Code use. Muster sends an excerpt of the card's conversation in
the range: your prompts and the agent's replies, without tool output, at most about
12,000 characters however long the session was. It goes to Anthropic through Claude
Code and nowhere else. The helper process gets no tools, no MCP servers and none of your
project's instructions or hooks, and it is told to describe the conversation, not to
follow anything written in it. Claude Code must be installed and signed in; if it is
not, the reason is shown under the totals.

The list is read when the tab opens, when you change the range or the project, and when
you press Refresh. From a terminal: `muster summary <project> --since yesterday` (also
`today`, `7d`, `30d`); it prints the written summaries the app has made and does not
make any itself.

## Tracker

The tracker groups all cards across the board into Needs you, Finished, Running and
Idle, each with a timer: how long it has waited for you, or how long since it last
did anything. The menu-bar icon shows the number of cards that need you.

On a MacBook with a camera notch, Muster also draws a small black strip right of the
notch: the number of cards that are working (orange), that need you (red), and the
last three that finished (green tick). The strip is hidden when there is nothing to
show and on displays without a notch. Point at it and it grows into a list of those
cards by name; click a name to open that card. A finished card stays in the strip,
even after you have read it in the window, until you click it there or it starts
working again. Switch it off in Settings → Notifications.

A notification names the card, with "Finished" or "Needs you" and the project under
it. For a finished card the text is the start of the agent's last reply; for one that
needs you, what it asks ("Asks to use Bash: git push", or its question).

Also shown:

- **Agents and limits**: one line per agent tool you have installed, saying whether it
  is signed in and how (Claude account, ChatGPT, Google account...), the model its own
  settings start with, and the models your live cards are running. For Claude Code the
  panel shows **Current session** (the 5-hour window) and **Current week (all models)**,
  how much of each is **used**, and when each resets, the same windows Claude Code's
  `/usage` shows. Codex shows its 5-hour and weekly limits. The numbers come from your
  cards' own sessions, so they update when one of them talks to the model; "as of" says
  how old they are. A window whose reset time has passed shows 0% until the next
  message. Per-model weekly limits (for example Opus only) are not shown: Claude Code
  only gets those from Anthropic's usage service, which Muster does not call. Collapse
  the panel to keep just the fullest window of each agent in its header.
- **Model**: each card and tracker row shows the model its agent last reported, e.g.
  "Opus 5.5" or "gpt-5.5". Gemini and Antigravity do not report one.
- **Memory and CPU**: per session and in total. Sampling runs only while something is
  displaying it.

## Settings

- Terminal font, weight, size and line height.
- Whether programs in cards may copy to the Mac clipboard (off by default).
- Files: the editor that **Open in editor** uses, and whether the file viewer may show
  any file on the Mac (off by default: only the card's folder and project).
- Quiet time before a hook-less agent is treated as waiting.
- Stopping sessions that have been idle (off by default; see below).
- Per project: name, colour, worktree setup script.

## Stopping idle sessions

An agent that sits unused still holds memory. Settings → Daemon → "Stop idle sessions"
ends a session that has been idle for the time you choose (off by default). The card
stays on the board, marked Sleeping; open it and press Resume to continue the same
conversation.

A session is left running while any of these is true:

- the agent is working, or the card is waiting for you;
- the card is open in the window;
- the agent did something, or you opened the card or typed into it, within the chosen
  time. Output by itself does not count: a terminal can repaint without anyone using it;
- the agent has scheduled a job or a wake-up inside the session (Claude Code's
  scheduled prompts and loops), or a command it started is still running;
- you ticked "Keep running" for the card, in its details.

How long a session has been idle is stored with its card, so restarting Muster does not
start the wait again; the first check runs about a minute after the daemon starts, then
every five minutes. When an idle session is left running, the card's details say why. Shell cards are never
stopped, because a shell cannot be resumed. Jobs in the Mac's own crontab or launchd do
not depend on a session and are not affected.
