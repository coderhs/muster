// Tracker grouping: which sessions need attention, across all projects.

import type { Card } from "../protocol/types";
import { shortDuration } from "../ui/time";

export type GroupKey = "needs_you" | "finished" | "running" | "idle" | "exited" | "draft";

export interface Group {
  key: GroupKey;
  label: string;
  cards: Card[];
  /** Collapsed unless the user opens it. */
  collapsedByDefault: boolean;
}

export const GROUP_LABEL: Record<GroupKey, string> = {
  needs_you: "Needs you",
  finished: "Finished",
  running: "Running",
  idle: "Idle",
  exited: "Exited",
  draft: "Drafts",
};

export function groupOf(c: Card): GroupKey {
  switch (c.status) {
    case "needs_you":
      return "needs_you";
    case "done":
      return "finished";
    case "running":
      return "running";
    case "idle":
      // A turn that ended while nobody looked is still "finished" until read.
      return c.unread ? "finished" : "idle";
    case "exited":
      return "exited";
    default:
      return "draft";
  }
}

/** Start of the interval the row's timer measures. */
export function timerStart(c: Card): number {
  switch (c.status) {
    case "running":
      return c.turnStartedAt ?? c.statusSince;
    case "idle":
      return c.lastActivityAt ?? c.statusSince;
    case "draft":
      return c.createdAt;
    default:
      return c.statusSince;
  }
}

export function timerLabel(c: Card, now: number): string {
  const d = shortDuration(now - timerStart(c));
  if (c.setup?.state === "running") return `setting up ${shortDuration(now - c.statusSince)}`;
  switch (c.status) {
    case "needs_you":
      return `waiting ${d}`;
    case "running":
      return `running ${d}`;
    case "idle":
      return c.unread ? `finished ${d} ago` : `idle ${d}`;
    case "done":
      return `finished ${d} ago`;
    case "exited":
      return `exited ${d} ago`;
    default:
      return `created ${d} ago`;
  }
}

const ORDER: GroupKey[] = ["needs_you", "finished", "running", "idle", "exited", "draft"];

export function groupCards(cards: Iterable<Card>): Group[] {
  const by = new Map<GroupKey, Card[]>(ORDER.map((k) => [k, []]));
  for (const c of cards) by.get(groupOf(c))!.push(c);
  const oldestFirst = (a: Card, b: Card) => timerStart(a) - timerStart(b) || (a.id < b.id ? -1 : 1);
  const newestFirst = (a: Card, b: Card) => timerStart(b) - timerStart(a) || (a.id < b.id ? -1 : 1);
  return ORDER.map((key) => {
    const list = by.get(key)!;
    // Longest-waiting first where waiting matters; most recent first elsewhere.
    list.sort(key === "needs_you" || key === "running" ? oldestFirst : newestFirst);
    return { key, label: GROUP_LABEL[key], cards: list, collapsedByDefault: key === "exited" || key === "draft" };
  });
}
