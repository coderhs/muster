# Changelog

Notable changes in each Muster release. Releases are on the [releases page](https://github.com/coderhs/muster/releases).

Until the app's version string is wired up, Muster reports 0.1.0 in every build. The release tag is the real version.

## v0.1.4 (2026-11-27)

### New

- **Sort a column by when its cards last ran.** From a column's menu, **Sort by last ran** reorders its cards once. **Keep sorted by last ran** keeps the column in that order as cards start running, and the column header shows a sort icon while it is on.

### Fixes

- When an agent asked several questions at once, the questions panel in the card grew past the bottom of the window and could not be scrolled. The last questions and the **Send answers** button were out of reach. The panel now scrolls and leaves room for the terminal.

## v0.1.3 (2026-11-26)

### New

- **Linux packages** for Debian 13 and Ubuntu 24.04 or newer, on `amd64` and `arm64`. Shortcuts use Ctrl instead of ⌘, closing the window quits the app (sessions keep running), and there is no notch strip.

The macOS app had no changes.

## v0.1.2 (2026-11-20)

### New

- **File viewer.** ⌘-click a file an agent names in the terminal, or click it in the Chat tab, to see it at that line with syntax colours and ⌘F to find.
- **Open in your editor** (⌘O) at the same line, plus **Reveal in Finder** and **Copy path**. Choose the editor in Settings → Files.
- Only files in the card's folder and project open, unless you allow any file in Settings → Files.

### Fixes

- A file path an agent wrapped onto an indented next line is joined back into one link.
- Names without an extension, such as `Guardfile` or `bin/rails:12`, count as file paths.

## v0.1.1 (2026-11-18)

The first public build: a macOS app that runs coding-agent CLIs as cards on a kanban board, with answers to agent questions and permission requests on the card, a Changes tab for reviewing diffs, a tracker, idle sessions that sleep and resume, and phone or tablet access over your own Tailscale network.
