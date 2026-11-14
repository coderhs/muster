import { describe, expect, it } from "vitest";
import type { Card } from "../protocol/types";
import { groupCards, groupOf, timerLabel } from "./tracker";

const card = (id: string, extra: Partial<Card>): Card => ({
  id,
  projectId: "p",
  columnId: "c",
  rank: "m",
  title: id,
  agentId: "claude",
  cwd: "/",
  status: "idle",
  statusSince: 0,
  unread: false,
  hasSession: true,
  createdAt: 0,
  updatedAt: 0,
  ...extra,
});

describe("tracker", () => {
  it("groups by status, treating unread idle as finished", () => {
    expect(groupOf(card("a", { status: "needs_you" }))).toBe("needs_you");
    expect(groupOf(card("a", { status: "done", unread: true }))).toBe("finished");
    expect(groupOf(card("a", { status: "idle", unread: true }))).toBe("finished");
    expect(groupOf(card("a", { status: "idle" }))).toBe("idle");
    expect(groupOf(card("a", { status: "draft", hasSession: false }))).toBe("draft");
  });

  it("orders needs-you by longest wait and finished by most recent", () => {
    const g = groupCards([
      card("n1", { status: "needs_you", statusSince: 500 }),
      card("n2", { status: "needs_you", statusSince: 100 }),
      card("f1", { status: "done", statusSince: 100 }),
      card("f2", { status: "done", statusSince: 900 }),
    ]);
    expect(g.find((x) => x.key === "needs_you")!.cards.map((c) => c.id)).toEqual(["n2", "n1"]);
    expect(g.find((x) => x.key === "finished")!.cards.map((c) => c.id)).toEqual(["f2", "f1"]);
    expect(g.map((x) => x.key)).toEqual(["needs_you", "finished", "running", "idle", "exited", "draft"]);
    expect(g.filter((x) => x.collapsedByDefault).map((x) => x.key)).toEqual(["exited", "draft"]);
  });

  it("labels timers from the right timestamp", () => {
    const now = 10_000_000;
    expect(timerLabel(card("a", { status: "needs_you", statusSince: now - 240_000 }), now)).toBe("waiting 4m");
    expect(
      timerLabel(card("a", { status: "running", statusSince: now - 5_000, turnStartedAt: now - 720_000 }), now),
    ).toBe("running 12m");
    expect(timerLabel(card("a", { status: "running", statusSince: now - 5_000 }), now)).toBe("running 5s");
    expect(
      timerLabel(card("a", { status: "idle", statusSince: now - 9e6, lastActivityAt: now - 7_200_000 }), now),
    ).toBe("idle 2h");
    expect(timerLabel(card("a", { status: "done", statusSince: now - 60_000 }), now)).toBe("finished 1m ago");
    expect(
      timerLabel(card("a", { status: "running", statusSince: now - 20_000, setup: { state: "running" } }), now),
    ).toBe("setting up 20s");
  });
});
