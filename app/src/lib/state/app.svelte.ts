// Application state: connection, projects, agents and the current board. Board
// mutations go through the pure reducer in ../board/board.ts.

import {
  beginMove,
  cardChanged,
  cardsByColumn,
  columnChanged,
  emptyBoard,
  endMove,
  loadBoard,
  projectChanged,
  sortedColumns,
  type BoardState,
} from "../board/board";
import { MusterClient, ProtocolError, errText, type ConnState } from "../protocol/client";
import type {
  Agent,
  Card,
  Column,
  Id,
  Link,
  Method,
  Params,
  ProcStats,
  Project,
  Result,
  Settings,
  Usage,
  AgentAccount,
  DaemonStatus,
} from "../protocol/types";
import type { Platform } from "../connection";

export interface Toast {
  id: number;
  text: string;
  kind: "error" | "info";
}

/** The tabs of the main window. The tracker covers all projects; the others one. */
export const VIEWS = ["board", "tracker", "docs"] as const;
export type View = (typeof VIEWS)[number];

const PROJECT_KEY = "muster.project";
const VIEW_KEY = "muster.view";
const SIDEBAR_KEY = "muster.sidebar.collapsed";
const CARD_SIDE_KEY = "muster.modal.side.collapsed";

function remember(key: string, v: string | null) {
  try {
    if (v) localStorage.setItem(key, v);
    else localStorage.removeItem(key);
  } catch {
    /* ignore */
  }
}
function recall(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export const SHELL_AGENT: Agent = { id: "shell", label: "Shell", color: "#8e8e93", installed: true };

export class AppState {
  conn = $state<ConnState>({ kind: "idle" });
  projects = $state.raw<Project[]>([]);
  agents = $state.raw<Agent[]>([]);
  board = $state.raw<BoardState>(emptyBoard());
  currentProjectId = $state<Id | null>(recall(PROJECT_KEY));
  selectedCardId = $state<Id | null>(null);
  openCardId = $state<Id | null>(null);
  newCard = $state<{ columnId: Id | null } | null>(null);
  /** The one-step agent picker, anchored to the button that opened it. */
  quick = $state<{ anchor: HTMLElement; columnId: Id | null } | null>(null);
  projectSettings = $state<Id | null>(null);
  /** Set by a quick start so the modal opens on a focused terminal. */
  focusTerminal = $state(false);
  /** Every card of every project, kept current from `card.changed`; feeds the tracker. */
  allCards = $state.raw<Record<Id, Card>>({});
  /** Account limits per agent id, merged by the daemon from every card's reports. */
  usage = $state.raw<Record<string, Usage>>({});
  /** Sign-in state per agent CLI; loaded when the tracker opens. */
  accounts = $state.raw<AgentAccount[] | null>(null);
  settings = $state.raw<Settings | null>(null);
  daemonStatus = $state.raw<DaemonStatus | null>(null);
  /** Notices the user dismissed this session. */
  dismissed = $state.raw<Set<string>>(new Set());
  settingsOpen = $state(false);
  /** Undirected card links, keyed "a|b" with a < b. */
  links = $state.raw<Set<string>>(new Set());
  stats = $state.raw<Map<Id, ProcStats>>(new Map());
  statsTotal = $state.raw<ProcStats | null>(null);
  statsDaemon = $state.raw<ProcStats | null>(null);
  /** Bumped to ask the sidebar to start adding a project (browser path entry). */
  addProjectRequest = $state(0);
  shortcutsOpen = $state(false);
  cardMenu = $state<{ cardId: Id; anchor: HTMLElement } | null>(null);
  /** A card picker for linking, opened from a card. */
  linking = $state<{ cardId: Id; anchor: HTMLElement } | null>(null);
  view = $state<View>(VIEWS.find((v) => v === recall(VIEW_KEY)) ?? "board");
  /** The projects sidebar is hidden (wide windows only; narrow ones use the drawer). */
  sidebarCollapsed = $state(recall(SIDEBAR_KEY) === "1");
  /** The details column of an open card is hidden, leaving the terminal the full width. */
  cardSideCollapsed = $state(recall(CARD_SIDE_KEY) === "1");
  toasts = $state<Toast[]>([]);
  /** Bumped after each successful (re)connect so views can re-sync. */
  epoch = $state(0);

  columns = $derived(sortedColumns(this.board));
  byColumn = $derived(cardsByColumn(this.board));
  currentProject = $derived(this.projects.find((p) => p.id === this.currentProjectId) ?? null);
  needsYouByProject = $derived.by(() => {
    const m = new Map<Id, number>();
    for (const c of Object.values(this.allCards)) if (c.status === "needs_you") m.set(c.projectId, (m.get(c.projectId) ?? 0) + 1);
    return m;
  });
  agentsWithShell = $derived(
    this.agents.some((a) => a.id === "shell") ? this.agents : [...this.agents, SHELL_AGENT],
  );

  readonly client: MusterClient;
  private toastSeq = 0;
  private boardSeq = 0;

  constructor(public readonly platform: Platform) {
    this.client = new MusterClient({
      client: "desktop",
      resolve: (lastError) => platform.resolve(lastError),
    });
    this.client.onState((s) => (this.conn = s));
    this.client.on("project.changed", (p) => {
      if ("removedId" in p) {
        this.projects = this.projects.filter((x) => x.id !== p.removedId);
        const all = { ...this.allCards };
        for (const c of Object.values(all)) if (c.projectId === p.removedId) delete all[c.id];
        this.allCards = all;
        if (this.currentProjectId === p.removedId) this.selectProject(this.projects[0]?.id ?? null);
      } else {
        const i = this.projects.findIndex((x) => x.id === p.project.id);
        const next = [...this.projects];
        if (i >= 0) next[i] = p.project;
        else next.push(p.project);
        next.sort((a, b) => a.position - b.position);
        this.projects = next;
      }
      this.board = projectChanged(this.board, p);
    });
    this.client.on("column.changed", (p) => (this.board = columnChanged(this.board, p)));
    this.client.on("card.changed", (p) => {
      if ("removedId" in p) this.removeCard(p.removedId, p.projectId);
      else this.applyCard(p.card);
    });
    this.client.on("usage.changed", (p) => (this.usage = { ...this.usage, [p.usage.agentId ?? "claude"]: p.usage }));
    this.client.on("settings.changed", (p) => (this.settings = p.settings));
    this.client.on("daemon.status", (p) => (this.daemonStatus = p));
    this.client.on("link.changed", (p) => {
      const next = new Set(this.links);
      const k = linkKey(p.link.a, p.link.b);
      if (p.linked) next.add(k);
      else next.delete(k);
      this.links = next;
    });
    this.client.on("stats.changed", (p) => {
      this.stats = new Map(p.cards.map((c) => [c.cardId, c]));
      this.statsTotal = p.total;
      this.statsDaemon = p.daemon;
    });
    this.client.onState((s) => {
      if (s.kind !== "ready") this.statsSubscribed = false;
    });
    if (typeof document !== "undefined") document.addEventListener("visibilitychange", () => this.syncStats());
    this.client.onReady(() => void this.resync());
  }

  start() {
    this.client.start();
  }

  card(id: Id | null): Card | null {
    if (!id) return null;
    return this.board.cards[id] ?? this.allCards[id] ?? null;
  }

  agent(id: string): Agent {
    return (
      this.agentsWithShell.find((a) => a.id === id) ?? {
        id,
        label: id.startsWith("custom:") ? id.slice(7) : id,
        color: "#8e8e93",
        installed: true,
      }
    );
  }

  /** Request wrapper that reports failures as a toast and rethrows. */
  async call<M extends Method>(m: M, p: Params<M>, quiet = false): Promise<Result<M>> {
    try {
      return await this.client.request(m, p);
    } catch (e) {
      if (!quiet) this.toast(describeError(m, e));
      throw e;
    }
  }

  toast(text: string, kind: Toast["kind"] = "error") {
    const id = ++this.toastSeq;
    this.toasts = [...this.toasts, { id, text, kind }];
    setTimeout(() => (this.toasts = this.toasts.filter((t) => t.id !== id)), kind === "error" ? 6000 : 3000);
  }

  private async resync() {
    try {
      const [{ projects }, agents] = await Promise.all([
        this.client.request("project.list", {}),
        this.client.request("agent.list", {}).catch(() => ({ agents: [] as Agent[] })),
      ]);
      this.projects = [...projects].sort((a, b) => a.position - b.position);
      this.agents = agents.agents;
      void this.loadAllCards(projects.map((p) => p.id));
      this.client
        .request("daemon.status", {})
        .then((r) => (this.daemonStatus = r))
        .catch(() => {});
      this.client
        .request("settings.get", {})
        .then((r) => (this.settings = r.settings))
        .catch(() => {});
      this.syncStats();
      // Older daemons do not know usage.get; that is fine.
      this.client
        .request("usage.get", {})
        .then((r) => {
          // Older daemons send one snapshot; newer ones one per agent.
          const list = r.accounts?.length ? r.accounts : r.usage ? [r.usage] : [];
          this.usage = Object.fromEntries(list.map((u) => [u.agentId ?? "claude", u]));
        })
        .catch(() => {});
      const want = projects.some((p) => p.id === this.currentProjectId)
        ? this.currentProjectId
        : (this.projects[0]?.id ?? null);
      await this.selectProject(want, true);
      this.epoch++;
    } catch (e) {
      this.toast(`Could not load state: ${errText(e)}`);
    }
  }

  toggleSidebar() {
    this.sidebarCollapsed = !this.sidebarCollapsed;
    remember(SIDEBAR_KEY, this.sidebarCollapsed ? "1" : null);
  }

  toggleCardSide() {
    this.cardSideCollapsed = !this.cardSideCollapsed;
    remember(CARD_SIDE_KEY, this.cardSideCollapsed ? "1" : null);
  }

  setView(v: View) {
    this.view = v;
    remember(VIEW_KEY, v);
  }

  /** Open a card from anywhere; switches to its project first so the modal has its columns. */
  private pendingOpen: Id | null = null;

  lastAgent(projectId: Id | null): string | null {
    return projectId ? recall(`muster.lastAgent.${projectId}`) : null;
  }

  /** Default column for new cards: the given one, else the first "not started" column. */
  defaultColumn(columnId: Id | null): Id | null {
    if (columnId && this.board.columns[columnId]) return columnId;
    return (this.columns.find((c) => c.category === "unstarted") ?? this.columns[0])?.id ?? null;
  }

  /** One step: create a started card with no title or prompt and open its terminal. */
  async quickStart(agentId: string, columnId: Id | null) {
    const projectId = this.currentProjectId;
    const col = this.defaultColumn(columnId);
    if (!projectId || !col) return;
    remember(`muster.lastAgent.${projectId}`, agentId);
    const { card } = await this.call("card.create", { projectId, columnId: col, agentId, start: true });
    this.applyCard(card);
    this.focusTerminal = true;
    await this.openCard(card.id);
  }

  async openCard(id: Id) {
    const c = this.card(id);
    if (!c && !this.board.loaded) {
      // Not loaded yet (app just started): open once the boards arrive.
      this.pendingOpen = id;
      return;
    }
    if (c && c.projectId !== this.currentProjectId) await this.selectProject(c.projectId);
    this.selectedCardId = id;
    this.openCardId = id;
  }

  private async loadAllCards(projectIds: Id[]) {
    const results = await Promise.all(
      projectIds.map((projectId) => this.client.request("board.get", { projectId }).catch(() => null)),
    );
    const all: Record<Id, Card> = {};
    const links = new Set<string>();
    for (const r of results) {
      if (!r) continue;
      for (const c of r.cards) all[c.id] = c;
      for (const l of r.links ?? []) links.add(linkKey(l.a, l.b));
    }
    this.links = links;
    // Keep anything newer that arrived by event while the boards were loading.
    for (const c of Object.values(this.allCards)) if (all[c.id] && all[c.id].updatedAt < c.updatedAt) all[c.id] = c;
    this.allCards = all;
    if (this.pendingOpen && all[this.pendingOpen]) {
      const id = this.pendingOpen;
      this.pendingOpen = null;
      void this.openCard(id);
    }
  }

  async selectProject(id: Id | null, force = false) {
    if (id === this.currentProjectId && !force && this.board.loaded) return;
    const changed = id !== this.currentProjectId;
    this.currentProjectId = id;
    remember(PROJECT_KEY, id);
    if (changed) {
      this.selectedCardId = null;
      this.openCardId = null;
      this.board = emptyBoard(id);
    }
    if (!id) {
      this.board = emptyBoard(null);
      return;
    }
    const seq = ++this.boardSeq;
    try {
      const data = await this.client.request("board.get", { projectId: id });
      if (seq !== this.boardSeq || this.currentProjectId !== id) return;
      this.board = loadBoard(this.board.projectId === id ? this.board : emptyBoard(id), data);
      const all = { ...this.allCards };
      for (const c of data.cards) if (!all[c.id] || all[c.id].updatedAt <= c.updatedAt) all[c.id] = c;
      this.allCards = all;
    } catch (e) {
      if (seq === this.boardSeq) this.toast(describeError("board.get", e));
    }
  }

  // ---- projects ----
  /** Folder picker in the desktop app; path entry in the sidebar otherwise. */
  async requestAddProject() {
    if (!this.platform.pickFolder) {
      this.addProjectRequest++;
      return;
    }
    try {
      const path = await this.platform.pickFolder();
      if (path) await this.addProject(path);
    } catch (e) {
      if (!(e instanceof ProtocolError)) this.toast(`Could not open the folder picker: ${errText(e)}`);
    }
  }

  async addProject(path: string) {
    const { project } = await this.call("project.add", { path });
    if (!this.projects.some((p) => p.id === project.id)) this.projects = [...this.projects, project];
    await this.selectProject(project.id);
  }

  // ---- cards ----
  /** Optimistic move; reconciles from the response and later `card.changed` events. */
  async moveCard(cardId: Id, columnId: Id, before?: Id, after?: Id) {
    const { state, token } = beginMove(this.board, cardId, columnId, before, after);
    this.board = state;
    try {
      const { card } = await this.client.request("card.move", { id: cardId, columnId, before, after });
      this.board = endMove(this.board, cardId, token, card);
      this.applyCard(card);
    } catch (e) {
      this.board = endMove(this.board, cardId, token, null);
      this.toast(describeError("card.move", e));
    }
  }

  async updateCard(p: Params<"card.update">) {
    const { card } = await this.call("card.update", p);
    this.applyCard(card);
    return card;
  }

  async startCard(id: Id) {
    const { card } = await this.call("card.start", { id });
    this.applyCard(card);
    return card;
  }

  async deleteCard(id: Id) {
    const c = this.card(id);
    await this.call("card.delete", { id, killSession: !!c?.hasSession });
    if (c) this.removeCard(id, c.projectId);
  }

  /** Answer a question or plan prompt (hook only; no terminal fallback). */
  async answerPrompt(
    card: Card,
    p: { decision: "answer" | "approve_plan" | "deny"; answers?: Record<string, string[]>; acceptEdits?: boolean; message?: string },
  ) {
    return this.call("approval.answer", { cardId: card.id, requestId: card.pendingApproval?.requestId ?? undefined, ...p });
  }

  /** Answer a pending permission request; returns how the daemon delivered it. */
  async answerApproval(card: Card, decision: "allow" | "deny", opts: { remember?: boolean; message?: string } = {}) {
    const requestId = card.pendingApproval?.requestId ?? undefined;
    const r = await this.call("approval.answer", {
      cardId: card.id,
      requestId,
      decision,
      remember: opts.remember || undefined,
      message: opts.message || undefined,
    });
    if (r.via === "keys") this.toast(decision === "allow" ? "Allowed by typing into the terminal" : "Denied (Escape typed into the terminal)", "info");
    return r.via;
  }

  async markRead(id: Id) {
    const { card } = await this.call("card.markRead", { id }, true);
    this.applyCard(card);
  }

  /** Asks which agent CLIs are signed in. Older daemons do not know the method. */
  loadAccounts(refresh = false) {
    if (!this.client.ready) return;
    this.client
      .request("agent.accounts", { refresh })
      .then((r) => (this.accounts = r.accounts))
      .catch(() => {});
  }

  // ---- stats: sampled by the daemon only while someone subscribes ----
  private statsSubscribed = false;

  /** Subscribe while the window is visible and connected; unsubscribe when hidden. */
  syncStats() {
    const want = this.client.ready && (typeof document === "undefined" || document.visibilityState === "visible");
    if (want && !this.statsSubscribed) {
      this.statsSubscribed = true;
      this.client.request("stats.subscribe", { intervalMs: 5000 }).catch(() => (this.statsSubscribed = false));
    } else if (!want && this.statsSubscribed) {
      this.statsSubscribed = false;
      if (this.client.ready) this.client.request("stats.unsubscribe", {}).catch(() => {});
    }
  }

  linkedTo(id: Id): Id[] {
    const out: Id[] = [];
    for (const k of this.links) {
      const [a, b] = k.split("|");
      if (a === id) out.push(b);
      else if (b === id) out.push(a);
    }
    return out;
  }

  async setLinked(a: Id, b: Id, linked: boolean) {
    const [x, y] = a < b ? [a, b] : [b, a];
    await this.call(linked ? "link.add" : "link.remove", { a: x, b: y });
    const next = new Set(this.links);
    if (linked) next.add(linkKey(x, y));
    else next.delete(linkKey(x, y));
    this.links = next;
  }

  async handoff(id: Id, agentId?: string) {
    const { card } = await this.call("card.handoff", { id, agentId });
    this.applyCard(card);
    const [x, y] = id < card.id ? [id, card.id] : [card.id, id];
    this.links = new Set(this.links).add(linkKey(x, y));
    this.focusTerminal = true;
    await this.openCard(card.id);
  }

  applyCard(card: Card) {
    this.board = cardChanged(this.board, { card });
    const prev = this.allCards[card.id];
    if (!prev || prev.updatedAt <= card.updatedAt) this.allCards = { ...this.allCards, [card.id]: card };
  }

  removeCard(id: Id, projectId: Id) {
    this.board = cardChanged(this.board, { removedId: id, projectId });
    if (this.allCards[id]) {
      const all = { ...this.allCards };
      delete all[id];
      this.allCards = all;
    }
    if (this.openCardId === id) this.openCardId = null;
    if (this.selectedCardId === id) this.selectedCardId = null;
  }

  // ---- columns ----
  applyColumn(column: Column) {
    this.board = columnChanged(this.board, { column });
  }
}

/** What to do next, per method and error code. */
const ADVICE: Record<string, string> = {
  "*:unavailable": "Muster is not connected to its daemon. Wait for it to reconnect, or press Start daemon.",
  "card.delete:conflict": "The card still has a live session. Kill the session first, then delete the card.",
  "project.remove:conflict": "Cards in this project still have live sessions. Kill them first, then remove the project.",
  "project.add:conflict": "That folder is already a project.",
  "project.add:invalid": "Check that the path exists and is a folder.",
  "project.add:not_found": "Check that the path exists and is a folder.",
  "session.attach:not_found": "The session has ended. Press Start to run the card again.",
  "card.start:conflict": "The card is already running.",
  "card.start:internal": "The agent could not be launched. Check that it is installed, and see musterd.log in the Muster data folder.",
  "card.create:invalid": "Check the branch name and the folder.",
  "approval.answer:conflict": "The agent is no longer waiting; it was answered elsewhere.",
  "approval.answer:unavailable": "This answer can only be given in the terminal.",
  "approval.answer:not_found": "The session has ended; there is nothing to answer.",
  "message.cancel:conflict": "Too late: the message was already delivered.",
  "link.add:invalid": "A card cannot be linked to itself.",
  "column.delete:invalid": "Pick another column of this project to move the cards to.",
  "term.write:not_found": "The session has ended. Press Start to run the card again.",
};

export function describeError(m: string, e: unknown): string {
  if (e instanceof ProtocolError) {
    const detail = e.message && e.message !== e.code ? e.message : "";
    const advice = ADVICE[`${m}:${e.code}`] ?? (e.code === "unavailable" ? ADVICE["*:unavailable"] : null);
    if (advice) return detail && e.code !== "unavailable" ? `${advice} (${detail})` : advice;
    const what: Record<string, string> = {
      conflict: "Not possible right now",
      not_found: "That no longer exists; the board may be out of date",
      invalid: "The daemon refused the request",
      unauthorized: "The daemon rejected the token",
      internal: "The daemon hit an error; see musterd.log in the Muster data folder",
    };
    return `${what[e.code] ?? e.code}${detail ? `: ${detail}` : ""}.`;
  }
  return `${m} failed: ${errText(e)}`;
}

export const linkKey = (a: Id, b: Id) => (a < b ? `${a}|${b}` : `${b}|${a}`);

export type { Link };
