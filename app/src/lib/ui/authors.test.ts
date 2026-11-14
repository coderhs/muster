import { describe, expect, it } from "vitest";
import { authorLabel } from "./authors";
import { agoLabel } from "./time";

const agentOf = (id: string) => ({ id, label: id === "claude" ? "Claude Code" : id, color: "#000", installed: true });

describe("authorLabel", () => {
  it("names the user and agents", () => {
    expect(authorLabel({ kind: "user" }, agentOf, "a")).toBe("You");
    expect(authorLabel(null, agentOf, "a")).toBe("You");
    expect(authorLabel({ kind: "agent", agentId: "claude", cardId: "a", cardTitle: "Fix login" }, agentOf, "a")).toBe("Claude Code");
  });

  it("adds the card when the agent wrote from another card", () => {
    expect(authorLabel({ kind: "agent", agentId: "claude", cardId: "b", cardTitle: "Fix login" }, agentOf, "a")).toBe(
      "Claude Code · Fix login",
    );
    expect(authorLabel({ kind: "agent" }, agentOf, "a")).toBe("Agent");
  });
});

describe("agoLabel", () => {
  it("is relative for a week, then a date", () => {
    const now = Date.UTC(2026, 10, 14, 12);
    expect(agoLabel(now - 5_000, now)).toBe("just now");
    expect(agoLabel(now - 5 * 60_000, now)).toBe("5m ago");
    expect(agoLabel(now - 3 * 3_600_000 - 60_000, now)).toBe("3h ago");
    expect(agoLabel(now - 2 * 86_400_000, now)).toBe("2d ago");
    expect(agoLabel(now - 30 * 86_400_000, now)).not.toContain("ago");
  });
});
