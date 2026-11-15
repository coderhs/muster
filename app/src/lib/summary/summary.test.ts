import { describe, expect, it } from "vitest";
import type { Card } from "../protocol/types";
import { activeNow, dayLabel, dayStart, endedLabel, rangeOf, runsLabel, splitWorked, timeSpan, workDuration } from "./summary";

const HOUR = 3_600_000;
// Local time, like the code under test: the 15th at 14:30.
const NOW = new Date(2026, 10, 15, 14, 30).getTime();
const day = (d: number) => new Date(2026, 10, d).getTime();

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

describe("summary ranges", () => {
  it("covers whole local days", () => {
    expect(rangeOf("today", NOW)).toEqual({ from: day(15), to: day(16), splits: [] });
    expect(rangeOf("yesterday", NOW)).toEqual({ from: day(14), to: day(15), splits: [] });
  });

  it("cuts several days at each midnight", () => {
    const week = rangeOf("7d", NOW);
    expect([week.from, week.to]).toEqual([day(9), day(16)]);
    expect(week.splits).toEqual([10, 11, 12, 13, 14, 15].map(day));
    const month = rangeOf("30d", NOW);
    // 30 days back from the 16th at midnight crosses into October.
    expect(month.from).toBe(new Date(2026, 9, 17).getTime());
    expect(month.to).toBe(day(16));
    expect(month.splits).toHaveLength(29);
    expect(month.splits[0]).toBe(new Date(2026, 9, 18).getTime());
    expect(month.splits.at(-1)).toBe(day(15));
  });

  it("finds midnight from any time of the day, across months", () => {
    expect(dayStart(day(15))).toBe(day(15));
    expect(dayStart(day(16) - 1)).toBe(day(15));
    expect(dayStart(new Date(2026, 10, 1, 8).getTime(), -1)).toBe(new Date(2026, 9, 31).getTime());
    expect(dayStart(new Date(2026, 11, 31, 23).getTime(), 1)).toBe(new Date(2027, 0, 1).getTime());
  });

  it("names days", () => {
    expect(dayLabel(day(15), NOW)).toBe("Today");
    expect(dayLabel(day(14), NOW)).toBe("Yesterday");
    expect(dayLabel(day(9), NOW)).toBe(new Date(2026, 10, 9).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" }));
  });
});

describe("summary words", () => {
  it("formats working time without folding hours into days", () => {
    expect(workDuration(0)).toBe("0s");
    expect(workDuration(-5)).toBe("0s");
    expect(workDuration(45_999)).toBe("45s");
    expect(workDuration(12 * 60_000)).toBe("12m");
    expect(workDuration(3 * HOUR)).toBe("3h");
    expect(workDuration(3 * HOUR + 5 * 60_000)).toBe("3h 5m");
    expect(workDuration(26 * HOUR + 30 * 60_000)).toBe("26h 30m");
  });

  it("says how the work ended", () => {
    expect(endedLabel({ outcome: "working" })).toEqual({ text: "Still working", tone: "running" });
    expect(endedLabel({ outcome: "needs_you" })).toEqual({ text: "Needs you", tone: "needs" });
    expect(endedLabel({ outcome: "finished" })).toEqual({ text: "Finished", tone: "done" });
    expect(endedLabel({ outcome: "finished", sessionEnd: "idle_stopped" }).text).toBe("Finished, stopped when idle");
    expect(endedLabel({ outcome: "finished", sessionEnd: "exited" }).text).toBe("Finished, exited");
    expect(endedLabel({ outcome: "idle" })).toEqual({ text: "Idle", tone: "idle" });
    expect(endedLabel({ outcome: "idle", sessionEnd: "exited" })).toEqual({ text: "Exited", tone: "exited" });
    expect(endedLabel({ outcome: "idle", sessionEnd: "idle_stopped" }).text).toBe("Stopped when idle");
    expect(endedLabel({ outcome: "not_started" }).text).toBe("Not started");
  });

  it("counts runs and shows the span of the work", () => {
    expect([runsLabel(1), runsLabel(3)]).toEqual(["1 run", "3 runs"]);
    const at = (h: number, m: number) => new Date(2026, 10, 15, h, m).getTime();
    const f = (t: number) => new Date(t).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
    expect(timeSpan(at(9, 5), at(11, 40))).toBe(`${f(at(9, 5))} – ${f(at(11, 40))}`);
    expect(timeSpan(at(9, 5), at(9, 5) + 20_000)).toBe(f(at(9, 5)));
  });

  it("separates worked cards from those only created", () => {
    const r = splitWorked([{ runs: 2 }, { runs: 0 }, { runs: 1 }]);
    expect(r.worked).toEqual([{ runs: 2 }, { runs: 1 }]);
    expect(r.createdOnly).toEqual([{ runs: 0 }]);
  });
});

describe("active now", () => {
  it("lists the project's running and waiting cards, waiting first", () => {
    const cards = [
      card("run-new", { status: "running", statusSince: 50 }),
      card("idle", { status: "idle" }),
      card("wait", { status: "needs_you", statusSince: 90 }),
      card("run-old", { status: "running", statusSince: 10 }),
      card("other", { status: "running", projectId: "q" }),
      card("wait-old", { status: "needs_you", statusSince: 20 }),
    ];
    expect(activeNow(cards, "p").map((c) => c.id)).toEqual(["wait-old", "wait", "run-old", "run-new"]);
    expect(activeNow(cards, "none")).toEqual([]);
  });
});
