// Board reducer. Pure functions over an immutable BoardState.
//
// `cards` always holds the daemon's last word about each card. Optimistic moves live
// in `moves` as overlays (target column + neighbours) and are merged in at view time,
// so a `card.changed` that arrives mid-move (say, a status change) never makes the
// card jump back, and a failed move simply drops its overlay.

import type { Card, Column, Id, Project } from "../protocol/types";

export interface PendingMove {
  token: number;
  columnId: Id;
  /** Card the moved card is placed immediately before, i.e. its neighbour below. */
  before?: Id;
  /** Card the moved card is placed immediately after, i.e. its neighbour above. */
  after?: Id;
}

export interface BoardState {
  projectId: Id | null;
  project: Project | null;
  columns: Record<Id, Column>;
  cards: Record<Id, Card>;
  moves: Record<Id, PendingMove>;
  loaded: boolean;
}

export const emptyBoard = (projectId: Id | null = null): BoardState => ({
  projectId,
  project: null,
  columns: {},
  cards: {},
  moves: {},
  loaded: false,
});

export function loadBoard(
  s: BoardState,
  data: { project: Project; columns: Column[]; cards: Card[] },
): BoardState {
  const columns: Record<Id, Column> = {};
  for (const c of data.columns) columns[c.id] = c;
  const cards: Record<Id, Card> = {};
  for (const c of data.cards) cards[c.id] = c;
  // Keep overlays for in-flight moves whose card still exists.
  const moves: Record<Id, PendingMove> = {};
  for (const [id, m] of Object.entries(s.moves)) if (cards[id] && columns[m.columnId]) moves[id] = m;
  return { projectId: data.project.id, project: data.project, columns, cards, moves, loaded: true };
}

export function projectChanged(s: BoardState, p: { project: Project } | { removedId: Id }): BoardState {
  if ("removedId" in p) return p.removedId === s.projectId ? emptyBoard(null) : s;
  if (p.project.id !== s.projectId) return s;
  return { ...s, project: p.project };
}

export function columnChanged(
  s: BoardState,
  p: { column: Column } | { removedId: Id; projectId: Id },
): BoardState {
  if ("removedId" in p) {
    if (p.projectId !== s.projectId || !s.columns[p.removedId]) return s;
    const columns = { ...s.columns };
    delete columns[p.removedId];
    return { ...s, columns };
  }
  if (p.column.projectId !== s.projectId) return s;
  return { ...s, columns: { ...s.columns, [p.column.id]: p.column } };
}

export function cardChanged(s: BoardState, p: { card: Card } | { removedId: Id; projectId: Id }): BoardState {
  if ("removedId" in p) {
    if (p.projectId !== s.projectId || !s.cards[p.removedId]) return s;
    const cards = { ...s.cards };
    delete cards[p.removedId];
    const moves = { ...s.moves };
    delete moves[p.removedId];
    return { ...s, cards, moves };
  }
  const c = p.card;
  if (c.projectId !== s.projectId) {
    // A card that moved to another project leaves this board.
    if (!s.cards[c.id]) return s;
    const cards = { ...s.cards };
    delete cards[c.id];
    return { ...s, cards };
  }
  const prev = s.cards[c.id];
  // Responses and events can race; never let an older copy overwrite a newer one.
  if (prev && prev.updatedAt > c.updatedAt) return s;
  return { ...s, cards: { ...s.cards, [c.id]: c } };
}

let moveToken = 0;

export function beginMove(
  s: BoardState,
  cardId: Id,
  columnId: Id,
  before?: Id,
  after?: Id,
): { state: BoardState; token: number } {
  const token = ++moveToken;
  return {
    state: { ...s, moves: { ...s.moves, [cardId]: { token, columnId, before, after } } },
    token,
  };
}

/** Resolve an optimistic move. On success pass the daemon's card; on failure pass null. */
export function endMove(s: BoardState, cardId: Id, token: number, card: Card | null): BoardState {
  let next = s;
  if (s.moves[cardId]?.token === token) {
    const moves = { ...s.moves };
    delete moves[cardId];
    next = { ...s, moves };
  }
  return card ? cardChanged(next, { card }) : next;
}

export function sortedColumns(s: BoardState): Column[] {
  return Object.values(s.columns).sort((a, b) => a.position - b.position || cmp(a.id, b.id));
}

const cmp = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

export function compareCards(a: Card, b: Card): number {
  return cmp(a.rank, b.rank) || cmp(a.id, b.id);
}

/** Cards per column in display order, with optimistic moves applied. */
export function cardsByColumn(s: BoardState): Map<Id, Card[]> {
  const out = new Map<Id, Card[]>();
  for (const id of Object.keys(s.columns)) out.set(id, []);
  const moving: Card[] = [];
  for (const c of Object.values(s.cards)) {
    if (s.moves[c.id]) moving.push(c);
    else out.get(c.columnId)?.push(c);
  }
  for (const list of out.values()) list.sort(compareCards);
  // Insert overlaid cards in move order so chained moves resolve predictably.
  moving.sort((a, b) => s.moves[a.id].token - s.moves[b.id].token);
  for (const c of moving) {
    const m = s.moves[c.id];
    const list = out.get(m.columnId);
    if (!list) {
      out.get(c.columnId)?.push(c);
      continue;
    }
    const moved = { ...c, columnId: m.columnId };
    let at = -1;
    if (m.after) {
      const i = list.findIndex((x) => x.id === m.after);
      if (i >= 0) at = i + 1;
    }
    if (at < 0 && m.before) {
      const i = list.findIndex((x) => x.id === m.before);
      if (i >= 0) at = i;
    }
    if (at < 0) at = m.after ? list.length : m.before ? 0 : list.length;
    list.splice(at, 0, moved);
  }
  return out;
}

/**
 * The neighbours `card.move` needs to place a card at `index` of a target list:
 * `before` is the card that will sit directly below it, `after` the one directly above.
 */
export function neighboursAt(list: Card[], index: number, movingId: Id): { before?: Id; after?: Id } {
  const rest = list.filter((c) => c.id !== movingId);
  const i = Math.max(0, Math.min(index, rest.length));
  return { before: rest[i]?.id, after: rest[i - 1]?.id };
}
