// A small Markdown reader for the Docs tab. It turns text into a tree that the view
// renders as elements, so nothing from a file is ever inserted as HTML. It covers what
// project documentation uses: headings, paragraphs, lists, task lists, quotes, fenced
// code, tables, rules, links, emphasis and inline code. Raw HTML tags are dropped and
// their text kept.

export type Inline =
  | { t: "text"; s: string }
  | { t: "code"; s: string }
  | { t: "strong" | "em" | "del"; c: Inline[] }
  | { t: "link"; href: string; c: Inline[] }
  | { t: "br" };

export interface ListItem {
  /** Set for a task list item. */
  checked?: boolean;
  c: Block[];
}

export type Align = "left" | "center" | "right" | "";

export type Block =
  | { t: "heading"; level: number; id: string; c: Inline[] }
  | { t: "p"; c: Inline[] }
  | { t: "code"; lang: string; s: string }
  | { t: "quote"; c: Block[] }
  | { t: "list"; ordered: boolean; start: number; items: ListItem[] }
  | { t: "table"; align: Align[]; head: Inline[][]; rows: Inline[][][] }
  | { t: "hr" };

type Refs = Record<string, string>;

const FENCE = /^ {0,3}(`{3,}|~{3,})\s*([^\s`]*)[^`]*$/;
const HEADING = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/;
const RULE = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/;
const SETEXT = /^ {0,3}(=+|-+)[ \t]*$/;
const ITEM = /^( *)([-*+]|\d{1,9}[.)])(?:( +)(.*)|$)/;
const QUOTE = /^ {0,3}>/;
const TABLE_RULE = /^ *\|?[ \t]*:?-+:?[ \t]*(?:\|[ \t]*:?-+:?[ \t]*)*\|?[ \t]*$/;
const REF = /^ {0,3}\[([^\]]+)\]:[ \t]*<?([^\s>]+)>?(?:[ \t]+.*)?$/;

/** The heading's anchor, as GitHub makes it. */
export function slug(text: string): string {
  return text
    .toLowerCase()
    .trim()
    .replace(/[^\p{L}\p{N}\s_-]/gu, "")
    .replace(/\s/g, "-");
}

/** The plain text of inline content. */
export function plain(nodes: Inline[]): string {
  return nodes.map((n) => (n.t === "text" || n.t === "code" ? n.s : n.t === "br" ? " " : plain(n.c))).join("");
}

export function parse(src: string): Block[] {
  let lines = src.replace(/\r\n?/g, "\n").replace(/<!--[\s\S]*?-->/g, "").split("\n");
  // Front matter is settings for a tool, not documentation: shown as it is written.
  const out: Block[] = [];
  if (lines[0] === "---") {
    const end = lines.indexOf("---", 1);
    if (end > 0) {
      out.push({ t: "code", lang: "yaml", s: lines.slice(1, end).join("\n") });
      lines = lines.slice(end + 1);
    }
  }
  const refs: Refs = {};
  lines = lines.filter((l) => {
    const m = REF.exec(l);
    if (m) refs[m[1].toLowerCase()] = m[2];
    return !m;
  });
  const used = new Map<string, number>();
  blocks(lines, refs, used, out);
  return out;
}

function cells(line: string): string[] {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|") && !s.endsWith("\\|")) s = s.slice(0, -1);
  const out: string[] = [];
  let cur = "";
  let code = false;
  for (let i = 0; i < s.length; i++) {
    const ch = s[i];
    if (ch === "\\" && s[i + 1] === "|") {
      cur += "|";
      i++;
    } else if (ch === "`") {
      code = !code;
      cur += ch;
    } else if (ch === "|" && !code) {
      out.push(cur.trim());
      cur = "";
    } else cur += ch;
  }
  out.push(cur.trim());
  return out;
}

/** Whether a line begins something other than a paragraph. */
function starts(line: string): boolean {
  return FENCE.test(line) || HEADING.test(line) || RULE.test(line) || QUOTE.test(line) || ITEM.test(line);
}

function blocks(lines: string[], refs: Refs, used: Map<string, number>, out: Block[]): void {
  let i = 0;
  const heading = (level: number, text: string) => {
    const c = inline(text, refs);
    const base = slug(plain(c));
    const n = used.get(base) ?? 0;
    used.set(base, n + 1);
    out.push({ t: "heading", level, id: n ? `${base}-${n}` : base, c });
  };
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      continue;
    }
    let m = FENCE.exec(line);
    if (m) {
      const [, fence, lang] = m;
      const indent = line.length - line.trimStart().length;
      const body: string[] = [];
      i++;
      while (i < lines.length) {
        const t = lines[i].trim();
        if (t.startsWith(fence) && t.replace(new RegExp(`^${fence[0]}+`), "") === "") break;
        body.push(lines[i].slice(Math.min(indent, lines[i].length - lines[i].trimStart().length)));
        i++;
      }
      i++;
      out.push({ t: "code", lang, s: body.join("\n") });
      continue;
    }
    m = HEADING.exec(line);
    if (m) {
      heading(m[1].length, m[2] ?? "");
      i++;
      continue;
    }
    if (RULE.test(line)) {
      out.push({ t: "hr" });
      i++;
      continue;
    }
    if (QUOTE.test(line)) {
      const body: string[] = [];
      while (i < lines.length && lines[i].trim() && (QUOTE.test(lines[i]) || !starts(lines[i]))) {
        body.push(lines[i].replace(/^ {0,3}> ?/, ""));
        i++;
      }
      const c: Block[] = [];
      blocks(body, refs, used, c);
      out.push({ t: "quote", c });
      continue;
    }
    m = ITEM.exec(line);
    if (m) {
      const ordered = /\d/.test(m[2]);
      const start = ordered ? parseInt(m[2], 10) : 1;
      const base = m[1].length;
      const items: ListItem[] = [];
      while (i < lines.length) {
        const im = ITEM.exec(lines[i]);
        if (!im || im[1].length !== base || /\d/.test(im[2]) !== ordered) break;
        // Text of the item starts after the marker; later lines are indented that far.
        const width = im[1].length + im[2].length + Math.min(im[3]?.length ?? 1, 4);
        const body = [im[4] ?? ""];
        i++;
        while (i < lines.length) {
          const l = lines[i];
          const ind = l.length - l.trimStart().length;
          if (!l.trim()) {
            // A blank line stays in the item only when indented text follows.
            let j = i + 1;
            while (j < lines.length && !lines[j].trim()) j++;
            if (j >= lines.length || lines[j].length - lines[j].trimStart().length <= base) break;
            body.push("");
          } else if (ind > base) body.push(l.slice(Math.min(ind, width)));
          else if (starts(l) || !body[body.length - 1].trim()) break;
          else body.push(l.trim());
          i++;
        }
        const item: ListItem = { c: [] };
        const task = /^\[([ xX])\](?: +|$)/.exec(body[0]);
        if (task) {
          item.checked = task[1] !== " ";
          body[0] = body[0].slice(task[0].length);
        }
        blocks(body, refs, used, item.c);
        items.push(item);
        while (i < lines.length && !lines[i].trim()) i++;
      }
      out.push({ t: "list", ordered, start, items });
      continue;
    }
    if (line.includes("|") && i + 1 < lines.length && TABLE_RULE.test(lines[i + 1]) && lines[i + 1].includes("-")) {
      const head = cells(line);
      const rule = cells(lines[i + 1]);
      if (rule.length === head.length) {
        const align: Align[] = rule.map((r) =>
          r.startsWith(":") && r.endsWith(":") ? "center" : r.endsWith(":") ? "right" : r.startsWith(":") ? "left" : "",
        );
        i += 2;
        const rows: Inline[][][] = [];
        while (i < lines.length && lines[i].trim() && lines[i].includes("|")) {
          const row = cells(lines[i]);
          rows.push(head.map((_, k) => inline(row[k] ?? "", refs)));
          i++;
        }
        out.push({ t: "table", align, head: head.map((h) => inline(h, refs)), rows });
        continue;
      }
    }
    // A paragraph, or a heading underlined with = or -.
    const body = [line];
    i++;
    let level = 0;
    while (i < lines.length && lines[i].trim()) {
      const s = SETEXT.exec(lines[i]);
      if (s) {
        level = s[1][0] === "=" ? 1 : 2;
        i++;
        break;
      }
      if (starts(lines[i])) break;
      body.push(lines[i]);
      i++;
    }
    const text = body.map((l) => l.trimStart()).join("\n");
    if (level) heading(level, text.replace(/\n/g, " "));
    else {
      const c = inline(text, refs);
      if (c.length) out.push({ t: "p", c });
    }
  }
}

const WORD = /[\p{L}\p{N}]/u;

/** The index of the bracket that closes the one at `open`, or -1. */
function closing(s: string, open: number, a: string, b: string): number {
  let depth = 0;
  for (let i = open; i < s.length; i++) {
    if (s[i] === "\\") i++;
    else if (s[i] === a) depth++;
    else if (s[i] === b && --depth === 0) return i;
  }
  return -1;
}

export function inline(s: string, refs: Refs = {}): Inline[] {
  const out: Inline[] = [];
  let text = "";
  const flush = () => {
    if (text) out.push({ t: "text", s: text });
    text = "";
  };
  let i = 0;
  while (i < s.length) {
    const ch = s[i];
    if (ch === "\\" && i + 1 < s.length) {
      if (s[i + 1] === "\n") {
        flush();
        out.push({ t: "br" });
      } else if (/[!-/:-@[-`{-~]/.test(s[i + 1])) text += s[i + 1];
      else text += ch + s[i + 1];
      i += 2;
      continue;
    }
    if (ch === "\n") {
      if (text.endsWith("  ")) {
        text = text.trimEnd();
        flush();
        out.push({ t: "br" });
      } else text = text.trimEnd() + " ";
      i++;
      continue;
    }
    if (ch === "`") {
      let n = 1;
      while (s[i + n] === "`") n++;
      const tick = "`".repeat(n);
      let end = s.indexOf(tick, i + n);
      while (end >= 0 && s[end + n] === "`") end = s.indexOf(tick, end + n + 1);
      if (end >= 0) {
        flush();
        out.push({ t: "code", s: s.slice(i + n, end).replace(/\n/g, " ").replace(/^ (.*) $/, "$1") });
        i = end + n;
        continue;
      }
      text += tick;
      i += n;
      continue;
    }
    if (ch === "[" || (ch === "!" && s[i + 1] === "[")) {
      const image = ch === "!";
      const open = image ? i + 1 : i;
      const close = closing(s, open, "[", "]");
      if (close > 0) {
        const label = s.slice(open + 1, close);
        let href: string | undefined;
        let next = close + 1;
        if (s[next] === "(") {
          const end = closing(s, next, "(", ")");
          if (end > 0) {
            href = s
              .slice(next + 1, end)
              .trim()
              .replace(/\s+(?:"[^"]*"|'[^']*')$/, "")
              .replace(/^<(.*)>$/, "$1");
            next = end + 1;
          }
        } else if (s[next] === "[") {
          const end = s.indexOf("]", next);
          if (end > 0) {
            href = refs[(s.slice(next + 1, end) || label).toLowerCase()];
            if (href !== undefined) next = end + 1;
          }
        } else href = refs[label.toLowerCase()];
        if (href !== undefined) {
          flush();
          // An image is shown as a link to it, named by its description.
          const c = image ? [{ t: "text" as const, s: label || href }] : inline(label, refs);
          out.push({ t: "link", href, c });
          i = next;
          continue;
        }
      }
      text += ch;
      i++;
      continue;
    }
    if (ch === "<") {
      const auto = /^<((?:https?:\/\/|mailto:)[^\s>]+)>/.exec(s.slice(i));
      if (auto) {
        flush();
        out.push({ t: "link", href: auto[1], c: [{ t: "text", s: auto[1].replace(/^mailto:/, "") }] });
        i += auto[0].length;
        continue;
      }
      const tag = /^<\/?[a-zA-Z][^<>]*>/.exec(s.slice(i));
      if (tag) {
        if (/^<br\b/i.test(tag[0])) {
          flush();
          out.push({ t: "br" });
        }
        i += tag[0].length;
        continue;
      }
    }
    if (ch === "h" && (i === 0 || !WORD.test(s[i - 1]))) {
      const url = /^https?:\/\/[^\s<>]+/.exec(s.slice(i));
      if (url) {
        // Punctuation that ends the sentence is not part of the address.
        let href = url[0].replace(/[.,;:!?'"]+$/, "");
        while (href.endsWith(")") && href.split("(").length <= href.split(")").length - 1) href = href.slice(0, -1);
        flush();
        out.push({ t: "link", href, c: [{ t: "text", s: href }] });
        i += href.length;
        continue;
      }
    }
    if (ch === "*" || ch === "_" || ch === "~") {
      let n = 1;
      while (s[i + n] === ch) n++;
      const mark = ch === "~" ? (n >= 2 ? "~~" : "") : ch.repeat(Math.min(n, 2));
      const after = s[i + mark.length];
      // `_` inside a word (snake_case) is not emphasis.
      const wordy = ch === "_" && i > 0 && WORD.test(s[i - 1]);
      if (mark && after && !/\s/.test(after) && !wordy) {
        let end = s.indexOf(mark, i + mark.length);
        while (
          end >= 0 &&
          (/\s/.test(s[end - 1]) ||
            s[end - 1] === "\\" ||
            (mark.length === 1 && (s[end + 1] === ch || s[end - 1] === ch)) ||
            (ch === "_" && s[end + mark.length] !== undefined && WORD.test(s[end + mark.length])))
        )
          end = s.indexOf(mark, end + 1);
        // In `**a *b***` the last two marks close the bold.
        if (end >= 0 && mark.length === 2) while (s[end + 2] === ch) end++;
        if (end > i + mark.length) {
          flush();
          const c = inline(s.slice(i + mark.length, end), refs);
          out.push({ t: ch === "~" ? "del" : mark.length === 2 ? "strong" : "em", c });
          i = end + mark.length;
          continue;
        }
      }
      text += ch.repeat(n);
      i += n;
      continue;
    }
    text += ch;
    i++;
  }
  flush();
  return out;
}

/** Where a link in a document leads. */
export type Target =
  | { kind: "web"; url: string }
  /** Another file of the project, by its path from the project's folder. */
  | { kind: "doc"; path: string; anchor: string }
  | { kind: "anchor"; anchor: string }
  | { kind: "none" };

/** Resolves a link written in the document at `from` (a path from the project's folder). */
export function target(href: string, from: string): Target {
  if (/^(https?:\/\/|mailto:)/i.test(href)) return { kind: "web", url: href };
  if (/^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith("//")) return { kind: "none" };
  const hash = href.indexOf("#");
  let anchor = hash >= 0 ? href.slice(hash + 1) : "";
  let path = (hash >= 0 ? href.slice(0, hash) : href).split("?")[0];
  try {
    anchor = decodeURIComponent(anchor);
    path = decodeURIComponent(path);
  } catch {
    // Kept as written.
  }
  if (!path) return anchor ? { kind: "anchor", anchor } : { kind: "none" };
  const parts = path.startsWith("/") ? [] : from.split("/").slice(0, -1);
  for (const p of path.split("/")) {
    if (p === "..") {
      if (!parts.length) return { kind: "none" };
      parts.pop();
    } else if (p && p !== ".") parts.push(p);
  }
  return { kind: "doc", path: parts.join("/"), anchor };
}
