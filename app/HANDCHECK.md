# Hand checks for the desktop app

Things only a person at the real window can confirm. Use a fresh build of `Muster.app`
and a project with at least two cards, one of them running a Claude Code or shell
session. Work top to bottom; each step says what should happen.

## 1. Launch and daemon

1. Quit Muster if it is running. Open `Muster.app` from Finder.
   **Expect:** the window opens; within a few seconds the toolbar shows "Connected".
   If no daemon was running, one is started (`ps ax | grep "musterd run"` shows it).
2. Press Cmd+Q, then run `ps ax | grep "musterd run"` in Terminal.
   **Expect:** Muster quits; `musterd run` is still running.
3. Open Muster again.
   **Expect:** it connects to the same daemon; `ps` still shows exactly one `musterd run`.

## 2. Window

4. Drag the window by the empty toolbar area to the right of the Board/Tracker switch.
   **Expect:** the window moves with the pointer.
5. Click the red close button.
   **Expect:** the window closes, Muster stays in the menu bar, the Dock icon remains.
6. Click Muster's Dock icon.
   **Expect:** the window comes back, on the same board.

## 3. Pickers and popups (placement)

7. Open a card (double-click). In the side panel, click the **Column** picker.
   **Expect:** the list opens directly under the picker, left edges aligned.
   Arrow keys move, Enter picks, Escape closes only the list.
8. Click the **Due date** field. **Expect:** the calendar opens directly under it.
9. Close the card. Press Shift+N. Click the column picker at the top right of the dialog.
   **Expect:** the list opens under the picker, also while the dialog is still sliding in.
10. Click "+" on a column header, and "…" on a column header.
    **Expect:** the agent list and the column menu open under the button that was clicked.
11. Right-click a card. **Expect:** a menu (Open, Link to card…, Hand off to …) opens under the card, not the WebKit "Reload / Inspect Element" menu.
12. Hover the gear at the bottom of the sidebar and a card's agent badge for a second.
    **Expect:** a tooltip appears next to the pointer ("Settings (⌘,)", the agent name).

## 4. Terminal

13. Open a card with a live session; click into the terminal and type `echo hello`, Enter.
    **Expect:** the text appears and the command runs.
14. Select `hello` with the mouse, press Cmd+C, then paste into another app.
    **Expect:** `hello` is pasted.
15. Copy some text in another app; back in the terminal press Cmd+V.
    **Expect:** the text is typed into the terminal.
16. Press Cmd+K. **Expect:** the terminal clears. Press Escape: it goes to the terminal (the window stays open). Press Cmd+W: the window closes.
17. Settings (Cmd+,) → Terminal → Size +. **Expect:** the open terminal's text grows immediately.

## 5. Folder picker

18. Click "+" next to **Projects** in the sidebar.
    **Expect:** the macOS folder chooser slides down from the title bar. Choose a folder:
    it appears in the sidebar and its board opens. Cancel does nothing.

## 6. Menu-bar item

19. Make a card need you (e.g. a Claude permission prompt).
    **Expect:** the menu-bar icon shows "1".
20. Click the menu-bar icon.
    **Expect:** "1 needs you", the card ("title — project · waiting …"), a "Sessions: … MB · …% CPU" line,
    "Show Muster", "Quit Muster".
21. With the window closed, choose the card in that menu.
    **Expect:** the window reopens with that card's window open.
22. Choose "Show Muster", then from the menu "Quit Muster".
    **Expect:** the window comes forward; then Muster quits (the daemon keeps running).

## 7. Notifications

23. Settings → Notifications: leave "Notify when a card needs you or finishes" on.
    Put another app in front. Make a card need you.
    **Expect (first time):** macOS asks whether Muster may send notifications. Allow.
    **Expect:** a "Needs you" notification with the project and card title.
24. Click the notification. **Expect:** Muster comes forward with that card open.
25. Repeat with Muster's window in front. **Expect:** no notification.
26. If no permission prompt appears at all: open Settings in Muster and read the line under
    Notifications. "Notifications need the app bundle" means the copy is not running as
    Muster.app. If it says nothing and System Settings → Notifications has no Muster entry,
    the build is not signed in a way macOS accepts for notifications (see the report).

## 8. Keyboard

27. On the board press ?. **Expect:** the shortcut sheet. Escape closes it.
28. Press N. **Expect:** the agent list under the first column's "+"; Enter starts the
    highlighted agent and opens its terminal with the cursor in it.

## 9. Claude prompts (needs a real Claude Code session with hooks)

29. In a Claude card, ask Claude to use a tool that needs permission (e.g. "run `ls` in a new shell").
    **Expect:** the card face, its tracker row and the card window show the tool and summary with Allow / Deny. Allow lets Claude continue.
30. Ask Claude to ask you a multiple-choice question (e.g. "ask me which of two options to pick, using your question tool").
    **Expect:** the options appear on the card; for one single-choice question they are buttons right on the card. Picking one continues the turn.
31. Put Claude in plan mode (Shift+Tab in its terminal) and give it a task.
    **Expect:** "Plan ready for review" on the card; in the card window the plan text with Approve, "Approve, accept edits" and "Send back…". Send back with a note: Claude revises.
32. If a yellow notice about PermissionRequest hooks shows at the top of the window, another tool's Claude hooks may answer permission prompts before Muster sees them; that is expected while those hooks exist.
