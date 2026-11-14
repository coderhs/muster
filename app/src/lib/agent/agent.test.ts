import { describe, expect, it } from "vitest";
import type { ChatMessage } from "../protocol/types";
import { composeInput, contextPercent, level, mergeChat, reasonLabel, resetLabel, tokens } from "./agent";

const msg = (id: string, text = id): ChatMessage => ({ id, role: "assistant", parts: [{ type: "text", text }] });

describe("agent helpers", () => {
  it("merges chat pages and live appends, replacing by id", () => {
    let m = mergeChat([], [msg("3"), msg("4")], "append");
    m = mergeChat(m, [msg("1"), msg("2")], "prepend");
    expect(m.map((x) => x.id)).toEqual(["1", "2", "3", "4"]);
    m = mergeChat(m, [msg("4", "extended"), msg("5")], "append");
    expect(m.map((x) => x.id)).toEqual(["1", "2", "3", "4", "5"]);
    expect((m[3].parts[0] as { text: string }).text).toBe("extended");
    // An overlapping older page does not duplicate.
    m = mergeChat(m, [msg("0"), msg("1")], "prepend");
    expect(m.map((x) => x.id)).toEqual(["0", "1", "2", "3", "4", "5"]);
  });

  it("computes context percent from either field", () => {
    expect(contextPercent({ usedTokens: 50_000, windowTokens: 200_000 })).toBe(25);
    expect(contextPercent({ usedTokens: 1, percent: 140 })).toBe(100);
    expect(contextPercent({ usedTokens: 10 })).toBeNull();
    expect(contextPercent(null)).toBeNull();
  });

  it("levels, labels and formats", () => {
    expect(level(50, 75, 90)).toBe("ok");
    expect(level(80, 75, 90)).toBe("warn");
    expect(level(95, 75, 90)).toBe("crit");
    expect(reasonLabel("permission")).toBe("Wants permission");
    expect(reasonLabel("something_new")).toBe("something new");
    expect(tokens(153_400)).toBe("153k");
    expect(resetLabel(1_000 + 3_600_000, 1_000)).toBe("resets in 1h");
    expect(resetLabel(null, 0)).toBeNull();
  });

  it("composes terminal input, bracketing multi-line text", () => {
    expect(composeInput("hello  \n")).toBe("hello\r");
    expect(composeInput("a\nb")).toBe("\x1b[200~a\nb\x1b[201~\r");
    expect(composeInput("   ")).toBe("");
  });
});
