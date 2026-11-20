// File paths in agents' text and terminal output, such as `src/lib.rs:932`,
// `./app/main.ts:12:4`, `docs/AGENTS.md#L40` or `/Users/me/x.py (line 3)`, for the
// file viewer.

export interface FileRef {
  path: string;
  line?: number;
  col?: number;
}

export interface FoundRef extends FileRef {
  /** Where the reference starts and ends in the text (end exclusive). */
  start: number;
  end: number;
}

/** File endings that make a bare name like `store.rs` a file without a folder or line. */
const EXTS = new Set(
  (
    "rs ts tsx js jsx mjs cjs svelte vue json jsonc toml yaml yml md mdx txt css scss sass less html htm xml svg " +
    "rb erb rake gemspec py pyi go java kt kts swift m mm h hpp c cc cpp cs php lua ex exs eex heex sh bash zsh fish " +
    "sql graphql gql proto lock ini cfg conf env plist gradle tf hcl dart scala clj zig nim r jl pl ps1 bat"
  ).split(" "),
);
/** Names without an ending that are still files. */
const NAMES = new Set(["Makefile", "Dockerfile", "Gemfile", "Rakefile", "Procfile", "Brewfile", "Justfile", "Podfile", "Fastfile"]);

// A path: optional `~/`, `./`, `../` or `/`, folders, then a name with an ending (or a
// known name). Then a line: `:12`, `:12:4`, `#L12`, `#L12-L20` or ` (line 12)`.
const PATH = String.raw`(?:~\/|\.{1,2}\/|\/)?(?:[\w@+\-.]+\/)*(?:[\w@+\-.]*\.[A-Za-z][\w]{0,11}|${[...NAMES].join("|")})`;
const LINE = String.raw`(?::(\d+)(?::(\d+))?|#L(\d+)(?:-L?\d+)?| \(line (\d+)\))?`;
const RE = new RegExp(String.raw`(?<![\w/.~\-:@])(${PATH})${LINE}(?![\w/])`, "g");

const num = (s: string | undefined) => (s ? Number(s) : undefined);

/**
 * The file references in `text`. A bare name such as `store.rs` (no folder, no line)
 * counts only when `loose` is set and its ending is a known one; with a folder or a
 * line any ending counts.
 */
export function findRefs(text: string, loose = false): FoundRef[] {
  const out: FoundRef[] = [];
  for (const m of text.matchAll(RE)) {
    const path = m[1];
    const line = num(m[2] ?? m[4] ?? m[5]);
    const col = num(m[3]);
    const name = path.slice(path.lastIndexOf("/") + 1);
    const ext = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "";
    const known = EXTS.has(ext) || NAMES.has(name);
    const folder = path.includes("/");
    // A version number or a domain is not a file: `v1.2`, `example.com`.
    if (!folder && line === undefined && !(loose && known)) continue;
    if (!folder && !known && !/^\.?[\w-]+\.[a-z]+$/i.test(name)) continue;
    if (name.startsWith(".") && name.length < 3) continue;
    // A URL's host (`github.com/...`) is not a folder of the project.
    if (folder && !path.startsWith(".") && !path.startsWith("/") && !path.startsWith("~") && /^[\w-]+\.(com|org|net|io|dev|app|ai|co)\//i.test(path)) continue;
    out.push({ path, line, col, start: m.index!, end: m.index! + m[0].length });
  }
  return out;
}

/** The whole of `text` read as one reference (e.g. inline code), or null. */
export function parseRef(text: string): FileRef | null {
  const t = text.trim();
  const r = findRefs(t, true);
  if (r.length !== 1 || r[0].start !== 0 || r[0].end !== t.length) return null;
  const { path, line, col } = r[0];
  return { path, line, col };
}

/** `path:line` for copying. */
export function refLabel(r: FileRef): string {
  return r.line ? `${r.path}:${r.line}` : r.path;
}
