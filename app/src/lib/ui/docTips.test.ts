import { describe, expect, it } from "vitest";
import type { DocEntry, DocKind } from "../protocol/types";
import { LONG_LINES, remarks } from "./docTips";

const doc = (path: string, kind: DocKind): DocEntry => ({ path, kind, size: 1, modifiedAt: 1 });
const claude = doc("CLAUDE.md", "claude");
const agents = doc("AGENTS.md", "agents");

describe("remarks", () => {
  it("says nothing about an ordinary document or a good instruction file", () => {
    expect(remarks("doc", "", [])).toEqual([]);
    expect(remarks("claude", "# Build\nmake", [claude])).toEqual([]);
    expect(remarks("claude", "@AGENTS.md", [claude, agents])).toEqual([]);
    expect(remarks("agents", "# Build", [claude, agents])).toEqual([]);
  });

  it("points out a long or empty file", () => {
    const long = remarks("claude", "x\n".repeat(LONG_LINES + 5), [claude]);
    expect(long).toHaveLength(1);
    expect(long[0]).toContain(`${LONG_LINES + 6} lines`);
    expect(remarks("agents", " \n", [claude, agents])).toEqual(["This file is empty."]);
  });

  it("points out instructions the other agents would miss", () => {
    expect(remarks("claude", "# Build", [claude, agents])[0]).toContain("does not import it");
    expect(remarks("agents", "# Build", [agents])[0]).toContain("no `CLAUDE.md`");
  });
});
