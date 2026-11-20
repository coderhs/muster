// Syntax colours for the file viewer: highlight.js with a set of common languages,
// returned one line at a time so the viewer can number lines and jump to one.
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import csharp from "highlight.js/lib/languages/csharp";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import dockerfile from "highlight.js/lib/languages/dockerfile";
import elixir from "highlight.js/lib/languages/elixir";
import erb from "highlight.js/lib/languages/erb";
import go from "highlight.js/lib/languages/go";
import ini from "highlight.js/lib/languages/ini";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import kotlin from "highlight.js/lib/languages/kotlin";
import lua from "highlight.js/lib/languages/lua";
import makefile from "highlight.js/lib/languages/makefile";
import markdown from "highlight.js/lib/languages/markdown";
import php from "highlight.js/lib/languages/php";
import python from "highlight.js/lib/languages/python";
import ruby from "highlight.js/lib/languages/ruby";
import rust from "highlight.js/lib/languages/rust";
import scss from "highlight.js/lib/languages/scss";
import sql from "highlight.js/lib/languages/sql";
import swift from "highlight.js/lib/languages/swift";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

const LANGS = { bash, c, cpp, csharp, css, diff, dockerfile, elixir, erb, go, ini, java, javascript, json, kotlin, lua, makefile, markdown, php, python, ruby, rust, scss, sql, swift, typescript, xml, yaml };
for (const [name, lang] of Object.entries(LANGS)) hljs.registerLanguage(name, lang);

/** Larger files are shown without colours, to stay quick. */
export const MAX_HIGHLIGHT = 400_000;

const BY_EXT: Record<string, keyof typeof LANGS> = {
  sh: "bash", bash: "bash", zsh: "bash", fish: "bash", env: "bash",
  c: "c", h: "c", cc: "cpp", cpp: "cpp", hpp: "cpp", mm: "cpp", m: "c", cs: "csharp",
  css: "css", less: "css", scss: "scss", sass: "scss",
  diff: "diff", patch: "diff", ex: "elixir", exs: "elixir", erb: "erb", eex: "erb", heex: "erb",
  go: "go", ini: "ini", cfg: "ini", conf: "ini", toml: "ini", java: "java", gradle: "java",
  js: "javascript", jsx: "javascript", mjs: "javascript", cjs: "javascript",
  json: "json", jsonc: "json", lock: "json", kt: "kotlin", kts: "kotlin", lua: "lua",
  md: "markdown", mdx: "markdown", markdown: "markdown", php: "php", py: "python", pyi: "python",
  rb: "ruby", rake: "ruby", gemspec: "ruby", ru: "ruby", rs: "rust", sql: "sql", swift: "swift",
  ts: "typescript", tsx: "typescript", mts: "typescript", cts: "typescript",
  html: "xml", htm: "xml", xml: "xml", svg: "xml", plist: "xml", svelte: "xml", vue: "xml",
  yaml: "yaml", yml: "yaml",
};
const BY_NAME: Record<string, keyof typeof LANGS> = {
  Makefile: "makefile", Dockerfile: "dockerfile", Gemfile: "ruby", Rakefile: "ruby", Podfile: "ruby", Fastfile: "ruby", Brewfile: "ruby",
};

/** The language for a file, from its name; null for plain text. */
export function languageFor(path: string): string | null {
  const name = path.slice(path.lastIndexOf("/") + 1);
  if (BY_NAME[name]) return BY_NAME[name];
  const dot = name.lastIndexOf(".");
  return dot > 0 ? (BY_EXT[name.slice(dot + 1).toLowerCase()] ?? null) : null;
}

const escape = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/**
 * The file as HTML, one string per line. highlight.js escapes the text itself; spans
 * that run over several lines are closed at each line's end and opened again.
 */
export function highlightLines(code: string, lang: string | null): string[] {
  const text = code.replace(/\r\n?/g, "\n");
  if (!lang || text.length > MAX_HIGHLIGHT) return text.split("\n").map(escape);
  let html: string;
  try {
    html = hljs.highlight(text, { language: lang, ignoreIllegals: true }).value;
  } catch {
    return text.split("\n").map(escape);
  }
  const lines: string[] = [];
  const open: string[] = [];
  let cur = "";
  for (const tok of html.match(/<span[^>]*>|<\/span>|\n|[^<\n]+/g) ?? []) {
    if (tok === "\n") {
      lines.push(cur + "</span>".repeat(open.length));
      cur = open.join("");
    } else if (tok.startsWith("<span")) {
      open.push(tok);
      cur += tok;
    } else if (tok === "</span>") {
      open.pop();
      cur += tok;
    } else cur += tok;
  }
  lines.push(cur + "</span>".repeat(open.length));
  return lines;
}
