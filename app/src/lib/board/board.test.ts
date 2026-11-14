import { describe, expect, it } from "vitest";
import type { Card, Column, Project } from "../protocol/types";
import {
  beginMove,
  cardChanged,
  cardsByColumn,
  columnChanged,
  emptyBoard,
  endMove,
  loadBoard,
  neighboursAt,
  projectChanged,
  sortedColumns,
} from "./board";

const project: Project = { id: "p", name: "P", path: "/p", color: "#000", position: 0 };
const col = (id: string, position: number): Column => ({
  id,
  projectId: "p",
  title: id,
  color: "#888",
  category: "started",
  position,
});
const card = (id: string, columnId: string, rank: string, extra: Partial<Card> = {}): Card => ({
  id,
  projectId: "p",
  columnId,
  rank,
  title: id,
  agentId: "shell",
  cwd: "/p",
  status: "idle",
  statusSince: 0,
  unread: false,
  hasSession: false,
  createdAt: 0,
  updatedAt: 1,
  ...extra,
});

const ids = (s: ReturnType<typeof base>, c: string) => (cardsByColumn(s).get(c) ?? []).map((x) => x.id);

function base() {
  return loadBoard(emptyBoard("p"), {
    project,
    columns: [col("b", 1), col("a", 0)],
    cards: [card("a2", "a", "m"), card("a1", "a", "g"), card("a3", "a", "t"), card("b1", "b", "m")],
  });
}

describe("board reducer", () => {
  it("loads and orders columns by position and cards by rank", () => {
    const s = base();
    expect(sortedColumns(s).map((c) => c.id)).toEqual(["a", "b"]);
    expect(ids(s, "a")).toEqual(["a1", "a2", "a3"]);
  });

  it("orders by byte-wise rank, then id", () => {
    const s = loadBoard(emptyBoard("p"), {
      project,
      columns: [col("a", 0)],
      cards: [card("x", "a", "a"), card("y", "a", "Z"), card("w", "a", "a")],
    });
    expect(ids(s, "a")).toEqual(["y", "w", "x"]);
  });

  it("applies card.changed upserts and removals, ignoring other projects", () => {
    let s = base();
    s = cardChanged(s, { card: card("a4", "b", "a", { updatedAt: 2 }) });
    expect(ids(s, "b")).toEqual(["a4", "b1"]);
    s = cardChanged(s, { removedId: "a1", projectId: "p" });
    expect(ids(s, "a")).toEqual(["a2", "a3"]);
    const before = s;
    s = cardChanged(s, { card: { ...card("z", "a", "a"), projectId: "other" } });
    expect(s).toBe(before);
  });

  it("never lets an older copy of a card replace a newer one", () => {
    let s = base();
    s = cardChanged(s, { card: card("a1", "a", "g", { title: "new", updatedAt: 5 }) });
    s = cardChanged(s, { card: card("a1", "a", "g", { title: "old", updatedAt: 3 }) });
    expect(s.cards.a1.title).toBe("new");
  });

  it("drops a card that moved to another project", () => {
    let s = base();
    s = cardChanged(s, { card: { ...card("a1", "a", "g", { updatedAt: 9 }), projectId: "q" } });
    expect(s.cards.a1).toBeUndefined();
  });

  it("applies column changes and removals", () => {
    let s = base();
    s = columnChanged(s, { column: col("c", 0.5) });
    expect(sortedColumns(s).map((c) => c.id)).toEqual(["a", "c", "b"]);
    s = columnChanged(s, { removedId: "c", projectId: "p" });
    expect(sortedColumns(s).map((c) => c.id)).toEqual(["a", "b"]);
    expect(columnChanged(s, { column: { ...col("x", 0), projectId: "q" } })).toBe(s);
  });

  it("clears the board when its project is removed", () => {
    const s = projectChanged(base(), { removedId: "p" });
    expect(s.projectId).toBeNull();
    expect(Object.keys(s.cards)).toHaveLength(0);
  });

  describe("optimistic moves", () => {
    it("shows the card at its target immediately", () => {
      const { state } = beginMove(base(), "a3", "b", "b1");
      expect(ids(state, "a")).toEqual(["a1", "a2"]);
      expect(ids(state, "b")).toEqual(["a3", "b1"]);
      expect(cardsByColumn(state).get("b")![0].columnId).toBe("b");
    });

    it("places before (above) / after (below) a card, within a column", () => {
      const { state } = beginMove(base(), "a1", "a", "a3", "a2");
      expect(ids(state, "a")).toEqual(["a2", "a1", "a3"]);
      const end = beginMove(base(), "a1", "a", undefined, "a3").state;
      expect(ids(end, "a")).toEqual(["a2", "a3", "a1"]);
      const top = beginMove(base(), "a3", "a", "a1").state;
      expect(ids(top, "a")).toEqual(["a3", "a1", "a2"]);
    });

    it("appends when no neighbours are given", () => {
      const { state } = beginMove(base(), "a1", "b");
      expect(ids(state, "b")).toEqual(["b1", "a1"]);
    });

    it("keeps the overlay when an unrelated card.changed arrives mid-move", () => {
      let { state: s, token } = beginMove(base(), "a1", "b", undefined, "b1");
      // Status change computed before the move: still in column a.
      s = cardChanged(s, { card: card("a1", "a", "g", { status: "running", updatedAt: 2 }) });
      expect(ids(s, "b")).toEqual(["b1", "a1"]);
      expect(cardsByColumn(s).get("b")![1].status).toBe("running");
      // The move's response reconciles to the daemon's rank.
      s = endMove(s, "a1", token, card("a1", "b", "z", { status: "running", updatedAt: 3 }));
      expect(s.moves).toEqual({});
      expect(ids(s, "b")).toEqual(["b1", "a1"]);
      expect(s.cards.a1.rank).toBe("z");
    });

    it("reverts to the daemon's copy when the move fails", () => {
      let { state: s, token } = beginMove(base(), "a1", "b");
      s = endMove(s, "a1", token, null);
      expect(ids(s, "a")).toEqual(["a1", "a2", "a3"]);
      expect(ids(s, "b")).toEqual(["b1"]);
    });

    it("a newer move on the same card is not cleared by an older one finishing", () => {
      const first = beginMove(base(), "a1", "b");
      const second = beginMove(first.state, "a1", "a", undefined, "a3");
      const s = endMove(second.state, "a1", first.token, null);
      expect(ids(s, "a")).toEqual(["a2", "a3", "a1"]);
    });

    it("drops the overlay when the card is deleted", () => {
      let { state: s } = beginMove(base(), "a1", "b");
      s = cardChanged(s, { removedId: "a1", projectId: "p" });
      expect(s.moves).toEqual({});
      expect(ids(s, "b")).toEqual(["b1"]);
    });

    it("survives a board reload while in flight", () => {
      const { state } = beginMove(base(), "a1", "b");
      const s = loadBoard(state, {
        project,
        columns: [col("a", 0), col("b", 1)],
        cards: [card("a1", "a", "g"), card("b1", "b", "m")],
      });
      expect(ids(s, "b")).toEqual(["b1", "a1"]);
    });
  });

  it("neighboursAt: before = card below the slot, after = card above, moving card excluded", () => {
    const list = [card("x", "a", "1"), card("m", "a", "2"), card("y", "a", "3")];
    expect(neighboursAt(list, 0, "m")).toEqual({ before: "x", after: undefined });
    expect(neighboursAt(list, 1, "m")).toEqual({ before: "y", after: "x" });
    expect(neighboursAt(list, 2, "m")).toEqual({ before: undefined, after: "y" });
    expect(neighboursAt([], 0, "m")).toEqual({ before: undefined, after: undefined });
  });
});
