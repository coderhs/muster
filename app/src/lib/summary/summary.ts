// Summary view: ranges in local days, and the words for what the daemon reports.

import type { Card, Id, ProjectSummary, SummaryCard, SummaryProgress } from "../protocol/types";
import { agoLabel } from "../ui/time";

export const RANGES = ["today", "yesterday", "7d", "30d"] as const;
export type RangeKey = (typeof RANGES)[number];

export const RANGE_LABEL: Record<RangeKey, string> = {
  today: "Today",
  yesterday: "Yesterday",
  "7d": "Last 7 days",
  "30d": "Last 30 days",
};

export interface Range {
  from: number;
  to: number;
  /** The local midnights inside the range; empty for a single day. */
  splits: number[];
}

/** Local midnight `days` days after the day of `at` (negative: before). */
export function dayStart(at: number, days = 0): number {
  const d = new Date(at);
  // Through the calendar, not by adding 24 hours: a day with a clock change is not 24 hours long.
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + days).getTime();
}

/** Whole local days: "Last 7 days" is today and the six days before it. */
export function rangeOf(key: RangeKey, now: number): Range {
  if (key === "yesterday") return { from: dayStart(now, -1), to: dayStart(now), splits: [] };
  const back = key === "7d" ? 6 : key === "30d" ? 29 : 0;
  const splits: number[] = [];
  for (let i = back - 1; i >= 0; i--) splits.push(dayStart(now, -i));
  return { from: dayStart(now, -back), to: dayStart(now, 1), splits };
}

/** "0s", "45s", "12m", "3h 5m", "26h": hours are not folded into days. */
export function workDuration(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  return m % 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${m / 60}h`;
}

export type Tone = "running" | "needs" | "done" | "idle" | "exited";

/** How the card's work in the range ended, in a few words, with the status colour to use. */
export function endedLabel(c: Pick<SummaryCard, "outcome" | "sessionEnd">): { text: string; tone: Tone } {
  const end = c.sessionEnd === "idle_stopped" ? "stopped when idle" : c.sessionEnd === "exited" ? "exited" : "";
  switch (c.outcome) {
    case "working":
      return { text: "Still working", tone: "running" };
    case "needs_you":
      return { text: "Needs you", tone: "needs" };
    case "finished":
      return { text: end ? `Finished, ${end}` : "Finished", tone: "done" };
    case "not_started":
      return { text: "Not started", tone: "idle" };
    default:
      // The run stopped without a finished turn; the session's end says more than "idle".
      return end ? { text: end[0].toUpperCase() + end.slice(1), tone: "exited" } : { text: "Idle", tone: "idle" };
  }
}

/** "1 run", "3 runs". */
export function runsLabel(n: number): string {
  return `${n} run${n === 1 ? "" : "s"}`;
}

/** "Today", "Yesterday", or "Mon, Nov 9" for the day that starts at `from`. */
export function dayLabel(from: number, now: number): string {
  if (from === dayStart(now)) return "Today";
  if (from === dayStart(now, -1)) return "Yesterday";
  return new Date(from).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
}

/** "09:05 – 11:40", or one time when both fall in the same minute. */
export function timeSpan(firstAt: number, lastAt: number): string {
  const f = (t: number) => new Date(t).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const [a, b] = [f(firstAt), f(lastAt)];
  return a === b ? a : `${a} – ${b}`;
}

/** Cards with a run in the range, and those only created in it. */
export function splitWorked<T extends Pick<SummaryCard, "runs">>(cards: T[]): { worked: T[]; createdOnly: T[] } {
  return { worked: cards.filter((c) => c.runs > 0), createdOnly: cards.filter((c) => c.runs === 0) };
}

/** Whether the range has written texts already: the button then updates them instead of starting. */
export function hasTexts(s: Pick<ProjectSummary, "text" | "cards">): boolean {
  return !!s.text || s.cards.some((c) => !!c.summary);
}

/** What the button says while a job runs: the cards first, then the paragraph. */
export function jobLabel(job: SummaryProgress): string {
  return job.done < job.total ? `Summarising ${job.done + 1} of ${job.total}…` : "Writing the overview…";
}

/** "written 5m ago", with a remark when work went on afterwards. */
export function writtenNote(at: number, stale: boolean | undefined, now: number): string {
  return `written ${agoLabel(at, now)}${stale ? " · work has gone on since" : ""}`;
}

/**
 * A card's text covers the whole range. In a range shown day by day it goes under the
 * most recent day the card was worked on: the day (its `from`) per card id.
 */
export function textDays(days: Array<{ from: number; cards: Array<Pick<SummaryCard, "id" | "runs">> }>): Map<Id, number> {
  const m = new Map<Id, number>();
  for (const d of [...days].sort((a, b) => b.from - a.from)) {
    for (const c of d.cards) if (c.runs > 0 && !m.has(c.id)) m.set(c.id, d.from);
  }
  return m;
}

/** The project's cards that run or wait for the user now: waiting first, the longest wait on top. */
export function activeNow(cards: Iterable<Card>, projectId: Id): Card[] {
  const list = [...cards].filter((c) => c.projectId === projectId && (c.status === "running" || c.status === "needs_you"));
  const order = (c: Card) => (c.status === "needs_you" ? 0 : 1);
  return list.sort((a, b) => order(a) - order(b) || a.statusSince - b.statusSince || (a.id < b.id ? -1 : 1));
}
