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
  "term.write": [{ cardId: Id; data: string }, Record<string, never>];
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
  "daemon.status": [Record<string, never>, DaemonStatus];
  "comment.list": [{ cardId: Id }, { comments: Comment[] }];
  "comment.add": [{ cardId: Id; text: string }, { comment: Comment }];
  "comment.delete": [{ id: Id }, Record<string, never>];
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
  "link.changed": { link: Link; linked: boolean };
  "daemon.status": DaemonStatus;
  "comment.changed": { comment: Comment } | { removedId: Id; cardId: Id };
}

export type EventName = keyof Events;

export type Frame =
  | { t: "req"; id: number; m: string; p: unknown }
  | { t: "res"; id: number; ok: true; r: unknown }
  | { t: "res"; id: number; ok: false; e: { code: string; msg?: string } }
  | { t: "ev"; m: string; p: unknown };
