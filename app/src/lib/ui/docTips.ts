// Advice on the instruction files agents read, shown in the Docs tab. Each tip is a
// line of Markdown.

import type { DocEntry, DocKind } from "../protocol/types";

export interface TipSet {
  /** The file the tips are about. */
  file: string;
  what: string;
  tips: Array<{ head: string; text: string }>;
}

export const TIPS: Record<Exclude<DocKind, "doc">, TipSet> = {
  claude: {
    file: "CLAUDE.md",
    what: "Claude Code reads `CLAUDE.md` at the start of every session in this project. It is the place for what Claude cannot work out from the code.",
    tips: [
      { head: "Say how to build and test", text: "Give the exact commands to build, test, lint and run one test. This is what gets used most." },
      { head: "Write rules that can be checked", text: "`Use 2-space indentation` works; `format code properly` does not. One rule to a line, under a heading." },
      { head: "Keep it short", text: "Aim for under about 200 lines. The whole file is read into every session, and a long one is followed less well." },
      { head: "Leave out what the code already says", text: "Skip file-by-file descriptions and standard conventions of the language. Point to a document instead of copying it." },
      { head: "Split what grows", text: "Import another file with `@path/to/file.md`, or put rules for one part of the code in `.claude/rules/`." },
      { head: "Share with other agents", text: "Claude Code does not read `AGENTS.md`. If the project has one, a `CLAUDE.md` holding the line `@AGENTS.md` gives both the same instructions." },
      { head: "Keep private notes apart", text: "Preferences that are only yours go in `CLAUDE.local.md`, left out of git." },
      { head: "Start from a draft", text: "Running `/init` in Claude Code writes a first version from the project. Correct it when Claude gets something wrong twice." },
    ],
  },
  agents: {
    file: "AGENTS.md",
    what: "`AGENTS.md` is a plain Markdown file of instructions that Codex, Cursor, opencode and other coding agents read. It has no required sections.",
    tips: [
      { head: "Say how to build and test", text: "Setup, build and test commands come first: an agent runs them to check its own work." },
      { head: "Give the conventions", text: "Code style, how tests are named and laid out, and how commits and pull requests should look." },
      { head: "Name what to stay away from", text: "Generated files, folders that must not be edited, commands that touch production." },
      { head: "Keep it short and current", text: "It is read into every session. Remove a line when it stops being true." },
      { head: "One file per package in a large repository", text: "An `AGENTS.md` in a folder applies to the code under it; the nearest one to the edited file wins." },
      { head: "Share with Claude Code", text: "Claude Code reads `CLAUDE.md`, not this file. A `CLAUDE.md` holding the line `@AGENTS.md` makes it follow these instructions too." },
    ],
  },
};

/** Above this many lines an instruction file is long enough to say so. */
export const LONG_LINES = 200;

/** Remarks about the instruction file being read, from its text and the files beside it. */
export function remarks(kind: DocKind, content: string, docs: DocEntry[]): string[] {
  if (kind === "doc") return [];
  const out: string[] = [];
  const lines = content.split("\n").length;
  if (lines > LONG_LINES) {
    out.push(`This file has ${lines} lines. It is read into every session; moving detail into other documents keeps it followed.`);
  }
  if (!content.trim()) out.push("This file is empty.");
  const has = (k: DocKind) => docs.some((d) => d.kind === k);
  if (kind === "claude" && has("agents") && !/@AGENTS\.md/i.test(content)) {
    out.push("The project also has `AGENTS.md`, and this file does not import it. Adding the line `@AGENTS.md` keeps the two from drifting apart.");
  }
  if (kind === "agents" && !has("claude")) {
    out.push("There is no `CLAUDE.md`, so Claude Code does not see these instructions. A `CLAUDE.md` holding the line `@AGENTS.md` fixes that.");
  }
  return out;
}
