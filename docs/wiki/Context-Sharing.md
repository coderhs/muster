# Context sharing

Agents in different cards can read each other's work and send each other messages,
but only where you allow it.

## Links

Link two cards from the card menu or with `muster link <card> <card>`. A link is the
permission: an agent can read only cards linked to its own.

## What a linked agent can read

| Kind | Contents |
|---|---|
| Summary | A short recent summary of the other card. |
| Transcript | The last N messages of its conversation. |
| Screen | What its terminal shows right now. |
| Notes | The card's notes. |

## Messages

`muster send <card> <text>` leaves a message for another agent. It is delivered when
that agent is **between turns**, never in the middle of one, and it is pasted as text,
so it cannot press keys or run commands by itself. Queued messages are visible on the
card and can be cancelled.

## Handoff

`muster handoff <card> [--agent A]` creates a new card, linked to the old one, that
continues the work, optionally with a different agent.

## The `muster` CLI

For you, in any terminal:

```
muster status
muster project add <path> [--name N]
muster board <project>
muster card new <project> [--agent A] [--title T] [--prompt P] [--cwd DIR]
                          [--worktree BRANCH] [--resume ID] [--start]
muster card move|start|kill|rm <card>
muster attach <card>
muster stats
muster link|unlink <card> <card>
muster handoff <card>
```

For an agent inside a card (the card is taken from `MUSTER_CARD_ID`):

```
muster context list
muster context read <card> [--summary | --transcript N | --screen | --notes]
muster card move --me <column>
muster notes set|append <text>
muster send <card> <text>
```

Claude Code cards get a `muster` skill that explains these commands to the agent.
