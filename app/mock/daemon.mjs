#!/usr/bin/env node
// In-memory mock of the Muster daemon (docs/PROTOCOL.md v1) for UI development.
//
//   node mock/daemon.mjs [--port 7317] [--token dev] [--empty]
//
// If MUSTER_HOME is set, writes daemon.json and token there so the Tauri shell can
// find it exactly as it would find the real daemon. Terminals are fake: each card
// gets a tiny line-echo "shell" that keeps a scrollback for repaints.

import { WebSocketServer } from "ws";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join, basename } from "node:path";

const args = process.argv.slice(2);
const arg = (name, dflt) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : dflt;
};
const PORT = Number(arg("port", process.env.MOCK_PORT ?? 7317));
const TOKEN = arg("token", process.env.MOCK_TOKEN ?? "dev");
const EMPTY = args.includes("--empty");
const QUIET = args.includes("--quiet");
const VERBOSE = args.includes("--verbose");
const VERSION = "0.0.0-mock";

const log = (...a) => QUIET || console.log("[mock]", ...a);
const now = () => Date.now();
// Deterministic ids, so a restarted mock serves the same seeded board (handy for
// testing reconnect and terminal re-attach).
let idSeq = 0;
const id = () => `m${(++idSeq).toString(36).padStart(4, "0")}`;

// ---------- fractional ranks (byte-wise sortable) ----------
const DIGITS = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
function rankBetween(a, b) {
  // a < result < b; a = "" means no lower bound, b = null means no upper bound.
  let out = "";
  for (let i = 0; ; i++) {
    const ca = i < a.length ? DIGITS.indexOf(a[i]) : 0;
    const cb = b !== null && i < b.length ? DIGITS.indexOf(b[i]) : DIGITS.length;
    if (ca === cb) {
      out += DIGITS[ca];
      continue;
    }
    const m = Math.floor((ca + cb) / 2);
    if (m > ca) return out + DIGITS[m];
    out += DIGITS[ca];
    b = null;
  }
}

// ---------- state ----------
const projects = new Map();
const columns = new Map();
const cards = new Map();
const sessions = new Map(); // cardId -> { buf, line, cols, rows, viewers: Map<client, {cols,rows,watch}> }

const cap = (statusSource, on) => ({
  statusSource,
  approvals: on.includes("approvals"),
  questions: on.includes("questions"),
  chat: on.includes("chat"),
  context: on.includes("context"),
  usage: on.includes("usage"),
  resume: on.includes("resume"),
});
const AGENTS = [
  { id: "claude", label: "Claude Code", color: "#d97757", installed: true, capabilities: cap("hooks", ["approvals", "questions", "chat", "context", "usage", "resume"]) },
  { id: "codex", label: "Codex", color: "#10a37f", installed: true, capabilities: cap("log", ["chat", "context", "usage", "resume"]) },
  { id: "cursor", label: "Cursor", color: "#7c7cf8", installed: false, capabilities: cap("hooks", ["questions", "chat", "context", "usage", "resume"]) },
  { id: "opencode", label: "opencode", color: "#e2b340", installed: true, capabilities: cap("activity", ["chat", "context", "resume"]) },
  { id: "gemini", label: "Gemini", color: "#4f8df5", installed: false, capabilities: cap("activity", ["resume"]) },
  { id: "shell", label: "Shell", color: "#8e8e93", installed: true, capabilities: cap("activity", []) },
];
const notices = [];
const daemonStatus = () => ({ version: VERSION, commit: "mock00000", commitDate: "2026-11-14T12:00:00+05:30", pid: process.pid, home: process.env.MUSTER_HOME ?? "(mock)", tmuxSocket: "mock", hooksAvailable: true, notices });

const DEFAULT_COLUMNS = [
  ["To Do", "#8e8e93", "unstarted"],
  ["In Progress", "#0a84ff", "started"],
  ["Done", "#30d158", "done"],
];

function addProject(path, name, color) {
  const p = {
    id: id(),
    name: name || basename(path) || path,
    path,
    color: color || "#0a84ff",
    position: projects.size,
  };
  projects.set(p.id, p);
  DEFAULT_COLUMNS.forEach(([title, c, category], i) => {
    const col = { id: id(), projectId: p.id, title, color: c, category, position: i };
    columns.set(col.id, col);
  });
  return p;
}

const projColumns = (pid) => [...columns.values()].filter((c) => c.projectId === pid).sort((a, b) => a.position - b.position);
const colCards = (cid) =>
  [...cards.values()].filter((c) => c.columnId === cid).sort((a, b) => (a.rank < b.rank ? -1 : a.rank > b.rank ? 1 : a.id < b.id ? -1 : 1));

function autoTitle(text) {
  const line = (text ?? "").split("\n").map((l) => l.replace(/\s+/g, " ").trim()).find(Boolean);
  if (!line || line.startsWith("/")) return null;
  return line.length > 60 ? line.slice(0, 60) + "…" : line;
}

function placeholderTitle(agentId, cwd) {
  if (agentId === "shell") return basename(cwd) || "Shell";
  return AGENTS.find((a) => a.id === agentId)?.label ?? agentId;
}

function makeCard({ projectId, columnId, title, agentId, cwd, prompt, branch }) {
  const list = colCards(columnId);
  const last = list[list.length - 1];
  const t = now();
  const c = {
    id: id(),
    projectId,
    columnId,
    rank: rankBetween(last ? last.rank : "", null),
    title: title || autoTitle(prompt) || placeholderTitle(agentId, cwd || projects.get(projectId).path),
    titleAuto: !title,
    agentId,
    cwd: cwd || projects.get(projectId).path,
    branch: branch ?? null,
    prompt: prompt ?? null,
    notes: null,
    priority: null,
    dueAt: null,
    agentSessionId: null,
    status: "draft",
    statusSince: t,
    turnStartedAt: null,
    lastActivityAt: null,
    unread: false,
    hasSession: false,
    restorable: false,
    createdAt: t,
    updatedAt: t,
  };
  cards.set(c.id, c);
  return c;
}

function seed() {
  const p = addProject("/Users/demo/code/muster", "muster", "#0a84ff");
  const [todo, doing, done] = projColumns(p.id);
  const t = now();
  const mk = (col, title, agentId, extra = {}) => Object.assign(makeCard({ projectId: p.id, columnId: col.id, title, agentId }), extra);
  mk(todo, "Write release notes for 0.2", "claude", { priority: "low" });
  mk(todo, "Investigate flaky websocket test", "codex", { priority: "high", dueAt: t + 86400e3 * 2 });
  mk(todo, "Port settings screen", "opencode");
  const r = mk(doing, "Refactor tmux backend into its own module", "claude", {
    branch: "refactor/tmux",
    priority: "medium",
  });
  const n = mk(doing, "Add SQLite migrations", "claude", { branch: "feat/migrations", priority: "urgent" });
  const s = mk(doing, "Scratch shell", "shell");
  mk(done, "Fix card rank collisions", "codex", { branch: "fix/rank" });
  for (const [c, status, ago, unread] of [
    [r, "running", 95e3, false],
    [n, "needs_you", 260e3, true],
    [s, "idle", 1800e3, false],
  ]) {
    startSession(c, false);
    c.status = status;
    c.statusSince = t - ago;
    c.unread = unread;
  }
  const d = [...cards.values()].find((c) => c.title.startsWith("Fix card"));
  startSession(d, false);
  d.status = "done";
  d.unread = true;
  d.statusSince = t - 40e3;
  // Phase 3: a pending permission request, context usage and transcripts.
  n.needsYouReason = "permission";
  n.pendingApproval = { requestId: "req-1", tool: "Bash", summary: "Bash: rm -rf build/ && cargo build", since: t - 260e3 };
  n.context = { usedTokens: 184_000, windowTokens: 200_000, percent: 92 };
  r.context = { usedTokens: 96_000, windowTokens: 200_000, percent: 48 };
  // The model each agent last reported.
  n.model = "Opus 5.5";
  r.model = "Sonnet 5.5";
  d.model = "Opus 5.5";
  seedTranscript(r, 130);
  links.add(linkKey(r.id, n.id));
  // A session cut off by a reboot: offers Restore.
  Object.assign(mk(doing, "Investigate memory growth in the daemon", "claude", { branch: "fix/mem" }), {
    status: "exited",
    statusSince: t - 3 * 3600e3,
    restorable: true,
    agentSessionId: "sess-restore-1",
  });
  seedTranscript(n, 12);
  const w = addProject("/Users/demo/code/website", "website", "#bf5af2");
  w.worktreeSetup = 'npm install\ncp "$MUSTER_PROJECT_PATH/.env" .env';
  const [wtodo, wdoing] = projColumns(w.id);
  const q = Object.assign(makeCard({ projectId: w.id, columnId: wdoing.id, title: "Redesign pricing page", agentId: "claude" }), {});
  startSession(q, false);
  Object.assign(q, { status: "needs_you", statusSince: t - 75e3, unread: true, needsYouReason: "question", model: "Fable 5.2" });
  seedTranscript(q, 4);
  makeCard({ projectId: w.id, columnId: wtodo.id, title: "Audit image sizes", agentId: "codex" });
}

// ---------- phase 3: chat, approvals, usage ----------
const transcripts = new Map(); // cardId -> ChatMessage[]
const chatSubs = new Map(); // cardId -> Set<client>
let msgSeq = 0;
// Account limits per agent, as the daemon merges them from every card.
const usage = {
  claude: {
    fiveHour: { usedPercent: 62, resetsAt: now() + 2.2 * 3600e3 },
    sevenDay: { usedPercent: 81, resetsAt: now() + 3.5 * 86400e3 },
    agentId: "claude",
    updatedAt: now() - 4 * 60e3,
  },
  codex: {
    fiveHour: { usedPercent: 12, resetsAt: now() + 4 * 3600e3 },
    sevenDay: { usedPercent: 33, resetsAt: now() + 6 * 86400e3 },
    windows: [
      { name: "primary", windowMinutes: 300, usedPercent: 12, resetsAt: now() + 4 * 3600e3 },
      { name: "secondary", windowMinutes: 10080, usedPercent: 33, resetsAt: now() + 6 * 86400e3 },
    ],
    agentId: "codex",
    updatedAt: now() - 50 * 60e3,
  },
};
const ACCOUNTS = [
  { agentId: "claude", installed: true, signedIn: true, method: "Claude account", defaultModel: "Opus", usageNote: "Session and weekly limits come from Claude Code's status line. Per-model weekly limits are only available from Anthropic's usage endpoint, which Muster does not call." },
  { agentId: "codex", installed: true, signedIn: true, method: "ChatGPT", defaultModel: "gpt-5.5", usageNote: "Limits appear after a Codex turn in a Muster card reports them." },
  { agentId: "cursor", installed: false, signedIn: true, method: "Cursor account", defaultModel: "Auto", usageNote: "Cursor reports plan usage only from its server." },
  { agentId: "opencode", installed: true, signedIn: false, detail: "free models only", usageNote: "opencode has no local usage record." },
  { agentId: "gemini", installed: false, usageNote: "Gemini reports quota only from Google's server." },
];

function chatMsg(role, parts, ts = now()) {
  return { id: `msg-${++msgSeq}`, role, timestamp: ts, model: role === "assistant" ? "mock-model" : undefined, parts };
}

function seedTranscript(card, n) {
  const list = [];
  const start = now() - n * 20e3;
  for (let i = 0; i < n; i++) {
    const ts = start + i * 20e3;
    const k = i % 4;
    if (k === 0) list.push(chatMsg("user", [{ type: "text", text: `Step ${i / 4 + 1}: please continue with the next part of "${card.title}".` }], ts));
    else if (k === 1)
      list.push(
        chatMsg("assistant", [{ type: "thinking" }, { type: "text", text: `Looking at the code for step ${Math.floor(i / 4) + 1}.` }, { type: "toolCall", id: `tc-${i}`, name: "Read", summary: `src/module_${i}.rs` }], ts),
      );
    else if (k === 2) list.push(chatMsg("tool", [{ type: "toolResult", toolUseId: `tc-${i - 1}`, isError: i % 12 === 2, summary: i % 12 === 2 ? "file not found" : "214 lines" }], ts));
    else list.push(chatMsg("assistant", [{ type: "text", text: `Done with step ${Math.floor(i / 4) + 1}.\nNext I will update the tests and run them.` }], ts));
  }
  transcripts.set(card.id, list);
}

function chatAppend(card, messages) {
  const list = transcripts.get(card.id) ?? [];
  for (const m of messages) {
    const i = list.findIndex((x) => x.id === m.id);
    if (i >= 0) list[i] = m;
    else list.push(m);
  }
  transcripts.set(card.id, list);
  const msg = JSON.stringify({ t: "ev", m: "chat.appended", p: { cardId: card.id, messages } });
  for (const c of chatSubs.get(card.id) ?? []) c.ws.send(msg);
}

function answerApproval(card, decision, via, message) {
  const a = card.pendingApproval;
  termOut(card.id, decision === "allow" ? `\x1b[32mAllowed (${via})\x1b[0m: ${a?.summary ?? ""}\r\n$ ` : `\x1b[31mDenied (${via})\x1b[0m${message ? `: ${message}` : ""}\r\n$ `);
  chatAppend(card, [chatMsg("tool", [{ type: "toolResult", toolUseId: "approval", isError: decision !== "allow", summary: decision === "allow" ? `allowed: ${a?.summary ?? ""}` : `denied${message ? `: ${message}` : ""}` }])]);
  setStatus(card, decision === "allow" ? "running" : "idle", false);
  return via;
}

function clearNeeds(card) {
  delete card.needsYouReason;
  delete card.pendingApproval;
}

// ---------- fake terminal ----------
const promptedCards = new Set();
const worktreeCards = new Set(); // cards created with a worktree whose setup has not succeeded yet

function startSession(card, broadcastIt = true) {
  const project = projects.get(card.projectId);
  if (worktreeCards.has(card.id) && project?.worktreeSetup) return runSetup(card, project, broadcastIt);
  startAgent(card, broadcastIt);
}

/** Simulated worktree setup: echoes the script's lines, fails if it contains "exit 1" or "fail". */
function runSetup(card, project, broadcastIt) {
  const sess = { buf: "", line: "", cols: 80, rows: 24, viewers: sessions.get(card.id)?.viewers ?? new Map() };
  sessions.set(card.id, sess);
  Object.assign(card, { hasSession: true, status: "running", statusSince: now(), setup: { state: "running" }, updatedAt: now() });
  delete card.needsYouReason;
  if (broadcastIt) cardEvent(card);
  const lines = project.worktreeSetup.split("\n").filter((l) => l.trim());
  const fails = /exit 1|fail/.test(project.worktreeSetup);
  termOut(card.id, `\x1b[2m# worktree setup in ${card.cwd}\x1b[0m\r\n\x1b[2m# MUSTER_BRANCH=${card.branch}\x1b[0m\r\n`);
  lines.forEach((l, i) =>
    setTimeout(() => termOut(card.id, `\x1b[1m$ ${l}\x1b[0m\r\n${fails && i === lines.length - 1 ? "error: something went wrong\r\n" : "ok\r\n"}`), 300 * (i + 1)),
  );
  setTimeout(() => {
    if (!cards.has(card.id) || sessions.get(card.id) !== sess) return;
    if (fails) {
      termOut(card.id, "\x1b[31msetup failed (exit 1)\x1b[0m\r\n$ ");
      Object.assign(card, { status: "needs_you", statusSince: now(), needsYouReason: "setup_failed", setup: { state: "failed", exitCode: 1 }, unread: true, updatedAt: now() });
      cardEvent(card);
    } else {
      worktreeCards.delete(card.id);
      delete card.setup;
      startAgent(card, true);
    }
  }, 300 * (lines.length + 1) + 200);
}

function startAgent(card, broadcastIt = true) {
  const sess = { buf: sessions.get(card.id)?.buf ?? "", line: "", cols: 80, rows: 24, viewers: sessions.get(card.id)?.viewers ?? new Map() };
  sessions.set(card.id, sess);
  card.hasSession = true;
  card.status = "running";
  card.statusSince = now();
  card.turnStartedAt = now();
  card.lastActivityAt = now();
  card.updatedAt = now();
  const agent = AGENTS.find((a) => a.id === card.agentId);
  termOut(
    card.id,
    `\x1b[1;38;5;75m${agent ? agent.label : card.agentId}\x1b[0m (mock) in \x1b[2m${card.cwd}\x1b[0m\r\n` +
      (card.prompt ? `\x1b[2m> ${card.prompt.replace(/\n/g, "\r\n  ")}\x1b[0m\r\n` : "") +
      `Type anything; lines are echoed back. Try \x1b[1mcolors\x1b[0m, \x1b[1mneed\x1b[0m, \x1b[1mneedkeys\x1b[0m, \x1b[1mask\x1b[0m, \x1b[1mdone\x1b[0m, \x1b[1musage\x1b[0m, \x1b[1mexit\x1b[0m, \x1b[1mflood\x1b[0m.\r\n$ `,
  );
  if (broadcastIt) cardEvent(card);
}

function termOut(cardId, text) {
  const sess = sessions.get(cardId);
  if (!sess) return;
  sess.buf += text;
  if (sess.buf.length > 64 * 1024) sess.buf = sess.buf.slice(-48 * 1024);
  const frame = termFrame(cardId, Buffer.from(text, "utf8"));
  for (const client of sess.viewers.keys()) client.ws.send(frame);
}

function termFrame(sid, bytes) {
  const s = Buffer.from(sid, "utf8");
  const head = Buffer.alloc(3);
  head[0] = 1;
  head.writeUInt16BE(s.length, 1);
  return Buffer.concat([head, s, bytes]);
}

function setStatus(card, status, unread) {
  if (status !== "needs_you") clearNeeds(card);
  card.status = status;
  card.statusSince = now();
  if (unread !== undefined) card.unread = unread;
  card.updatedAt = now();
  cardEvent(card);
  if (status === "idle" || status === "done") deliverNext(card);
}

// ---------- phase 4/5: links, messages, stats, settings ----------
const links = new Set(); // "a|b", a < b
const linkKey = (a, b) => (a < b ? `${a}|${b}` : `${b}|${a}`);
let settings = { keepAwake: true, autoRestore: false, silenceSeconds: 5 };
let msgIds = 0;
const comments = new Map(); // card id -> Comment[]
let commentIds = 0;
// As the daemon does: an agent author when the call names a card, else the user.
const authorOf = (fromCardId) => {
  const from = fromCardId ? cards.get(fromCardId) : null;
  return from ? { kind: "agent", cardId: from.id, agentId: from.agentId, cardTitle: from.title } : { kind: "user" };
};

/** Type the oldest queued message into an idle/done card that has a session. */
function deliverNext(card) {
  const q = card.queuedMessages;
  if (!q?.length || !sessions.has(card.id) || !["idle", "done"].includes(card.status)) return;
  const m = q.shift();
  if (!q.length) delete card.queuedMessages;
  const from = m.fromCardId ? cards.get(m.fromCardId) : null;
  const head = from ? `[Message from Muster card ${from.id} "${from.title}"; reply with: muster send ${from.id} "..."]` : "[Message from the user via Muster]";
  termOut(card.id, `\x1b[36m${head}\x1b[0m ${m.text}\r\n$ `);
  card.updatedAt = now();
  setStatus(card, "running", false);
}

const statsSubs = new Map(); // client -> intervalMs
let statsTimer = null;
const procStats = new Map(); // cardId -> {memBytes, cpuPercent, processes}

function sampleStats() {
  const list = [];
  for (const [cid] of sessions) {
    const c = cards.get(cid);
    if (!c) continue;
    const prev = procStats.get(cid) ?? { memBytes: (c.agentId === "shell" ? 6 : 180) * 2 ** 20, cpuPercent: 0, processes: c.agentId === "shell" ? 1 : 3 };
    const busy = c.status === "running";
    const s = {
      cardId: cid,
      memBytes: Math.max(4 * 2 ** 20, Math.round(prev.memBytes * (1 + (Math.random() - 0.45) * 0.04))),
      cpuPercent: Math.round((busy ? 20 + Math.random() * 60 : Math.random() * 2) * 10) / 10,
      processes: prev.processes,
    };
    procStats.set(cid, s);
    list.push(s);
  }
  const total = list.reduce((t, s) => ({ memBytes: t.memBytes + s.memBytes, cpuPercent: t.cpuPercent + s.cpuPercent, processes: t.processes + s.processes }), { memBytes: 0, cpuPercent: 0, processes: 0 });
  total.cpuPercent = Math.round(total.cpuPercent * 10) / 10;
  const msg = JSON.stringify({ t: "ev", m: "stats.changed", p: { cards: list, total, daemon: { memBytes: 14 * 2 ** 20, cpuPercent: 0.3, processes: 1 }, at: now() } });
  for (const c of statsSubs.keys()) c.ws.send(msg);
}

function restartStats() {
  clearInterval(statsTimer);
  statsTimer = null;
  if (!statsSubs.size) return;
  const ms = Math.min(...statsSubs.values());
  statsTimer = setInterval(sampleStats, ms);
  setTimeout(sampleStats, 50);
}

function termInput(card, data) {
  const sess = sessions.get(card.id);
  // Bracketed paste: drop the markers and keep pasted newlines inside the line.
  data = data.replace(/\x1b\[200~([\s\S]*?)\x1b\[201~/g, (_, t) => t.replace(/\r?\n/g, " ⏎ "));
  if (data === "1" && card.pendingApproval) return answerApproval(card, "allow", "keys");
  if (data === "\x1b" && card.pendingApproval) return answerApproval(card, "deny", "keys");
  for (const ch of data) {
    if (ch === "\r") {
      const line = sess.line;
      sess.line = "";
      termOut(card.id, "\r\n");
      runLine(card, line.trim());
    } else if (ch === "\x7f") {
      if (sess.line.length) {
        sess.line = sess.line.slice(0, -1);
        termOut(card.id, "\b \b");
      }
    } else if (ch === "\x03") {
      sess.line = "";
      termOut(card.id, "^C\r\n$ ");
    } else if (ch === "\x15") {
      termOut(card.id, "\b \b".repeat(sess.line.length));
      sess.line = "";
    } else if (ch >= " " || ch === "\t") {
      sess.line += ch;
      termOut(card.id, ch);
    }
  }
  card.lastActivityAt = now();
}

function runLine(card, line) {
  const sess = sessions.get(card.id);
  if (line === "exit") {
    termOut(card.id, "logout\r\n");
    endSession(card, 0);
    return;
  }
  if (line === "colors") {
    let s = "";
    for (let i = 0; i < 16; i++) s += `\x1b[48;5;${i}m  `;
    termOut(card.id, s + "\x1b[0m\r\n\x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munder\x1b[0m \x1b[31mred\x1b[32m green\x1b[0m 漢字 ✓\r\n$ ");
    return;
  }
  if (line === "flood") {
    let s = "";
    for (let i = 1; i <= 2000; i++) s += `line ${i} ${"#".repeat(i % 60)}\r\n`;
    termOut(card.id, s + "$ ");
    return;
  }
  if (line === "need" || line === "needkeys") {
    termOut(card.id, "\x1b[33mAgent wants to run: Bash(npm test)\x1b[0m\r\n  1. Yes  2. No (esc)\r\n");
    card.pendingApproval = { requestId: line === "need" ? `req-${++msgSeq}` : undefined, tool: "Bash", summary: "Bash: npm test", since: now() };
    card.needsYouReason = "permission";
    setStatus(card, "needs_you", true);
    return;
  }
  if (line === "question" || line === "questions") {
    termOut(card.id, "\x1b[33mClaude has a question for you.\x1b[0m\r\n");
    const qs = [{ question: "Which database should the service use?", header: "Database", multiSelect: false, options: [{ label: "SQLite", description: "Embedded, zero setup" }, { label: "PostgreSQL", description: "Separate server" }] }];
    if (line === "questions")
      qs.push({ question: "Which checks should run in CI?", header: "CI", multiSelect: true, options: [{ label: "Unit tests" }, { label: "Lint" }, { label: "Type check" }] });
    card.pendingApproval = { requestId: `req-${++msgSeq}`, kind: "question", tool: "AskUserQuestion", summary: qs[0].question, since: now(), questions: qs };
    card.needsYouReason = "question";
    setStatus(card, "needs_you", true);
    return;
  }
  if (line === "plan") {
    termOut(card.id, "\x1b[33mClaude finished planning.\x1b[0m\r\n");
    card.pendingApproval = {
      requestId: `req-${++msgSeq}`,
      kind: "plan",
      tool: "ExitPlanMode",
      summary: "Plan: split the tmux backend",
      since: now(),
      plan: "1. Move session handling into tmux.rs\n2. Add a SessionBackend trait\n3. Update callers and tests\n4. Run the full test suite",
      planFilePath: `${card.cwd}/.claude/plans/split-backend.md`,
    };
    card.needsYouReason = "plan";
    setStatus(card, "needs_you", true);
    return;
  }
  if (line === "notice") {
    if (notices.length) notices.length = 0;
    else notices.push({ id: "claude_permission_hooks", level: "warning", message: "Your Claude Code settings define PermissionRequest hooks, so approvals from Muster may be pre-empted by other tools." });
    broadcast("daemon.status", daemonStatus());
    termOut(card.id, `notices: ${notices.length}\r\n$ `);
    return;
  }
  if (line === "ask") {
    termOut(card.id, "\x1b[33mWhich database should I use?\x1b[0m\r\n$ ");
    card.needsYouReason = "question";
    setStatus(card, "needs_you", true);
    return;
  }
  if (line === "usage") {
    const bump = (w) => ({ ...w, usedPercent: Math.min(100, w.usedPercent + 7) });
    const u = usage.claude;
    usage.claude = { ...u, fiveHour: bump(u.fiveHour), sevenDay: bump(u.sevenDay), updatedAt: now() };
    broadcast("usage.changed", { usage: usage.claude });
    termOut(card.id, `usage: ${usage.claude.fiveHour.usedPercent}% / ${usage.claude.sevenDay.usedPercent}%\r\n$ `);
    return;
  }
  if (line && card.titleAuto && card.agentId !== "shell" && !promptedCards.has(card.id)) {
    // First prompt of the session names the card while the title is automatic.
    promptedCards.add(card.id);
    const t = autoTitle(line);
    if (t) {
      card.title = t;
      card.updatedAt = now();
      cardEvent(card);
    }
  }
  if (line && card.agentId !== "shell") {
    // Pretend the agent took a turn: user message, then a streamed reply that is extended once.
    chatAppend(card, [chatMsg("user", [{ type: "text", text: line }])]);
    const reply = chatMsg("assistant", [{ type: "text", text: `Working on: ${line}` }]);
    chatAppend(card, [reply]);
    if (card.context) {
      card.context.usedTokens = Math.min(card.context.windowTokens, card.context.usedTokens + 6000);
      card.context.percent = Math.round((card.context.usedTokens / card.context.windowTokens) * 1000) / 10;
    }
    setTimeout(() => {
      chatAppend(card, [{ ...reply, parts: [...reply.parts, { type: "text", text: "Finished." }] }]);
    }, 600);
  }
  if (line === "done") {
    termOut(card.id, "\x1b[32mTurn finished.\x1b[0m\r\n$ ");
    setStatus(card, "done", true);
    return;
  }
  if (line === "size") {
    termOut(card.id, `${sess.cols}x${sess.rows}\r\n$ `);
    return;
  }
  if (line) termOut(card.id, `echo: ${line}\r\n`);
  termOut(card.id, "$ ");
  if (card.status !== "running") setStatus(card, "running", false);
}

function endSession(card, exitCode) {
  const sess = sessions.get(card.id);
  if (!sess) return;
  sessions.delete(card.id);
  card.hasSession = false;
  card.status = "exited";
  card.statusSince = now();
  card.updatedAt = now();
  broadcast("session.exit", { cardId: card.id, exitCode });
  cardEvent(card);
}

function effectiveSize(sess) {
  let cols = Infinity;
  let rows = Infinity;
  for (const v of sess.viewers.values()) {
    if (v.watch) continue;
    cols = Math.min(cols, v.cols);
    rows = Math.min(rows, v.rows);
  }
  return Number.isFinite(cols) ? { cols, rows } : { cols: sess.cols, rows: sess.rows };
}

function applySize(cardId) {
  const sess = sessions.get(cardId);
  if (!sess) return;
  const { cols, rows } = effectiveSize(sess);
  if (cols !== sess.cols || rows !== sess.rows) {
    sess.cols = cols;
    sess.rows = rows;
    broadcast("session.size", { cardId, cols, rows });
  }
}

// ---------- wire ----------
const clients = new Set();

function broadcast(m, p) {
  const msg = JSON.stringify({ t: "ev", m, p });
  for (const c of clients) if (c.authed) c.ws.send(msg);
}
const cardEvent = (card) => broadcast("card.changed", { card });

class Fail extends Error {
  constructor(code, msg) {
    super(msg);
    this.code = code;
  }
}
const need = (cond, code, msg) => {
  if (!cond) throw new Fail(code, msg);
};
const getCard = (cid) => {
  const c = cards.get(cid);
  need(c, "not_found", `card ${cid} not found`);
  return c;
};
const nonEmpty = (v) => typeof v === "string" && v.trim().length > 0;

function renumber(list) {
  list.forEach((x, i) => {
    if (x.position !== i) {
      x.position = i;
      x._dirty = true;
    }
  });
}

const handlers = {
  "project.list": () => ({ projects: [...projects.values()].sort((a, b) => a.position - b.position) }),
  "project.add": ({ path, name, color }) => {
    need(nonEmpty(path), "invalid", "path is required");
    need(![...projects.values()].some((p) => p.path === path), "conflict", "project already added");
    const p = addProject(path, name, color);
    broadcast("project.changed", { project: p });
    for (const c of projColumns(p.id)) broadcast("column.changed", { column: c });
    return { project: p };
  },
  "project.update": ({ id: pid, name, color, position, worktreeSetup }) => {
    const p = projects.get(pid);
    need(p, "not_found", "project not found");
    if (name !== undefined) need(nonEmpty(name), "invalid", "name must not be empty"), (p.name = name);
    if (color !== undefined) p.color = color;
    if (worktreeSetup !== undefined) {
      if (worktreeSetup) p.worktreeSetup = worktreeSetup;
      else delete p.worktreeSetup;
    }
    if (position !== undefined) {
      const list = [...projects.values()].sort((a, b) => a.position - b.position).filter((x) => x !== p);
      list.splice(Math.max(0, Math.min(position, list.length)), 0, p);
      renumber(list);
      for (const x of list) if (x._dirty && x !== p) delete x._dirty, broadcast("project.changed", { project: x });
      delete p._dirty;
    }
    broadcast("project.changed", { project: p });
    return { project: p };
  },
  "project.remove": ({ id: pid }) => {
    need(projects.has(pid), "not_found", "project not found");
    need(![...cards.values()].some((c) => c.projectId === pid && c.hasSession), "conflict", "a card in this project still has a live session");
    for (const c of [...cards.values()]) if (c.projectId === pid) cards.delete(c.id);
    for (const c of [...columns.values()]) if (c.projectId === pid) columns.delete(c.id);
    projects.delete(pid);
    broadcast("project.changed", { removedId: pid });
    return {};
  },
  "board.get": ({ projectId }) => {
    const p = projects.get(projectId);
    need(p, "not_found", "project not found");
    const own = [...cards.values()].filter((c) => c.projectId === p.id);
    const ids = new Set(own.map((c) => c.id));
    const ls = [...links].map((k) => k.split("|")).filter(([a, b]) => ids.has(a) || ids.has(b)).map(([a, b]) => ({ a, b }));
    return { project: p, columns: projColumns(p.id), cards: own, links: ls };
  },
  "card.get": ({ id: cid }) => ({ card: getCard(cid) }),
  "stats.subscribe": ({ intervalMs }, client) => {
    const ms = Math.max(2000, intervalMs ?? 5000);
    statsSubs.set(client, ms);
    restartStats();
    return { intervalMs: ms };
  },
  "stats.unsubscribe": (_, client) => {
    statsSubs.delete(client);
    restartStats();
    return {};
  },
  "daemon.status": () => daemonStatus(),
  "settings.get": () => ({ settings }),
  "settings.update": ({ keepAwake, autoRestore, silenceSeconds }) => {
    if (typeof keepAwake === "boolean") settings = { ...settings, keepAwake };
    if (typeof autoRestore === "boolean") settings = { ...settings, autoRestore };
    if (silenceSeconds !== undefined) {
      need(Number.isInteger(silenceSeconds) && silenceSeconds >= 1 && silenceSeconds <= 600, "invalid", "silenceSeconds must be 1..600");
      settings = { ...settings, silenceSeconds };
    }
    broadcast("settings.changed", { settings });
    return { settings };
  },
  "link.add": ({ a, b }) => {
    getCard(a);
    getCard(b);
    need(a !== b, "invalid", "cannot link a card to itself");
    if (!links.has(linkKey(a, b))) {
      links.add(linkKey(a, b));
      const [x, y] = linkKey(a, b).split("|");
      broadcast("link.changed", { link: { a: x, b: y }, linked: true });
    }
    return {};
  },
  "link.remove": ({ a, b }) => {
    if (links.delete(linkKey(a, b))) {
      const [x, y] = linkKey(a, b).split("|");
      broadcast("link.changed", { link: { a: x, b: y }, linked: false });
    }
    return {};
  },
  "context.list": ({ cardId }) => {
    getCard(cardId);
    const out = [...links].map((k) => k.split("|")).flatMap(([a, b]) => (a === cardId ? [b] : b === cardId ? [a] : []));
    return { cards: out.map((id) => cards.get(id)).filter(Boolean) };
  },
  "context.read": ({ cardId, fromCardId, kind, limit }) => {
    const c = getCard(cardId);
    need(!fromCardId || fromCardId === cardId || links.has(linkKey(cardId, fromCardId)), "invalid", "cards are not linked");
    if (kind === "transcript") {
      const msgs = (transcripts.get(cardId) ?? []).slice(-(limit ?? 20));
      return { text: msgs.map((m) => `${m.role}: ${m.parts.filter((p) => p.type === "text").map((p) => p.text).join(" ")}`).join("\n") };
    }
    if (kind === "screen") return { text: (sessions.get(cardId)?.buf ?? "").replace(/\x1b\[[0-9;]*[A-Za-z]/g, "").slice(-4000) };
    if (kind === "notes") return { text: c.notes ?? "" };
    if (kind === "comments") {
      const list = comments.get(cardId) ?? [];
      const who = (a) => (a.kind === "agent" ? `agent ${a.agentId} (card ${a.cardId} "${a.cardTitle}")` : "user");
      return { text: list.length ? list.map((x) => `${who(x.author)}:\n${x.text}`).join("\n\n") : "(no comments)" };
    }
    return { text: `${c.title}\nagent: ${c.agentId}\nstatus: ${c.status}\ncwd: ${c.cwd}` };
  },
  "message.send": ({ cardId, fromCardId, text }) => {
    const c = getCard(cardId);
    need(typeof text === "string" && text.trim(), "invalid", "text is required");
    const m = { id: `msg-q${++msgIds}`, cardId, fromCardId: fromCardId ?? undefined, text, createdAt: now() };
    c.queuedMessages = [...(c.queuedMessages ?? []), m];
    c.updatedAt = now();
    cardEvent(c);
    deliverNext(c);
    return { message: m };
  },
  "comment.list": ({ cardId }) => {
    getCard(cardId);
    return { comments: comments.get(cardId) ?? [] };
  },
  "comment.add": ({ cardId, fromCardId, text }) => {
    const c = getCard(cardId);
    need(typeof text === "string" && text.trim(), "invalid", "empty comment");
    const comment = { id: `cm-${++commentIds}`, cardId, author: authorOf(fromCardId), text: text.trim(), createdAt: now() };
    comments.set(cardId, [...(comments.get(cardId) ?? []), comment]);
    c.commentCount = comments.get(cardId).length;
    if (comment.author.kind === "agent") c.unread = true;
    c.updatedAt = now();
    broadcast("comment.changed", { comment });
    cardEvent(c);
    return { comment };
  },
  "comment.delete": ({ id: mid }) => {
    const entry = [...comments.entries()].find(([, list]) => list.some((x) => x.id === mid));
    need(entry, "not_found", "no such comment");
    const [cardId, list] = entry;
    comments.set(cardId, list.filter((x) => x.id !== mid));
    const c = cards.get(cardId);
    if (c) {
      c.commentCount = comments.get(cardId).length;
      cardEvent(c);
    }
    broadcast("comment.changed", { removedId: mid, cardId });
    return {};
  },
  "message.cancel": ({ id: mid }) => {
    const c = [...cards.values()].find((x) => x.queuedMessages?.some((m) => m.id === mid));
    need(c, "conflict", "message already delivered or unknown");
    c.queuedMessages = c.queuedMessages.filter((m) => m.id !== mid);
    if (!c.queuedMessages.length) delete c.queuedMessages;
    c.updatedAt = now();
    cardEvent(c);
    return {};
  },
  "card.handoff": ({ id: cid, agentId }) => {
    const src = getCard(cid);
    const c = makeCard({ projectId: src.projectId, columnId: src.columnId, title: `Continue: ${src.title}`, agentId: agentId ?? src.agentId, cwd: src.cwd, branch: src.branch, prompt: `Read card ${src.id} with muster context read and continue its work.` });
    startSession(c, false);
    cardEvent(c);
    links.add(linkKey(src.id, c.id));
    const [x, y] = linkKey(src.id, c.id).split("|");
    broadcast("link.changed", { link: { a: x, b: y }, linked: true });
    return { card: c };
  },
  "column.create": ({ projectId, title, color, category }) => {
    need(projects.has(projectId), "not_found", "project not found");
    need(nonEmpty(title), "invalid", "title is required");
    const col = { id: id(), projectId, title, color: color || "#8e8e93", category: category || "started", position: projColumns(projectId).length };
    columns.set(col.id, col);
    broadcast("column.changed", { column: col });
    return { column: col };
  },
  "column.update": ({ id: cid, title, color, category, position }) => {
    const col = columns.get(cid);
    need(col, "not_found", "column not found");
    if (title !== undefined) need(nonEmpty(title), "invalid", "title must not be empty"), (col.title = title);
    if (color !== undefined) col.color = color;
    if (category !== undefined) {
      need(["unstarted", "started", "done", "closed"].includes(category), "invalid", "bad category");
      col.category = category;
    }
    if (position !== undefined) {
      const list = projColumns(col.projectId).filter((x) => x !== col);
      list.splice(Math.max(0, Math.min(position, list.length)), 0, col);
      renumber(list);
      for (const x of list) if (x._dirty && x !== col) delete x._dirty, broadcast("column.changed", { column: x });
      delete col._dirty;
    }
    broadcast("column.changed", { column: col });
    return { column: col };
  },
  "column.delete": ({ id: cid, moveCardsTo }) => {
    const col = columns.get(cid);
    need(col, "not_found", "column not found");
    const target = columns.get(moveCardsTo);
    need(target && target.projectId === col.projectId && target.id !== col.id, "invalid", "moveCardsTo must be another column of the same project");
    for (const c of colCards(cid)) {
      const last = colCards(target.id).at(-1);
      c.columnId = target.id;
      c.rank = rankBetween(last ? last.rank : "", null);
      c.updatedAt = now();
      cardEvent(c);
    }
    columns.delete(cid);
    const rest = projColumns(col.projectId);
    renumber(rest);
    for (const x of rest) if (x._dirty) delete x._dirty, broadcast("column.changed", { column: x });
    broadcast("column.changed", { removedId: cid, projectId: col.projectId });
    return {};
  },
  "card.create": ({ projectId, columnId, title, agentId, cwd, prompt, start, worktree }) => {
    const p = projects.get(projectId);
    need(p, "not_found", "project not found");
    const col = columns.get(columnId);
    need(col && col.projectId === projectId, "invalid", "column does not belong to project");
    need(AGENTS.some((a) => a.id === agentId) || String(agentId).startsWith("custom:"), "invalid", `unknown agent ${agentId}`);
    let branch = null;
    if (worktree) {
      need(nonEmpty(worktree.branch) && !/\s/.test(worktree.branch), "invalid", "worktree.branch must be a branch name");
      branch = worktree.branch;
      cwd = `${p.path}-worktrees/${branch.replace(/\//g, "-")}`;
    }
    const card = makeCard({ projectId, columnId, title, agentId, cwd, prompt, branch });
    if (worktree) worktreeCards.add(card.id);
    if (start) startSession(card, false);
    cardEvent(card);
    return { card };
  },
  "card.update": (params) => {
    const card = getCard(params.id);
    if (params.title !== undefined) need(nonEmpty(params.title), "invalid", "title must not be empty");
    if (params.priority != null) need(["low", "medium", "high", "urgent"].includes(params.priority), "invalid", "bad priority");
    if (params.notes !== undefined && params.notes !== card.notes) {
      card.notesEditedBy = authorOf(params.fromCardId);
      card.notesEditedAt = now();
    }
    for (const k of ["title", "prompt", "notes", "priority", "dueAt"]) {
      if (params[k] !== undefined) card[k] = params[k];
    }
    if (params.title !== undefined) card.titleAuto = false;
    if (params.titleAuto === true) card.titleAuto = true;
    card.updatedAt = now();
    cardEvent(card);
    return { card };
  },
  "card.move": ({ id: cid, columnId, before, after }) => {
    const card = getCard(cid);
    const col = columns.get(columnId);
    need(col && col.projectId === card.projectId, "invalid", "column does not belong to the card's project");
    const list = colCards(columnId).filter((c) => c.id !== cid);
    // As in muster-proto: `before` = place immediately before (above) that card,
    // `after` = place immediately after (below) that card; neither = append.
    const bi = before ? list.findIndex((c) => c.id === before) : -1;
    const ai = after ? list.findIndex((c) => c.id === after) : -1;
    need(!before || bi >= 0, "invalid", "before is not in the target column");
    need(!after || ai >= 0, "invalid", "after is not in the target column");
    need(!(before && after) || ai + 1 === bi, "invalid", "`after` and `before` must be adjacent cards");
    const pos = before ? bi : after ? ai + 1 : list.length;
    const lo = pos - 1;
    const hi = pos;
    const a = lo >= 0 ? list[lo].rank : "";
    const b = hi >= 0 && hi < list.length ? list[hi].rank : null;
    card.columnId = columnId;
    card.rank = rankBetween(a, b);
    card.updatedAt = now();
    cardEvent(card);
    return { card };
  },
  "card.delete": ({ id: cid, killSession }) => {
    const card = getCard(cid);
    if (card.hasSession) {
      need(killSession, "conflict", "card has a live session; pass killSession");
      endSession(card, null);
    }
    cards.delete(cid);
    comments.delete(cid);
    broadcast("card.changed", { removedId: cid, projectId: card.projectId });
    return {};
  },
  "card.start": ({ id: cid }) => {
    const card = getCard(cid);
    if (card.setup?.state === "failed") {
      // Replace the session and run the setup again.
      sessions.delete(card.id);
      startSession(card);
      return { card };
    }
    if (card.hasSession) return { card };
    if (card.restorable) {
      // Replay the saved scrollback, then resume the agent.
      card.restorable = false;
      startSession(card, false);
      termOut(card.id, "\x1b[2m--- restored from saved scrollback ---\x1b[0m\r\n");
      cardEvent(card);
      return { card };
    }
    startSession(card);
    return { card };
  },
  "card.markRead": ({ id: cid }) => {
    const card = getCard(cid);
    if (card.unread || card.status === "done") {
      card.unread = false;
      if (card.status === "done") {
        card.status = "idle";
        card.statusSince = now();
      }
      card.updatedAt = now();
      cardEvent(card);
    }
    return { card };
  },
  "session.attach": ({ cardId, cols, rows, watch }, client) => {
    getCard(cardId);
    const sess = sessions.get(cardId);
    need(sess, "not_found", "card has no session"); // matches musterd
    need(Number.isInteger(cols) && Number.isInteger(rows) && cols > 0 && rows > 0, "invalid", "bad size");
    sess.viewers.set(client, { cols, rows, watch: !!watch });
    client.attached.add(cardId);
    applySize(cardId);
    // Response first, then the repaint (the server sends both in order).
    queueMicrotask(() => client.ws.send(termFrame(cardId, Buffer.from("\x1bc" + sess.buf, "utf8"))));
    return { cols: sess.cols, rows: sess.rows };
  },
  "session.detach": ({ cardId }, client) => {
    const sess = sessions.get(cardId);
    client.attached.delete(cardId);
    if (sess) {
      sess.viewers.delete(client);
      applySize(cardId);
    }
    return {};
  },
  "session.kill": ({ cardId }) => {
    const card = getCard(cardId);
    need(card.hasSession, "unavailable", "card has no session");
    endSession(card, null);
    return {};
  },
  "term.write": ({ cardId, data }) => {
    const card = getCard(cardId);
    need(sessions.has(cardId), "unavailable", "card has no session");
    need(typeof data === "string", "invalid", "data must be a string");
    termInput(card, data);
    return {};
  },
  "term.resize": ({ cardId, cols, rows }, client) => {
    const sess = sessions.get(cardId);
    need(sess, "unavailable", "card has no session");
    const v = sess.viewers.get(client);
    need(v, "invalid", "not attached");
    v.cols = cols;
    v.rows = rows;
    applySize(cardId);
    return { cols: sess.cols, rows: sess.rows };
  },
  "agent.list": () => ({ agents: AGENTS }),
  "agent.accounts": () => ({ accounts: ACCOUNTS.map((a) => ({ ...a, checkedAt: now() })) }),
  "approval.answer": ({ cardId, requestId, decision, remember, message, answers, acceptEdits }) => {
    const card = getCard(cardId);
    need(sessions.has(cardId), "not_found", "card has no live session");
    need(card.status === "needs_you", "conflict", "card is not waiting");
    need(["allow", "deny", "answer", "approve_plan"].includes(decision), "invalid", "bad decision");
    const viaHook = !!(requestId && card.pendingApproval?.requestId === requestId);
    const kind = card.pendingApproval?.kind ?? "tool";
    if (kind !== "tool") {
      need(viaHook, "unavailable", "questions and plans can only be answered through the hook");
      if (decision === "answer") {
        need(kind === "question" && answers && typeof answers === "object", "invalid", "answers required");
        const text = Object.entries(answers).map(([q, a]) => `${q} → ${a.join(", ")}`).join("; ");
        termOut(card.id, `\x1b[32mAnswered\x1b[0m: ${text}\r\n$ `);
        setStatus(card, "running", false);
        return { via: "hook" };
      }
      if (decision === "approve_plan") {
        need(kind === "plan", "invalid", "not a plan");
        termOut(card.id, `\x1b[32mPlan approved\x1b[0m${acceptEdits ? " (accept edits)" : ""}\r\n$ `);
        setStatus(card, "running", false);
        return { via: "hook" };
      }
    }
    need(viaHook || !(decision === "allow" && remember), "unavailable", "no reliable key for allow + remember without a hook request");
    return { via: answerApproval(card, decision, viaHook ? "hook" : "keys", message) };
  },
  "chat.page": ({ cardId, before, limit }, client) => {
    getCard(cardId);
    const n = Math.max(1, Math.min(500, limit ?? 50));
    const list = transcripts.get(cardId) ?? [];
    let end = list.length;
    if (before) {
      const i = list.findIndex((m) => m.id === before);
      need(i >= 0, "invalid", "unknown cursor");
      end = i;
    } else {
      if (!chatSubs.has(cardId)) chatSubs.set(cardId, new Set());
      chatSubs.get(cardId).add(client);
      client.chats.add(cardId);
    }
    const start = Math.max(0, end - n);
    const messages = list.slice(start, end);
    return start > 0 ? { messages, older: list[start].id } : { messages };
  },
  "chat.unsubscribe": ({ cardId }, client) => {
    chatSubs.get(cardId)?.delete(client);
    client.chats.delete(cardId);
    return {};
  },
  "usage.get": () => ({ usage: usage.claude, accounts: Object.values(usage) }),
};

// ---------- server ----------
if (!EMPTY) seed();

const wss = new WebSocketServer({ host: "127.0.0.1", port: PORT, path: "/ws" });
wss.on("listening", () => {
  log(`listening on ws://127.0.0.1:${PORT}/ws token=${TOKEN}`);
  const home = process.env.MUSTER_HOME;
  if (home) {
    mkdirSync(home, { recursive: true });
    writeFileSync(join(home, "daemon.json"), JSON.stringify({ pid: process.pid, port: PORT }));
    writeFileSync(join(home, "token"), TOKEN + "\n", { mode: 0o600 });
    log(`wrote ${join(home, "daemon.json")}`);
    const cleanup = () => {
      try {
        rmSync(join(home, "daemon.json"));
      } catch {}
      process.exit(0);
    };
    process.on("SIGINT", cleanup);
    process.on("SIGTERM", cleanup);
  }
});

wss.on("connection", (ws, req) => {
  const client = { ws, authed: false, attached: new Set(), chats: new Set(), origin: req.headers.origin ?? "-" };
  clients.add(client);
  log(`client connected origin=${client.origin}`);
  ws.on("message", (raw, isBinary) => {
    if (isBinary) return;
    let f;
    try {
      f = JSON.parse(raw.toString());
    } catch {
      return;
    }
    if (f.t !== "req") return;
    const reply = (ok, body) => ws.send(JSON.stringify(ok ? { t: "res", id: f.id, ok: true, r: body } : { t: "res", id: f.id, ok: false, e: body }));
    if (!client.authed) {
      if (f.m === "hello" && f.p?.token === TOKEN) {
        client.authed = true;
        client.kind = f.p.client;
        log(`hello ok client=${f.p.client} origin=${client.origin}`);
        return reply(true, { protocol: 1, daemonVersion: VERSION });
      }
      log(`rejecting ${f.m} before hello (origin=${client.origin})`);
      reply(false, { code: "unauthorized", msg: f.m === "hello" ? "bad token" : "hello first" });
      return ws.close(4001, "unauthorized");
    }
    if (VERBOSE) log(`req ${f.m} ${JSON.stringify(f.p ?? {}).slice(0, 120)}`);
    const h = handlers[f.m];
    if (!h) return reply(false, { code: "invalid", msg: `unknown method ${f.m}` });
    try {
      reply(true, h(f.p ?? {}, client));
    } catch (e) {
      if (e instanceof Fail) reply(false, { code: e.code, msg: e.message });
      else {
        console.error(e);
        reply(false, { code: "internal", msg: String(e) });
      }
    }
  });
  ws.on("close", () => {
    clients.delete(client);
    for (const cid of client.chats) chatSubs.get(cid)?.delete(client);
    if (statsSubs.delete(client)) restartStats();
    for (const cid of client.attached) {
      const sess = sessions.get(cid);
      if (sess) {
        sess.viewers.delete(client);
        applySize(cid);
      }
    }
    log("client disconnected");
  });
});
