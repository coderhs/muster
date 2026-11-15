import type { Agent, Card } from "../protocol/types";

export interface AiRename {
  /** The card's agent can name its own session; otherwise the control is not shown. */
  offered: boolean;
  /** Why it cannot run right now, for a disabled control's tooltip. */
  blocked: string | null;
}

/**
 * "Rename with AI" types the agent's rename command into the session, so it needs a
 * live session that is between turns.
 */
export function aiRename(card: Pick<Card, "hasSession" | "status">, agent: Pick<Agent, "capabilities"> | null | undefined): AiRename {
  if (agent?.capabilities?.aiRename !== true) return { offered: false, blocked: null };
  if (!card.hasSession) return { offered: true, blocked: "Start the session first: the agent names it from the conversation" };
  if (card.status !== "idle" && card.status !== "done") return { offered: true, blocked: "Available when the agent is between turns" };
  return { offered: true, blocked: null };
}

/** Cards with a request under way, so a second click does nothing. */
const pending = new Set<string>();

interface Caller {
  call(m: "card.aiRename", p: { id: string }): Promise<unknown>;
}

/** Asks the daemon to rename the card; failures are reported by `app.call`. */
export async function requestAiRename(app: Caller, cardId: string): Promise<void> {
  if (pending.has(cardId)) return;
  pending.add(cardId);
  try {
    await app.call("card.aiRename", { id: cardId });
  } catch {
    /* toast shown */
  } finally {
    pending.delete(cardId);
  }
}
