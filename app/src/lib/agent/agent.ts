// Pure helpers for agent status, approvals, context, chat and usage (protocol phase 3).

import type { Agent, AgentCapabilities, ChatMessage, ContextInfo, NeedsYouReason } from "../protocol/types";
import { shortDuration } from "../ui/time";

export const REASON_LABEL: Record<NeedsYouReason, string> = {
  permission: "Wants permission",
  question: "Asked a question",
  input: "Waiting for input",
  notification: "Sent a notification",
  startup_prompt: "Waiting at startup",
  terminal_prompt: "Prompt open in terminal",
  setup_failed: "Worktree setup failed",
  plan: "Has a plan for you to review",
};

export function reasonLabel(r: string | null | undefined, exitCode?: number | null): string | null {
  if (!r) return null;
  if (r === "setup_failed" && exitCode != null) return `Worktree setup failed (exit ${exitCode})`;
  return REASON_LABEL[r as NeedsYouReason] ?? r.replace(/_/g, " ");
}

/** 0..100, or null when the daemon gave nothing usable. */
export function contextPercent(c: ContextInfo | null | undefined): number | null {
  if (!c) return null;
  if (typeof c.percent === "number" && Number.isFinite(c.percent)) return clamp(c.percent);
  if (c.windowTokens && c.windowTokens > 0) return clamp((c.usedTokens / c.windowTokens) * 100);
  return null;
}

const clamp = (n: number) => Math.max(0, Math.min(100, n));

export type Level = "ok" | "warn" | "crit";

export function level(percent: number, warn: number, crit: number): Level {
  return percent >= crit ? "crit" : percent >= warn ? "warn" : "ok";
}

export function tokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1000) return `${Math.round(n / 1000)}k`;
  return String(n);
}

export function resetLabel(resetsAt: number | null | undefined, now: number): string | null {
  if (!resetsAt) return null;
  const d = resetsAt - now;
  return d <= 0 ? "resetting" : `resets in ${shortDuration(d)}`;
}

/**
 * Merge chat messages: `older` pages go in front, live appends at the end, and a
 * message that comes again with the same id replaces its earlier copy in place.
 */
export function mergeChat(current: ChatMessage[], incoming: ChatMessage[], where: "append" | "prepend"): ChatMessage[] {
  if (!incoming.length) return current;
  const index = new Map(current.map((m, i) => [m.id, i]));
  const out = [...current];
  const fresh: ChatMessage[] = [];
  for (const m of incoming) {
    const i = index.get(m.id);
    if (i !== undefined) out[i] = m;
    else {
      index.set(m.id, -1);
      fresh.push(m);
    }
  }
  return where === "prepend" ? [...fresh, ...out] : [...out, ...fresh];
}

/** Text to type into an agent's terminal for a composed message. */
export function composeInput(text: string): string {
  const t = text.replace(/\r\n?/g, "\n").replace(/\s+$/, "");
  if (!t) return "";
  // Multi-line text goes in as one bracketed paste so newlines do not submit early.
  return t.includes("\n") ? `\x1b[200~${t}\x1b[201~\r` : `${t}\r`;
}


/** Capabilities of an agent; an older daemon that reports none gets everything shown. */
export function caps(agent: Agent | null | undefined): AgentCapabilities {
  const all: AgentCapabilities = { statusSource: "hooks", approvals: true, questions: true, chat: true, context: true, usage: true, resume: true };
  if (!agent) return all;
  if (agent.capabilities) return agent.capabilities;
  return agent.id === "shell" ? { statusSource: "activity", approvals: false, questions: false, chat: false, context: false, usage: false, resume: false } : all;
}
