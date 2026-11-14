import type { Agent, Author, Id } from "../protocol/types";

/**
 * How an author is shown: "You", or the agent's name with its card when the comment
 * or edit came from another card than the one being looked at.
 */
export function authorLabel(a: Author | null | undefined, agentOf: (id: string) => Agent, viewingCard: Id): string {
  if (!a || a.kind === "user") return "You";
  const name = a.agentId ? agentOf(a.agentId).label : "Agent";
  return a.cardId && a.cardId !== viewingCard && a.cardTitle ? `${name} · ${a.cardTitle}` : name;
}
