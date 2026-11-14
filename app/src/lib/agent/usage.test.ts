import { describe, expect, it } from "vitest";
import type { AgentAccount, Card } from "../protocol/types";
import { limitRows, modelsInUse, resetText, signIn, signInLabel, worstRow } from "./usage";

const H = 3_600_000;
const now = Date.UTC(2026, 10, 14, 12, 0, 0);

describe("usage rows", () => {
  it("names Claude's windows like its /usage screen", () => {
    const rows = limitRows(
      { fiveHour: { usedPercent: 42, resetsAt: now + 2 * H }, sevenDay: { usedPercent: 130, resetsAt: now + 50 * H }, updatedAt: now },
      "claude",
      now,
    );
    expect(rows.map((r) => r.label)).toEqual(["Current session", "Current week (all models)"]);
    expect(rows[1].usedPercent).toBe(100);
    expect(worstRow(rows)?.key).toBe("week");
  });

  it("shows a window whose reset has passed as empty", () => {
    const [r] = limitRows({ fiveHour: { usedPercent: 88, resetsAt: now - 1000 }, updatedAt: now - 6 * H }, "claude", now);
    expect(r.expired).toBe(true);
    expect(r.usedPercent).toBe(0);
    expect(resetText(r, now)).toMatch(/^Reset/);
  });

  it("keeps Codex windows that are not 5-hour or weekly", () => {
    const rows = limitRows(
      {
        fiveHour: { usedPercent: 10, resetsAt: now + H },
        windows: [
          { name: "primary", windowMinutes: 300, usedPercent: 10, resetsAt: now + H },
          { name: "secondary", windowMinutes: 43200, usedPercent: 3, resetsAt: now + 300 * H },
        ],
        updatedAt: now,
      },
      "codex",
      now,
    );
    expect(rows.map((r) => r.label)).toEqual(["5h limit", "30-day"]);
    // Only unnamed windows: every one is shown.
    expect(limitRows({ windows: [{ name: "primary", windowMinutes: 300, usedPercent: 1 }], updatedAt: now }, "x", now)[0].label).toBe("5-hour");
  });

  it("formats the reset with the time left", () => {
    const [r] = limitRows({ fiveHour: { usedPercent: 5, resetsAt: now + 2 * H + 14 * 60_000 }, updatedAt: now }, "claude", now);
    expect(resetText(r, now)).toMatch(/^Resets .* · in 2h 14m$/);
  });
});

describe("models in use", () => {
  const card = (id: string, model: string | null, agentId = "claude", status: Card["status"] = "idle", hasSession = true) =>
    ({ id, model, agentId, status, hasSession }) as Card;

  it("counts live sessions per model", () => {
    const cards = [
      card("a", "Opus 5.5", "claude", "running"),
      card("b", "Opus 5.5"),
      card("c", "Sonnet 5.5"),
      card("d", "gpt-5.5", "codex"),
      card("e", "Opus 5.5", "claude", "idle", false),
      card("f", null),
    ];
    expect(modelsInUse(cards, "claude")).toEqual([
      { model: "Opus 5.5", live: 2, running: 1 },
      { model: "Sonnet 5.5", live: 1, running: 0 },
    ]);
    expect(modelsInUse(cards, null)).toHaveLength(3);
  });
});

describe("sign-in", () => {
  const acct = (p: Partial<AgentAccount>): AgentAccount => ({ agentId: "x", installed: true, checkedAt: 0, ...p });
  it("labels each state", () => {
    expect(signInLabel(acct({ signedIn: true, method: "ChatGPT" }))).toBe("Signed in · ChatGPT");
    expect(signIn(acct({ signedIn: false }))).toBe("signed_out");
    expect(signIn(acct({}))).toBe("unknown");
    expect(signIn(acct({ installed: false, signedIn: true }))).toBe("not_installed");
    expect(signInLabel(null)).toBe("Sign-in unknown");
  });
});
