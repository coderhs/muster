# Muster

A macOS app that runs coding-agent command-line tools (Claude Code, Codex, Cursor,
opencode, Gemini, Antigravity, or a plain shell) as cards on a kanban board. Each card
is a live terminal session. A tracker shows which sessions are waiting for you, which
are finished and which are still working.

Later, a phone client will reach the same sessions over your own Tailscale network.

## Quick start

Requirements: macOS 12+, [mise](https://mise.jdx.dev), tmux (`brew install tmux`),
Xcode Command Line Tools.

```sh
mise install                 # Rust and Node, pinned in mise.toml
scripts/install-stable.sh    # builds Muster.app and installs it in /Applications
```

To work on Muster itself, run the development app next to the stable one:

```sh
scripts/dev-app.sh           # opens "Muster Dev" with its own data and sessions
```

## Documentation

Start with the wiki in [`docs/wiki`](docs/wiki/Home.md):

| Page | What it covers |
|---|---|
| [Home](docs/wiki/Home.md) | What Muster is, in one page |
| [Using Muster](docs/wiki/Using-Muster.md) | Board, cards, tracker, settings |
| [Architecture](docs/wiki/Architecture.md) | Daemon, clients, where data lives |
| [Sessions and tmux](docs/wiki/Sessions-and-tmux.md) | How a card's terminal works and survives restarts |
| [Agents](docs/wiki/Agents.md) | What each supported agent can do |
| [Context sharing](docs/wiki/Context-Sharing.md) | Links, messages, handoff, the `muster` CLI |
| [Security](docs/wiki/Security.md) | Tokens, what a card may do |
| [Development](docs/wiki/Development.md) | Build, test, stable and dev, repository layout |
| [Troubleshooting](docs/wiki/Troubleshooting.md) | Known problems and fixes |
| [Roadmap](docs/wiki/Roadmap.md) | What is done and what is next |

Reference documents: [`docs/PLAN.md`](docs/PLAN.md) (design and phases),
[`docs/PROTOCOL.md`](docs/PROTOCOL.md) (daemon API),
[`docs/AGENTS.md`](docs/AGENTS.md) (agent integration details),
[`docs/REVIEW-daemon.md`](docs/REVIEW-daemon.md) (security review).
