// Card search (⌘F / ⌘K): matches cards in every project, the current project first.
import type { Card, Id, Project } from "../protocol/types";

export interface SearchHit {
  card: Card;
  /** Where the query matched, when not in the title: a short excerpt to show under it. */
  excerpt: string | null;
}

export interface SearchGroup {
  project: Project;
  hits: SearchHit[];
}

/** Rows shown for an empty query, per project. */
const RECENT_PER_PROJECT = 5;
/** Rows shown per project for a query. */
const MAX_PER_PROJECT = 50;
const EXCERPT = 70;

const FIELDS = ["title", "branch", "prompt", "notes", "model", "agentId"] as const;

function terms(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

function text(card: Card, field: (typeof FIELDS)[number]): string {
  return (card[field] ?? "").toString();
}

/** Lower is better; null when a term is missing from every field. */
function score(card: Card, ts: string[]): number | null {
  const title = card.title.toLowerCase();
  const rest = FIELDS.slice(1)
    .map((f) => text(card, f).toLowerCase())
    .join("\n");
  let total = 0;
  for (const t of ts) {
    const at = title.indexOf(t);
    if (at === 0) continue;
    if (at > 0) {
      // A match at a word start reads as a better hit than one inside a word.
      total += /[\s\-_/.:]/.test(title[at - 1]) ? 1 : 2;
    } else if (rest.includes(t)) {
      total += 4;
    } else {
      return null;
    }
  }
  return total;
}

/** A short, single-line piece of the first non-title field that holds a term. */
function excerpt(card: Card, ts: string[]): string | null {
  const title = card.title.toLowerCase();
  if (ts.every((t) => title.includes(t))) return null;
  for (const f of FIELDS.slice(1)) {
    const raw = text(card, f).replace(/\s+/g, " ").trim();
    const lower = raw.toLowerCase();
    const hit = ts.map((t) => lower.indexOf(t)).find((i) => i >= 0);
    if (hit === undefined) continue;
    const start = Math.max(0, hit - 20);
    const piece = raw.slice(start, start + EXCERPT);
    return (start > 0 ? "…" : "") + piece + (start + EXCERPT < raw.length ? "…" : "");
  }
  return null;
}

const recency = (c: Card) => c.lastActivityAt ?? c.updatedAt;

/**
 * Cards grouped by project: the current project first, then the others in sidebar
 * order. An empty query lists each project's most recently active cards.
 */
export function searchCards(cards: Card[], projects: Project[], query: string, currentId: Id | null): SearchGroup[] {
  const ts = terms(query);
  const byProject = new Map<Id, Array<{ card: Card; score: number }>>();
  for (const card of cards) {
    const s = ts.length ? score(card, ts) : 0;
    if (s === null) continue;
    const list = byProject.get(card.projectId) ?? [];
    list.push({ card, score: s });
    byProject.set(card.projectId, list);
  }
  const ordered = [...projects].sort(
    (a, b) => Number(b.id === currentId) - Number(a.id === currentId) || a.position - b.position,
  );
  const limit = ts.length ? MAX_PER_PROJECT : RECENT_PER_PROJECT;
  const groups: SearchGroup[] = [];
  for (const project of ordered) {
    const list = byProject.get(project.id);
    if (!list?.length) continue;
    list.sort((a, b) => a.score - b.score || recency(b.card) - recency(a.card));
    groups.push({
      project,
      hits: list.slice(0, limit).map(({ card }) => ({ card, excerpt: ts.length ? excerpt(card, ts) : null })),
    });
  }
  return groups;
}
