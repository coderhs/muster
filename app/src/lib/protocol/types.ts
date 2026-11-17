// Wire types for the Muster daemon protocol v1 (docs/PROTOCOL.md).
// Field names are camelCase on the wire; timestamps are unix milliseconds.

export const PROTOCOL_VERSION = 1;

export type Id = string;

export type ColumnCategory = "unstarted" | "started" | "done" | "closed";
export type CardStatus = "draft" | "running" | "needs_you" | "done" | "idle" | "exited";
export type Priority = "low" | "medium" | "high" | "urgent";
export type ClientKind = "desktop" | "mobile" | "cli";

export interface Project {
  id: Id;
  name: string;
  path: string;
  color: string;
  position: number;
  /** Shell script run in each new worktree before its agent starts. */
  worktreeSetup?: string | null;
}

export interface Column {
  id: Id;
  projectId: Id;
  title: string;
  color: string;
  category: ColumnCategory;
  position: number;
}

export interface Card {
  id: Id;
  projectId: Id;
  columnId: Id;
  rank: string;
  title: string;
  agentId: string;
  cwd: string;
  branch?: string | null;
  prompt?: string | null;
  notes?: string | null;
  priority?: Priority | null;
  dueAt?: number | null;
  agentSessionId?: string | null;
  status: CardStatus;
  statusSince: number;
  turnStartedAt?: number | null;
  lastActivityAt?: number | null;
  unread: boolean;
  hasSession: boolean;
  createdAt: number;
  updatedAt: number;
  // Phase 3 (absent for agents without hooks):
  needsYouReason?: NeedsYouReason | null;
  pendingApproval?: PendingApproval | null;
  context?: ContextInfo | null;
  /** True while the daemon maintains the title. */
  titleAuto?: boolean;
  /** Worktree setup script state; absent when none runs or it succeeded. */
  setup?: SetupInfo | null;
  /** The session vanished without exiting (reboot); card.start restores it. */
  restorable?: boolean;
  queuedMessages?: Message[] | null;
  /** Number of comments on the card; `comment.list` returns them. */
  commentCount?: number;
  /** Who last changed the notes, and when. */
  notesEditedBy?: Author | null;
  notesEditedAt?: number | null;
  /** Short name of the model the agent last reported ("Opus 5.5", "gpt-5.5"). */
  model?: string | null;
  /** The user exempted this card from being stopped when idle. */
  keepRunning?: boolean;
  /** When Muster stopped the session for being idle; cleared when it starts again. */
  idleStoppedAt?: number | null;
  /** Why an idle session was left running at the last check ("has a scheduled job"). */
  idleKept?: string | null;
  /** The start of the agent's last reply, on one line, from when its latest turn ended. */
  lastMessage?: string | null;
}

/** Who wrote a comment or edited notes. Set by the daemon, never by the client. */
export interface Author {
  kind: "user" | "agent";
  /** For an agent: its card, agent id and the card's title when it wrote. */
  cardId?: Id | null;
  agentId?: string | null;
  cardTitle?: string | null;
}

export interface Comment {
  id: Id;
  cardId: Id;
  author: Author;
  text: string;
  createdAt: number;
}

export interface Link {
  a: Id;
  b: Id;
}

export interface Message {
  id: Id;
  cardId: Id;
  fromCardId?: Id | null;
  text: string;
  createdAt: number;
  deliveredAt?: number | null;
}

export interface Settings {
  keepAwake: boolean;
  autoRestore: boolean;
  /** Seconds of terminal silence before a card without hooks counts as idle (1..600). */
  silenceSeconds?: number;
  /** Whether programs in cards may set the Mac clipboard (OSC 52). */
  clipboardPassthrough?: boolean;
  /** Stop a session idle this many minutes, leaving a card to resume. 0 = never. */
  idleStopMinutes?: number;
}

/** A phone, tablet or computer paired for remote access. */
export interface Device {
  id: Id;
  name: string;
  createdAt: number;
  lastSeenAt?: number | null;
}

export interface RemoteStatus {
  enabled: boolean;
  port: number;
  httpsPort: number;
  /** The address devices open, once Tailscale serves it. */
  url?: string | null;
  /** The built web app was found next to the daemon. */
  webUi: boolean;
  tailscale: { found: boolean; running: boolean; dnsName?: string | null; https: boolean };
  /** Why remote access is not working, while it is on. */
  error?: string | null;
  devices: Device[];
}

export interface ProcStats {
  memBytes: number;
  cpuPercent: number;
  processes: number;
}

export interface StatsSnapshot {
  cards: Array<ProcStats & { cardId: Id }>;
  total: ProcStats;
  daemon: ProcStats;
  at: number;
}

export interface SetupInfo {
  state: "running" | "failed";
  exitCode?: number | null;
}

export type NeedsYouReason =
  | "permission"
  | "question"
  | "input"
  | "notification"
  | "startup_prompt"
  | "terminal_prompt"
  | "setup_failed"
  | "plan";

export interface PendingApproval {
  /** Set when the request can be answered through the hook; otherwise only terminal keys can. */
  requestId?: string | null;
  tool: string;
  summary: string;
  since: number;
  /** "tool" when absent. */
  kind?: "tool" | "question" | "plan";
  questions?: Question[] | null;
  plan?: string | null;
  planFilePath?: string | null;
}

export interface Question {
  question: string;
  header?: string | null;
  options: Array<{ label: string; description?: string | null }>;
  multiSelect: boolean;
}

export interface AgentCapabilities {
  statusSource: "hooks" | "log" | "activity";
  approvals: boolean;
  questions: boolean;
  chat: boolean;
  context: boolean;
  usage: boolean;
  resume: boolean;
  /** `card.aiRename` works for this agent. Absent from older daemons. */
  aiRename?: boolean;
}

export interface Notice {
  id: string;
  level: "warning" | "info";
  message: string;
}

export interface DaemonStatus {
  version: string;
  /** Short git commit the daemon was built from ("+" for uncommitted changes), and its date. */
  commit?: string;
  commitDate?: string;
  pid: number;
  home: string;
  tmuxSocket: string;
  hooksAvailable: boolean;
  notices: Notice[];
}

export interface ContextInfo {
  usedTokens: number;
  windowTokens?: number | null;
  percent?: number | null;
}

export type ChatPart =
  | { type: "text"; text: string }
  | { type: "thinking" }
  | { type: "toolCall"; id: string; name: string; summary: string }
  | { type: "toolResult"; toolUseId: string; isError: boolean; summary: string }
  | { type: "image" }
  | { type: "interrupted" };

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "tool";
  timestamp?: number | null;
  model?: string | null;
  parts: ChatPart[];
}

export interface UsageWindow {
  usedPercent: number;
  resetsAt?: number | null;
}

export interface Usage {
  fiveHour?: UsageWindow | null;
  sevenDay?: UsageWindow | null;
  /** Every window the agent reported, by its own name. */
  windows?: Array<UsageWindow & { name: string; windowMinutes?: number | null }> | null;
  agentId?: string | null;
  updatedAt: number;
}

/** Sign-in state and defaults of one agent CLI, from local state only. */
export interface AgentAccount {
  agentId: string;
  installed: boolean;
  /** Absent when Muster cannot tell. */
  signedIn?: boolean | null;
  method?: string | null;
  detail?: string | null;
  defaultModel?: string | null;
  /** Why usage limits are not shown for this agent. */
  usageNote?: string | null;
  checkedAt: number;
}

export interface Agent {
  id: string;
  label: string;
  color: string;
  installed: boolean;
  capabilities?: AgentCapabilities | null;
}

export type ErrorCode =
  | "unauthorized"
  | "not_found"
  | "invalid"
  | "conflict"
  | "unavailable"
  | "internal";

/** Method name -> [params, result]. */
export interface Methods {
  hello: [{ token: string; client: ClientKind; protocol: number }, { protocol: number; daemonVersion: string }];
  "project.list": [Record<string, never>, { projects: Project[] }];
  "project.add": [{ path: string; name?: string; color?: string }, { project: Project }];
  "project.update": [
    { id: Id; name?: string; color?: string; position?: number; worktreeSetup?: string | null },
    { project: Project },
  ];
  "project.remove": [{ id: Id }, Record<string, never>];
  "board.get": [{ projectId: Id }, { project: Project; columns: Column[]; cards: Card[]; links?: Link[] }];
  "column.create": [
    { projectId: Id; title: string; color?: string; category?: ColumnCategory },
    { column: Column },
  ];
  "column.update": [
    { id: Id; title?: string; color?: string; category?: ColumnCategory; position?: number },
    { column: Column },
  ];
  "column.delete": [{ id: Id; moveCardsTo: Id }, Record<string, never>];
  "card.create": [
    {
      projectId: Id;
      columnId: Id;
      title?: string;
      agentId: string;
      cwd?: string;
      prompt?: string;
      start?: boolean;
      worktree?: { branch: string };
      resumeSessionId?: string;
    },
    { card: Card },
  ];
  "card.update": [
    {
      id: Id;
      title?: string;
      prompt?: string;
      notes?: string;
      priority?: Priority | null;
      dueAt?: number | null;
      titleAuto?: boolean;
      keepRunning?: boolean;
    },
    { card: Card },
  ];
  "card.move": [{ id: Id; columnId: Id; before?: Id; after?: Id }, { card: Card }];
  "card.delete": [{ id: Id; killSession?: boolean }, Record<string, never>];
  "card.start": [{ id: Id }, { card: Card }];
  "card.markRead": [{ id: Id }, { card: Card }];
  "session.attach": [{ cardId: Id; cols: number; rows: number; watch?: boolean }, { cols: number; rows: number }];
  "session.detach": [{ cardId: Id }, Record<string, never>];
  "session.kill": [{ cardId: Id }, Record<string, never>];
  /** `raw` delivers the bytes as they are, past the session's terminal client (see keys.ts). */
  "term.write": [{ cardId: Id; data: string; raw?: boolean }, Record<string, never>];
  "term.resize": [{ cardId: Id; cols: number; rows: number }, { cols: number; rows: number }];
  "agent.list": [Record<string, never>, { agents: Agent[] }];
  "approval.answer": [
    {
      cardId: Id;
      requestId?: string;
      decision: "allow" | "deny" | "answer" | "approve_plan";
      remember?: boolean;
      message?: string;
      answers?: Record<string, string[]>;
      acceptEdits?: boolean;
    },
    { via: "hook" | "keys" },
  ];
  "chat.page": [{ cardId: Id; before?: string; limit?: number }, { messages: ChatMessage[]; older?: string | null }];
  "chat.unsubscribe": [{ cardId: Id }, Record<string, never>];
  "usage.get": [Record<string, never>, { usage?: Usage | null; accounts?: Usage[] }];
  "agent.accounts": [{ refresh?: boolean }, { accounts: AgentAccount[] }];
  "card.get": [{ id: Id }, { card: Card }];
  "stats.subscribe": [{ intervalMs?: number }, { intervalMs: number }];
  "stats.unsubscribe": [Record<string, never>, Record<string, never>];
  "settings.get": [Record<string, never>, { settings: Settings }];
  "settings.update": [Partial<Settings>, { settings: Settings }];
  "remote.status": [Record<string, never>, RemoteStatus];
  "remote.set": [{ enabled: boolean }, RemoteStatus];
  "remote.pair": [Record<string, never>, { url: string; expiresAt: number }];
  "remote.revoke": [{ id: Id }, RemoteStatus];
  "link.add": [Link, Record<string, never>];
  "link.remove": [Link, Record<string, never>];
  "context.list": [{ cardId: Id }, { cards: Card[] }];
  "context.read": [
    { cardId: Id; fromCardId?: Id; kind: "transcript" | "screen" | "notes" | "summary" | "comments"; limit?: number },
    { text: string },
  ];
  "message.send": [{ cardId: Id; fromCardId?: Id; text: string }, { message: Message }];
  "message.cancel": [{ id: Id }, Record<string, never>];
  "card.handoff": [{ id: Id; agentId?: string }, { card: Card }];
  "card.aiRename": [{ id: Id }, { card: Card }];
  "card.changes": [{ id: Id; base?: string; scope?: ChangeScope }, CardChanges];
  "card.diff": [{ id: Id; path: string; base?: string; scope?: ChangeScope }, { path: string; patch: string; truncated?: boolean }];
  "daemon.status": [Record<string, never>, DaemonStatus];
  "comment.list": [{ cardId: Id }, { comments: Comment[] }];
  "comment.add": [{ cardId: Id; text: string }, { comment: Comment }];
  "comment.delete": [{ id: Id }, Record<string, never>];
  "docs.list": [{ projectId: Id }, { docs: DocEntry[]; truncated?: boolean }];
  "docs.read": [{ projectId: Id; path: string }, { path: string; content: string; modifiedAt: number; truncated?: boolean }];
  /** `create` makes a new file; `expectedModifiedAt` refuses the save when the file changed since it was read. */
  "docs.write": [{ projectId: Id; path: string; content: string; create?: boolean; expectedModifiedAt?: number }, { doc: DocEntry }];
  /** `from` inclusive, `to` exclusive (ms); `splits` cut the range into the parts returned in `days`. */
  "project.summary": [{ projectId: Id; from: number; to: number; splits?: number[] }, ProjectSummary];
  /** Starts writing the summaries of a range and returns at once; `summary.changed` follows the job. */
  "project.summarize": [{ projectId: Id; from: number; to: number }, { started: boolean }];
}

/** How far a `project.summarize` job is. */
export interface SummaryProgress {
  done: number;
  total: number;
}

/** How a card's last run in a range ended; `idle` is a run that stopped without a finished turn. */
export type SummaryOutcome = "working" | "needs_you" | "finished" | "idle" | "not_started";
export type SummarySessionEnd = "exited" | "idle_stopped";

/** What happened on one card inside a range. */
export interface SummaryCard {
  id: Id;
  title: string;
  agentId: string;
  model?: string;
  columnId: Id;
  columnTitle: string;
  /** The card's status now, not at the end of the range. */
  status: CardStatus;
  hasSession: boolean;
  firstAt: number;
  lastAt: number;
  /** Time spent running inside the range. */
  workingMs: number;
  runs: number;
  outcome: SummaryOutcome;
  sessionEnd?: SummarySessionEnd;
  created?: boolean;
  /** A few sentences on what was done, written on request. Only on the whole range's cards, not inside `days`. */
  summary?: string;
  /** When `summary` was written, or when the card was found to have nothing to summarise. */
  summaryAt?: number;
  /** The card ran again after its conversation was read for the summary. */
  summaryStale?: boolean;
  /** The card had no conversation in the range that could be read. */
  summaryNone?: boolean;
}

export interface SummaryTotals {
  /** Cards with at least one run. */
  cards: number;
  workingMs: number;
  finished: number;
  created: number;
}

export interface SummaryDay {
  from: number;
  to: number;
  cards: SummaryCard[];
  totals: SummaryTotals;
}

export interface SummaryActive {
  id: Id;
  title: string;
  agentId: string;
  model?: string;
  columnTitle: string;
  status: CardStatus;
  statusSince: number;
  hasSession: boolean;
}

export interface ProjectSummary {
  from: number;
  to: number;
  /** Most recent first. */
  cards: SummaryCard[];
  totals: SummaryTotals;
  /** Most recent first, without the empty ones; absent when no `splits` were sent. */
  days?: SummaryDay[];
  active: SummaryActive[];
  /** A paragraph on the whole range, written on request. */
  text?: string;
  textAt?: number;
  /** Work went on, or a card's summary was rewritten, after `text` was written. */
  textStale?: boolean;
  /** Present while a job for this range is running. */
  summarizing?: SummaryProgress;
}

/** `claude` is CLAUDE.md, `agents` is AGENTS.md, `doc` any other Markdown file. */
export type DocKind = "claude" | "agents" | "doc";

/** A Markdown file of a project; `path` is relative to the project's folder. */
export type ChangeScope = "all" | "committed" | "uncommitted";

export type ChangeKind = "added" | "modified" | "deleted" | "renamed" | "untracked";

export interface ChangedFile {
  path: string;
  oldPath?: string;
  kind: ChangeKind;
  additions: number;
  deletions: number;
  binary?: boolean;
}

export interface CardChanges {
  root: string;
  branch?: string;
  /** A branch name, or "HEAD" when only uncommitted changes are shown. */
  base: string;
  /** The base used when none is chosen. */
  defaultBase: string;
  /** Branches that can be chosen as the base, most recently changed first. */
  branches: string[];
  scope: ChangeScope;
  baseCommit: string;
  commits: number;
  uncommitted: boolean;
  files: ChangedFile[];
  truncated?: boolean;
}

export interface DocEntry {
  path: string;
  kind: DocKind;
  size: number;
  modifiedAt: number;
}

export type Method = keyof Methods;
export type Params<M extends Method> = Methods[M][0];
export type Result<M extends Method> = Methods[M][1];

/** Event name -> payload. */
export interface Events {
  "project.changed": { project: Project } | { removedId: Id };
  "column.changed": { column: Column } | { removedId: Id; projectId: Id };
  "card.changed": { card: Card } | { removedId: Id; projectId: Id };
  "session.size": { cardId: Id; cols: number; rows: number };
  "session.exit": { cardId: Id; exitCode?: number | null };
  "chat.appended": { cardId: Id; messages: ChatMessage[] };
  "usage.changed": { usage: Usage };
  "stats.changed": StatsSnapshot;
  "settings.changed": { settings: Settings };
  "remote.changed": RemoteStatus;
  "link.changed": { link: Link; linked: boolean };
  "daemon.status": DaemonStatus;
  "comment.changed": { comment: Comment } | { removedId: Id; cardId: Id };
  /** Progress of a `project.summarize` job; `running` is false on its last event, `error` says why it stopped early. */
  "summary.changed": { projectId: Id; from: number; to: number; done: number; total: number; running: boolean; error?: string };
}

export type EventName = keyof Events;

export type Frame =
  | { t: "req"; id: number; m: string; p: unknown }
  | { t: "res"; id: number; ok: true; r: unknown }
  | { t: "res"; id: number; ok: false; e: { code: string; msg?: string } }
  | { t: "ev"; m: string; p: unknown };
