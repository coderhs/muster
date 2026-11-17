// Parsing a unified diff from `card.diff`, and writing review comments as one message
// for the agent.

export type LineKind = "add" | "del" | "ctx";

export interface DiffLine {
  kind: LineKind;
  /** Line number in the old file (context and removed lines). */
  old?: number;
  /** Line number in the new file (context and added lines). */
  new?: number;
  text: string;
}

export interface Hunk {
  header: string;
  lines: DiffLine[];
}

export interface Patch {
  /** Lines before the first hunk that say something: rename, mode, binary. */
  notes: string[];
  hunks: Hunk[];
  binary: boolean;
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/;

export function parsePatch(patch: string): Patch {
  const out: Patch = { notes: [], hunks: [], binary: false };
  let hunk: Hunk | null = null;
  let oldNo = 0;
  let newNo = 0;
  const lines = patch.split("\n");
  if (lines[lines.length - 1] === "") lines.pop();
  for (const line of lines) {
    const m = HUNK.exec(line);
    if (m) {
      oldNo = Number(m[1]);
      newNo = Number(m[2]);
      hunk = { header: m[3].trim(), lines: [] };
      out.hunks.push(hunk);
      continue;
    }
    if (!hunk) {
      if (/^Binary files .* differ$/.test(line) || line === "GIT binary patch") out.binary = true;
      if (/^(rename|copy) (from|to) |^(new|deleted) file mode|^old mode|^new mode|^similarity index/.test(line))
        out.notes.push(line);
      continue;
    }
    const sign = line[0];
    const text = line.slice(1);
    if (sign === "+") hunk.lines.push({ kind: "add", new: newNo++, text });
    else if (sign === "-") hunk.lines.push({ kind: "del", old: oldNo++, text });
    else if (sign === " " || line === "") hunk.lines.push({ kind: "ctx", old: oldNo++, new: newNo++, text });
    // "\ No newline at end of file" and anything else is not a line of the file.
  }
  return out;
}

/** Where a comment points: a line of the new file, or a removed line of the old one. */
export interface LineRef {
  path: string;
  side: "new" | "old";
  line: number;
}

export interface ReviewComment extends LineRef {
  /** The line's text when the comment was written, quoted for the agent. */
  quote: string;
  text: string;
}

export const refKey = (r: LineRef) => `${r.path}\u0000${r.side}\u0000${r.line}`;

export function lineRef(path: string, l: DiffLine): LineRef {
  return l.kind === "del" ? { path, side: "old", line: l.old! } : { path, side: "new", line: l.new! };
}

/** The daemon's limit for one message. */
export const MAX_MESSAGE = 8000;

const QUOTE_MAX = 160;

/** The review as one prompt: an optional general note, then each comment with the
 *  line it is about, in file and line order. */
export function formatReview(comments: ReviewComment[], note: string): string {
  const sorted = [...comments].sort(
    (a, b) => a.path.localeCompare(b.path) || a.line - b.line || a.side.localeCompare(b.side),
  );
  const parts: string[] = ["Review of your changes. Please address each point, then summarise what you changed."];
  if (note.trim()) parts.push(note.trim());
  sorted.forEach((c, i) => {
    const where = c.side === "new" ? `${c.path}:${c.line}` : `${c.path}, removed line ${c.line} of the old version`;
    let quote = c.quote.trim();
    if (quote.length > QUOTE_MAX) quote = quote.slice(0, QUOTE_MAX) + "…";
    parts.push(`${i + 1}. ${where}${quote ? `\n   > ${quote}` : ""}\n   ${c.text.trim().replace(/\n/g, "\n   ")}`);
  });
  return parts.join("\n\n");
}
