// Pure helpers for the tracker's accounts panel: limit rows, reset text, models in use.

import type { AgentAccount, Card, Usage, UsageWindow } from "../protocol/types";
import { shortDuration } from "../ui/time";

export interface LimitRow {
  key: string;
  label: string;
  usedPercent: number;
  resetsAt: number | null;
  /** The window's reset time has passed: nothing used yet in the next one. */
  expired: boolean;
}

/** Window names as each agent's own usage screen words them. */
const NAMES: Record<string, { five: string; week: string }> = {
  claude: { five: "Current session", week: "Current week (all models)" },
  codex: { five: "5h limit", week: "Weekly limit" },
};

function windowLabel(minutes: number | null | undefined, name: string): string {
  if (!minutes) return name;
  if (minutes % 1440 === 0) return minutes === 10080 ? "Weekly" : `${minutes / 1440}-day`;
  if (minutes % 60 === 0) return `${minutes / 60}-hour`;
  return `${minutes}-min`;
}

function row(key: string, label: string, w: UsageWindow, now: number): LimitRow {
  const resetsAt = w.resetsAt ?? null;
  const expired = resetsAt != null && resetsAt <= now;
  return { key, label, usedPercent: expired ? 0 : Math.max(0, Math.min(100, w.usedPercent)), resetsAt, expired };
}

/** The limit rows to show for one agent's usage, in its own wording. */
export function limitRows(u: Usage | null | undefined, agentId: string, now: number): LimitRow[] {
  if (!u) return [];
  const names = NAMES[agentId] ?? { five: "5-hour", week: "Weekly" };
  const named: LimitRow[] = [];
  if (u.fiveHour) named.push(row("five", names.five, u.fiveHour, now));
  if (u.sevenDay) named.push(row("week", names.week, u.sevenDay, now));
  const known = new Set([300, 10080]);
  // Windows that are neither 5-hour nor weekly (Codex reports a 30-day one on some plans).
  const extra = (u.windows ?? [])
    .filter((w) => !named.length || !known.has(w.windowMinutes ?? 0))
    .map((w) => row(`w:${w.name}`, windowLabel(w.windowMinutes, w.name), w, now));
  return [...named, ...extra];
}

/** "Resets 4:59 PM · in 2h 14m", or "Resets Nov 20, 10:00 AM · in 5d" further out. */
export function resetText(r: LimitRow, now: number): string | null {
  if (r.resetsAt == null) return null;
  if (r.expired) return "Reset · starts with your next message";
  const d = new Date(r.resetsAt);
  const sameDay = new Date(now).toDateString() === d.toDateString();
  const time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  const when = sameDay ? time : `${d.toLocaleDateString(undefined, { month: "short", day: "numeric" })}, ${time}`;
  return `Resets ${when} · in ${shortDuration(r.resetsAt - now)}`;
}

/** The fullest row, for a one-line summary. */
export function worstRow(rows: LimitRow[]): LimitRow | null {
  return rows.reduce<LimitRow | null>((a, b) => (!a || b.usedPercent > a.usedPercent ? b : a), null);
}

export interface ModelUse {
  model: string;
  /** Cards with a live session on this model. */
  live: number;
  /** Of those, running a turn now. */
  running: number;
}

/** Models of the cards of one agent (or all when `agentId` is null), live sessions first. */
export function modelsInUse(cards: Iterable<Card>, agentId: string | null): ModelUse[] {
  const by = new Map<string, ModelUse>();
  for (const c of cards) {
    if (!c.model || !c.hasSession || (agentId && c.agentId !== agentId)) continue;
    const m = by.get(c.model) ?? { model: c.model, live: 0, running: 0 };
    m.live++;
    if (c.status === "running" || c.status === "needs_you") m.running++;
    by.set(c.model, m);
  }
  return [...by.values()].sort((a, b) => b.live - a.live || a.model.localeCompare(b.model));
}

export type SignIn = "signed_in" | "signed_out" | "unknown" | "not_installed";

export function signIn(a: AgentAccount | null | undefined): SignIn {
  if (!a) return "unknown";
  if (!a.installed) return "not_installed";
  return a.signedIn === true ? "signed_in" : a.signedIn === false ? "signed_out" : "unknown";
}

export function signInLabel(a: AgentAccount | null | undefined): string {
  switch (signIn(a)) {
    case "signed_in":
      return a?.method ? `Signed in · ${a.method}` : "Signed in";
    case "signed_out":
      return "Not signed in";
    case "not_installed":
      return "Not installed";
    default:
      return "Sign-in unknown";
  }
}
