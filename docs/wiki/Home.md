# Muster

Muster is a macOS app for running several coding agents at once without losing track
of them.

- Every agent session is a **card** on a **kanban board**.
- Opening a card shows the agent's real terminal, exactly as it would look in a
  terminal window.
- A **tracker** lists the cards that need you (a question or an approval), the ones
  that finished, the ones still running and the idle ones, with how long each has
  waited.
- Sessions keep running when the window is closed, when the app is quit, and when the
  app is updated.

## The idea in five sentences

1. A small background program, the **daemon** (`musterd`), owns everything: the board,
   the sessions and the status of each agent.
2. Each session runs inside **tmux**, so it lives independently of the app.
3. The **app window** is only a viewer and remote control; it talks to the daemon over
   a local WebSocket.
4. Agents report what they are doing through **hooks**, small callbacks the agent
   tools already support, so Muster knows a card's status without reading the screen.
5. Because the window is just a client, a phone can become a second client later.

## Pages

- [Using Muster](Using-Muster.md)
- [Architecture](Architecture.md)
- [Sessions and tmux](Sessions-and-tmux.md)
- [Agents](Agents.md)
- [Context sharing](Context-Sharing.md)
- [Security](Security.md)
- [Development](Development.md)
- [Troubleshooting](Troubleshooting.md)
- [Roadmap](Roadmap.md)

## Words used in these pages

| Word | Meaning |
|---|---|
| Card | One task on the board. It may have a running session. |
| Session | The agent process and its terminal, held by tmux. |
| Draft | A card that has not been started yet. |
| Daemon | `musterd`, the background program that owns all state. |
| Hook | A callback an agent tool runs on events such as "turn finished". |
| Project | A folder on disk with its own board. |
