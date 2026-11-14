# Using Muster

## Projects and the board

A **project** is a folder on disk. Each project has its own board. A new project gets
three columns: To Do, In Progress and Done. Columns can be added, renamed, recoloured
and reordered. Cards are moved by dragging.

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
- **Approvals and questions**: for Claude Code, a permission request, a question or a
  plan can be answered from the card without opening the terminal.
- **Details**: column, priority, due date, notes and links. Links connect cards for
  [context sharing](Context-Sharing.md). The notes show who last edited them.
- **Comments**: the second tab of the same column. You and agents both write here, and
  every comment says who wrote it.
- The details column can be hidden with its button in the card's title bar or ⌘I,
  which gives the terminal the full width.

## Sidebar

The projects sidebar can be hidden with the button at the left of the toolbar or ⌘B.
⌘1 to ⌘9 keep switching projects while it is hidden.

## Tracker

The tracker groups all cards across the board into Needs you, Finished, Running and
Idle, each with a timer: how long it has waited for you, or how long since it last
did anything. The menu-bar icon shows the number of cards that need you.

Also shown:

- **Usage limits**: for Claude Code, how much of the 5-hour and weekly allowance is
  **used**, and when each resets.
- **Memory and CPU**: per session and in total. Sampling runs only while something is
  displaying it.

## Settings

- Terminal font, weight, size and line height.
- Whether programs in cards may copy to the Mac clipboard (off by default).
- Quiet time before a hook-less agent is treated as waiting.
- Per project: name, colour, worktree setup script.
