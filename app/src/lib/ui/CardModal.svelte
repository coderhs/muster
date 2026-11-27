<script lang="ts">
  import { appMod, computer, keyText } from "../os";
  import { isSleeping } from "./idle";
  import Select from "./Select.svelte";
  import DatePicker from "./DatePicker.svelte";
  import { CATEGORY_COLOR, colorOr } from "./colors";
  import { onMount, tick, untrack } from "svelte";
  import type { Id, Priority } from "../protocol/types";
  import { useApp } from "../state/context";
  import type { TermSession } from "../term/terminal";
  import AgentMark from "./AgentMark.svelte";
  import StatusBadge from "./StatusBadge.svelte";
  import Icon from "./Icon.svelte";
  import Dialog from "./Dialog.svelte";
  import ApprovalBar from "./ApprovalBar.svelte";
  import ChatView from "./ChatView.svelte";
  import Changes from "./Changes.svelte";
  import ContextMeter from "./ContextMeter.svelte";
  import Comments from "./Comments.svelte";
  import { authorLabel } from "./authors";
  import { agoLabel } from "./time";
  import { clock } from "../state/clock.svelte";
  import { caps, reasonLabel } from "../agent/agent";
  import { aiRename, requestAiRename } from "./aiRename";

  let { cardId }: { cardId: Id } = $props();
  const app = useApp();

  const card = $derived(app.card(cardId));
  const agent = $derived(card ? app.agent(card.agentId) : null);
  const column = $derived(card ? app.board.columns[card.columnId] : null);

  let termHost = $state<HTMLDivElement>();
  let term: TermSession | null = null;
  let termShown = $state(false);
  let exitInfo = $state<{ code: number | null | undefined } | null>(null);
  let size = $state<{ cols: number; rows: number } | null>(null);
  let termError = $state<string | null>(null);
  let killing = $state(false);
  let deleting = $state(false);
  let starting = $state(false);
  let msgDraft = $state("");
  const linked = $derived(app.linkedTo(cardId).map((id) => app.card(id)).filter((c) => c != null));

  async function sendMessage() {
    const text = msgDraft.trim();
    if (!text) return;
    try {
      await app.call("message.send", { cardId, text });
      msgDraft = "";
      // queuedMessages arrives with card.changed; fetch now in case the event lags.
      const { card: c } = await app.client.request("card.get", { id: cardId });
      app.applyCard(c);
    } catch {
      /* toast shown */
    }
  }

  async function cancelMessage(id: string) {
    try {
      await app.call("message.cancel", { id });
      const { card: c } = await app.client.request("card.get", { id: cardId });
      app.applyCard(c);
    } catch {
      /* toast shown (conflict: already delivered) */
    }
  }
  // The details column has two tabs; the choice is remembered.
  const SIDE_TAB_KEY = "muster.modal.sideTab";
  let sideTab = $state<"details" | "comments">(
    (() => {
      try {
        return localStorage.getItem(SIDE_TAB_KEY) === "comments" ? "comments" : "details";
      } catch {
        return "details";
      }
    })(),
  );
  function setSideTab(t: "details" | "comments") {
    sideTab = t;
    try {
      localStorage.setItem(SIDE_TAB_KEY, t);
    } catch {
      /* ignore */
    }
  }
  const TAB_KEY = "muster.modal.tab";
  // A quick start opens straight onto a focused terminal.
  // svelte-ignore state_referenced_locally
  const forceTerminal = app.focusTerminal;
  app.focusTerminal = false;
  type Tab = "terminal" | "chat" | "changes";
  let tab = $state<Tab>(
    forceTerminal ? "terminal" :
    (() => {
      try {
        return localStorage.getItem(TAB_KEY) === "chat" ? "chat" : "terminal";
      } catch {
        return "terminal";
      }
    })(),
  );
  let chatMounted = $state(false);
  let changesMounted = $state(false);
  const cap = $derived(caps(agent));
  const rename = $derived(card ? aiRename(card, agent) : { offered: false, blocked: null });
  const hasChat = $derived(!!card && cap.chat);
  const canResume = $derived(!!card?.agentSessionId && cap.resume);
  const showChat = $derived(hasChat && tab === "chat");
  const showChanges = $derived(tab === "changes");
  const tabs = $derived<Tab[]>(hasChat ? ["terminal", "chat", "changes"] : ["terminal", "changes"]);
  $effect(() => {
    if (showChat) chatMounted = true;
    if (showChanges) changesMounted = true;
  });
  function setTab(t: Tab) {
    tab = t;
    // Changes is not remembered: a card opens on the agent, not on its diff.
    try {
      if (t !== "changes") localStorage.setItem(TAB_KEY, t);
    } catch {
      /* ignore */
    }
    if (t === "terminal") void tick().then(() => term?.focus());
  }
  let mountGen = 0;
  let closed = false;

  // ---- terminal lifecycle ----
  function mountTerminal() {
    term?.dispose();
    term = null;
    exitInfo = null;
    termError = null;
    termShown = true;
    const gen = ++mountGen;
    // xterm is loaded on first use so the board itself stays small.
    void Promise.all([import("../term/terminal"), tick()]).then(([{ openTerminal }]) => {
      if (gen !== mountGen || closed || !termHost || !card?.hasSession) return;
      term = openTerminal(termHost, app.client, cardId, {
        onError: (m) => (termError = m),
        onSize: (cols, rows) => (size = { cols, rows }),
        clipboardFiles: app.platform.clipboardFiles,
        // Claude Code binds Ctrl+Enter to "send now".
        extendedCtrlEnter: card.agentId === "claude",
        openUrl: (url) => void app.platform.openUrl(url).catch((e) => app.toast(`Could not open the link: ${e instanceof Error ? e.message : e}`)),
        openFile: (ref) => (app.fileView = { cardId, ...ref }),
      });
      term.focus();
    });
  }

  $effect(() => {
    const has = !!card?.hasSession;
    untrack(() => {
      if (has && (!term || exitInfo)) mountTerminal();
      else if (!has && term && !exitInfo) exitInfo = { code: undefined };
    });
  });

  onMount(() => {
    const opener = document.activeElement as HTMLElement | null;
    // Without a terminal to focus, put focus on the main action so Enter starts the card.
    setTimeout(() => {
      if (!card?.hasSession) document.querySelector<HTMLElement>(".modal .noterm .btn.primary")?.focus();
    }, 0);
    const restoreFocus = () => {
      // Back to the card the modal was opened from (board card or tracker row).
      const sel = `[data-card-id="${CSS.escape(cardId)}"], [data-row-id="${CSS.escape(cardId)}"]`;
      setTimeout(() => {
        const el = document.querySelector<HTMLElement>(sel) ?? (opener?.isConnected ? opener : null);
        el?.focus({ preventScroll: false });
      }, 0);
    };
    const off = app.client.on("session.exit", (p) => {
      if (p.cardId === cardId) exitInfo = { code: p.exitCode };
    });
    return () => {
      closed = true;
      restoreFocus();
      off();
      term?.dispose();
      term = null;
    };
  });

  // Opening a card reads it; keep it read while it stays open.
  $effect(() => {
    if (card && (card.unread || card.status === "done")) untrack(() => void app.markRead(cardId).catch(() => {}));
  });

  // ---- fields ----
  let titleDraft = $state("");
  let notesDraft = $state("");
  let notesTimer: ReturnType<typeof setTimeout> | null = null;
  let lastNotes = "";
  let focused = $state<"title" | "notes" | null>(null);

  $effect(() => {
    const c = card;
    if (!c) return;
    untrack(() => {
      if (focused !== "title") titleDraft = c.title;
      if (focused !== "notes" && (c.notes ?? "") !== lastNotes) {
        notesDraft = c.notes ?? "";
        lastNotes = notesDraft;
      }
    });
  });

  async function saveTitle() {
    focused = null;
    const t = titleDraft.trim();
    if (!card || !t || t === card.title) {
      if (card) titleDraft = card.title;
      return;
    }
    await app.updateCard({ id: cardId, title: t }).catch(() => {});
  }

  function notesInput() {
    if (notesTimer) clearTimeout(notesTimer);
    notesTimer = setTimeout(saveNotes, 800);
  }

  async function saveNotes() {
    if (notesTimer) clearTimeout(notesTimer);
    notesTimer = null;
    if (notesDraft === lastNotes) return;
    lastNotes = notesDraft;
    await app.updateCard({ id: cardId, notes: notesDraft }).catch(() => {});
  }

  async function setPriority(p: Priority | null) {
    await app.updateCard({ id: cardId, priority: p }).catch(() => {});
  }

  async function setDue(v: number | null) {
    await app.updateCard({ id: cardId, dueAt: v }).catch(() => {});
  }

  async function moveTo(colId: Id) {
    if (!card || colId === card.columnId) return;
    await app.moveCard(cardId, colId); // no neighbours: append to the column
  }

  async function start() {
    starting = true;
    // A failed worktree setup is retried in a replaced session: attach to the new one.
    const replacing = card?.needsYouReason === "setup_failed";
    try {
      await app.startCard(cardId);
      if (replacing) mountTerminal();
    } catch {
      /* toast shown */
    } finally {
      starting = false;
    }
  }

  function kill() {
    killing = false;
    void app.call("session.kill", { cardId }).catch(() => {});
  }

  function close() {
    if (focused === "notes") void saveNotes();
    app.openCardId = null;
  }

  function onKey(e: KeyboardEvent) {
    if (deleting || killing || app.searchOpen || app.fileView) return;
    const inTerm = !!(e.target as HTMLElement)?.closest?.(".xterm");
    if (e.key === "w" && appMod(e) && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      close();
    } else if (e.key === "Escape" && !inTerm && !e.defaultPrevented) {
      e.preventDefault();
      if ((e.target as HTMLElement)?.closest?.("input, textarea, select")) (e.target as HTMLElement).blur();
      else close();
    } else if (e.key === "Tab" && e.ctrlKey) {
      // Ctrl+Tab steps through Terminal, Chat and Changes; with Shift, backwards.
      e.preventDefault();
      const i = tabs.indexOf(tab);
      setTab(tabs[(i + (e.shiftKey ? tabs.length - 1 : 1)) % tabs.length]);
    } else if (e.key === "i" && appMod(e) && !e.shiftKey && !e.altKey) {
      // Cmd+I hides or shows the details column.
      e.preventDefault();
      app.toggleCardSide();
    } else if (e.key === "`" && e.ctrlKey) {
      // Ctrl+` toggles focus between terminal and the side panel.
      e.preventDefault();
      if (inTerm) {
        if (app.cardSideCollapsed) app.toggleCardSide();
        void tick().then(() =>
          (document.querySelector(".side .sidepane:not(.hidden) :is(input, textarea)") as HTMLElement | null)?.focus(),
        );
      }
      else term?.focus();
    }
  }

  const PRIORITIES: Array<{ v: Priority | null; label: string }> = [
    { v: null, label: "None" },
    { v: "low", label: "Low" },
    { v: "medium", label: "Med" },
    { v: "high", label: "High" },
    { v: "urgent", label: "Urgent" },
  ];
</script>

<svelte:window onkeydown={onKey} />

{#if card && agent}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="scrim"
    onpointerdown={(e) => {
      if (e.target === e.currentTarget) close();
    }}
  >
    <div class="modal" role="dialog" aria-modal="true" aria-label={card.title}>
      <header class="bar">
        <AgentMark {agent} size={22} />
        <input
          class="title"
          bind:value={titleDraft}
          onfocus={() => (focused = "title")}
          onblur={saveTitle}
          onkeydown={(e) => {
            if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur();
            if (e.key === "Escape") {
              e.preventDefault();
              titleDraft = card.title;
              (e.currentTarget as HTMLInputElement).blur();
            }
          }}
          aria-label="Title"
        />
        {#if card.titleAuto}
          <span class="auto" title="Muster names this card from the session. Type a title to set your own.">auto</span>
        {:else if card.titleAuto === false}
          <button class="btn small ghost" title="Let Muster name this card from the session again" onclick={() => app.updateCard({ id: cardId, titleAuto: true }).catch(() => {})}>
            Auto title
          </button>
        {/if}
        {#if rename.offered}
          <button
            class="btn small ghost"
            disabled={!!rename.blocked}
            title={rename.blocked ?? "Ask the agent to name this session from the conversation so far"}
            onclick={() => requestAiRename(app, cardId)}
          >
            Rename with AI
          </button>
        {/if}
        {#if card.status === "needs_you" && card.needsYouReason}
          <span class="reason">{reasonLabel(card.needsYouReason, card.setup?.exitCode)}</span>
        {/if}
        <StatusBadge status={card.status} since={card.statusSince} settingUp={card.setup?.state === "running"} sleeping={isSleeping(card)} source={cap.statusSource} />
        <button
          class="icon-btn"
          class:hidden-side={app.cardSideCollapsed}
          title={keyText(app.cardSideCollapsed ? "Show details (⌘I)" : "Hide details (⌘I)")}
          aria-pressed={!app.cardSideCollapsed}
          onclick={() => app.toggleCardSide()}><Icon name="sidebarRight" stroke={app.cardSideCollapsed ? 2.4 : 1.5} /></button
        >
        <button class="icon-btn" title={keyText("Close (Esc, ⌘W)")} onclick={close}><Icon name="close" /></button>
      </header>

      <div class="main">
        <div class="pane">
        <div class="tabs" role="tablist">
          <button role="tab" aria-selected={tab === "terminal"} class:on={tab === "terminal"} onclick={() => setTab("terminal")}>
            <Icon name="terminal" size={13} />Terminal
          </button>
          {#if hasChat}
            <button role="tab" aria-selected={tab === "chat"} class:on={tab === "chat"} onclick={() => setTab("chat")}>
              Chat
            </button>
          {/if}
          <button role="tab" aria-selected={tab === "changes"} class:on={tab === "changes"} onclick={() => setTab("changes")}>
            <Icon name="branch" size={13} />Changes
          </button>
          <span class="tabkeys"><kbd>⌃</kbd><kbd>⇥</kbd> switch</span>
        </div>
        {#if card.needsYouReason === "setup_failed"}
          <div class="appr-wrap">
            <div class="setup-failed">
              <Icon name="warning" size={14} />
              <span>
                The worktree setup script failed{card.setup?.exitCode != null ? ` (exit ${card.setup.exitCode})` : ""}; the agent was not started.
                The terminal is a shell in the worktree. Fix the script or the repository, then run it again.
              </span>
              <button class="btn small" onclick={() => (app.projectSettings = card.projectId)}>Edit script</button>
              <button class="btn small primary" disabled={starting} onclick={start}>Run setup again</button>
            </div>
          </div>
        {/if}
        {#if card.pendingApproval}
          <div class="appr-wrap"><ApprovalBar {card} /></div>
        {/if}
        {#if chatMounted}
          <div class="chat-wrap" class:hidden={!showChat}><ChatView {card} active={showChat} /></div>
        {/if}
        {#if changesMounted}
          <div class="chat-wrap" class:hidden={!showChanges}><Changes {card} active={showChanges} /></div>
        {/if}
        <div class="term-wrap" class:hidden={showChat || showChanges}>
          <div class="term" class:hidden={!termShown} bind:this={termHost}></div>
          {#if !termShown}
            <div class="noterm">
              {#if card.restorable}
                <p>This session was interrupted (the {computer()} restarted or tmux stopped). Restoring replays its scrollback and resumes the agent's conversation.</p>
              {:else if card.status === "draft"}
                <p>This card is a draft. Starting it launches <b>{agent.label}</b>{card.prompt ? " with its prompt" : ""}.</p>
              {:else if isSleeping(card)}
                <p>Muster stopped this session after it sat idle. Resuming continues the conversation where it left off.</p>
              {:else}
                <p>No session is running for this card.</p>
              {/if}
              <button class="btn primary" disabled={starting} onclick={start}>
                <Icon name={card.restorable ? "restore" : "play"} size={11} />{card.restorable ? "Restore session" : canResume ? "Resume" : "Start"}
              </button>
              {#if card.prompt}<pre class="prompt">{card.prompt}</pre>{/if}
            </div>
          {/if}
          {#if exitInfo}
            <div class="exit">
              <Icon name="stop" size={12} />
              {#if isSleeping(card)}Stopped after sitting idle.{:else}Session ended{exitInfo.code != null ? ` (exit ${exitInfo.code})` : ""}.{/if}
              <button class="btn small" disabled={starting} onclick={start}>{canResume ? "Resume" : "Start again"}</button>
            </div>
          {/if}
          {#if termError}<div class="exit err"><Icon name="warning" size={12} />{termError}</div>{/if}
        </div>
        </div>

        <aside class="side" class:collapsed={app.cardSideCollapsed}>
          <div class="sidetabs" role="tablist" aria-label="Card panel">
            <button role="tab" aria-selected={sideTab === "details"} class:on={sideTab === "details"} onclick={() => setSideTab("details")}>Details</button>
            <button role="tab" aria-selected={sideTab === "comments"} class:on={sideTab === "comments"} onclick={() => setSideTab("comments")}>
              Comments{#if card.commentCount}<span class="count">{card.commentCount}</span>{/if}
            </button>
          </div>
          <div class="sidepane" class:hidden={sideTab !== "comments"}>
            <Comments {cardId} active={sideTab === "comments" && !app.cardSideCollapsed} />
          </div>
          <div class="sidepane details" class:hidden={sideTab !== "details"}>
          <section>
            <div class="label">Column</div>
            <Select
              label="Column"
              value={card.columnId}
              options={app.columns.map((c) => ({ value: c.id, label: c.title, color: colorOr(c.color, CATEGORY_COLOR[c.category]) }))}
              onchange={(v) => moveTo(v)}
            />
          </section>

          <section>
            <div class="label">Priority</div>
            <div class="seg" role="radiogroup" aria-label="Priority">
              {#each PRIORITIES as p (p.label)}
                <button
                  role="radio"
                  aria-checked={(card.priority ?? null) === p.v}
                  class:on={(card.priority ?? null) === p.v}
                  class="p-{p.v ?? 'none'}"
                  onclick={() => setPriority(p.v)}>{p.label}</button
                >
              {/each}
            </div>
          </section>

          <section>
            <div class="label">Due</div>
            <DatePicker label="Due date" value={card.dueAt ?? null} onchange={(v) => setDue(v)} />
          </section>

          <section class="grow">
            <div class="label">Notes</div>
            <textarea
              class="field notes"
              placeholder="Notes: yours and the agent's"
              bind:value={notesDraft}
              oninput={notesInput}
              onfocus={() => (focused = "notes")}
              onblur={() => {
                focused = null;
                void saveNotes();
              }}
            ></textarea>
            {#if card.notesEditedAt}
              <div class="muted edited" title={new Date(card.notesEditedAt).toLocaleString()}>
                Edited by {authorLabel(card.notesEditedBy, (id) => app.agent(id), cardId)} · {agoLabel(card.notesEditedAt, clock.now)}
              </div>
            {/if}
          </section>

          <section>
            <div class="label row-label">
              Links
              <button class="icon-btn" title="Link to another card" onclick={(e) => (app.linking = { cardId, anchor: e.currentTarget as HTMLElement })}>
                <Icon name="plus" size={13} />
              </button>
            </div>
            {#each linked as l (l.id)}
              <div class="linkrow">
                <button class="linkname" title="Open {l.title}" onclick={() => app.openCard(l.id)}>
                  <AgentMark agent={app.agent(l.agentId)} size={14} /><span>{l.title}</span>
                </button>
                <button class="icon-btn" title="Remove link" onclick={() => app.setLinked(cardId, l.id, false).catch(() => {})}><Icon name="close" size={12} /></button>
              </div>
            {:else}
              <div class="muted">Linked cards can read each other's work.</div>
            {/each}
          </section>

          <section>
            <div class="label">Messages</div>
            {#each card.queuedMessages ?? [] as m (m.id)}
              <div class="msgrow">
                <Icon name="mail" size={12} />
                <span class="mt" title={m.text}>{m.text}</span>
                <span class="mfrom">{m.fromCardId ? `from ${app.card(m.fromCardId)?.title ?? "a card"}` : "from you"}</span>
                <button class="icon-btn" title="Cancel this message" onclick={() => cancelMessage(m.id)}><Icon name="close" size={12} /></button>
              </div>
            {/each}
            <form
              class="msgform"
              onsubmit={(e) => {
                e.preventDefault();
                void sendMessage();
              }}
            >
              <input class="field" placeholder="Queue a message for when it is idle" bind:value={msgDraft} />
            </form>
          </section>

          <section class="info">
            <div><span>Agent</span>{agent.label}</div>
            {#if card.model}<div><span>Model</span>{card.model}</div>{/if}
            {#if cap.resume && card.agentId !== "shell" && (app.settings?.idleStopMinutes || card.keepRunning)}
              <div>
                <span>When idle</span>
                <label class="keep" title="Muster stops sessions that sit idle (Settings → Daemon). Tick to exempt this card.">
                  <input
                    type="checkbox"
                    checked={!!card.keepRunning}
                    onchange={(e) => app.updateCard({ id: cardId, keepRunning: (e.currentTarget as HTMLInputElement).checked }).catch(() => {})}
                  />
                  Keep running
                </label>
              </div>
              {#if card.idleKept && !card.keepRunning}
                <div><span></span><small class="kept">Idle, but left running: {card.idleKept}.</small></div>
              {/if}
            {/if}
            {#if column}<div><span>Column</span>{column.title}</div>{/if}
            {#if card.branch}<div><span>Branch</span><code>{card.branch}</code></div>{/if}
            <div><span>Folder</span><code title={card.cwd}>{card.cwd}</code></div>
            {#if size && termShown}<div><span>Size</span>{size.cols}×{size.rows}</div>{/if}
            {#if card.context && cap.context}<div><span>Context</span><ContextMeter context={card.context} wide /></div>{/if}
          </section>

          <section class="actions">
            {#if card.hasSession}
              <button class="btn danger" onclick={() => (killing = true)}><Icon name="stop" size={11} />Kill session</button>
            {:else}
              <button class="btn" disabled={starting} onclick={start}><Icon name={card.restorable ? "restore" : "play"} size={11} />{card.restorable ? "Restore" : canResume ? "Resume" : "Start"}</button>
            {/if}
            {#if card.agentId !== "shell"}
              <button class="btn" title="Continue this work in a new linked card that reads this one first" onclick={() => app.handoff(cardId, card.agentId).catch(() => {})}>
                <Icon name="handoff" size={12} />Hand off
              </button>
            {/if}
            <button class="btn danger" onclick={() => (deleting = true)}><Icon name="trash" size={12} />Delete</button>
          </section>
          </div>
        </aside>
      </div>
    </div>
  </div>

  {#if killing}
    <Dialog label="Kill session" onclose={() => (killing = false)} width={400}>
      <div class="confirm">
        <h3>Kill the session of “{card.title}”?</h3>
        <p>
          The {agent.label} process and everything running in its terminal stop now; unsaved work in it is lost.
          {card.agentSessionId ? "The conversation can be resumed later with Start." : ""}
        </p>
        <div class="cactions">
          <button class="btn" onclick={() => (killing = false)} data-autofocus>Cancel</button>
          <button class="btn primary" style="background:var(--danger)" onclick={kill}>Kill session</button>
        </div>
      </div>
    </Dialog>
  {/if}

  {#if deleting}
    <Dialog label="Delete card" onclose={() => (deleting = false)} width={380}>
      <div class="confirm">
        <h3>Delete “{card.title}”?</h3>
        <p>{card.hasSession ? "Its running session is killed. " : ""}This cannot be undone.</p>
        <div class="cactions">
          <button class="btn" onclick={() => (deleting = false)} data-autofocus>Cancel</button>
          <button
            class="btn primary"
            style="background:var(--danger)"
            onclick={() => {
              deleting = false;
              void app.deleteCard(cardId).catch(() => {});
            }}>Delete</button
          >
        </div>
      </div>
    </Dialog>
  {/if}
{/if}

<style>
  .keep {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .kept {
    color: var(--text-2);
  }
  /* Heavier and brighter while the details column is hidden. */
  .icon-btn.hidden-side {
    color: var(--text);
  }
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: var(--scrim);
    display: grid;
    place-items: center;
    padding: 28px 24px 24px;
    animation: fade 0.1s ease-out;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .modal {
    width: min(1280px, 100%);
    height: min(860px, 100%);
    background: var(--bg-elev);
    border-radius: 12px;
    box-shadow: var(--shadow-pop);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px 8px 14px;
    border-bottom: 1px solid var(--border);
  }
  .title {
    flex: 1;
    min-width: 0;
    border: 1px solid transparent;
    background: transparent;
    font-size: 15px;
    font-weight: 600;
    padding: 3px 6px;
    border-radius: 6px;
    outline: none;
  }
  .title:hover {
    background: var(--bg-hover);
  }
  .title:focus {
    border-color: var(--accent);
    background: var(--bg-input);
  }
  .main {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .pane {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-term);
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 10px 0;
    border-bottom: 1px solid var(--border);
    background: var(--bg-elev);
  }
  .tabs button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 0;
    background: transparent;
    padding: 5px 12px 7px;
    font-size: 12px;
    color: var(--text-2);
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .tabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .tabkeys {
    margin-left: auto;
    font-size: 11px;
    color: var(--text-3);
  }
  .appr-wrap {
    padding: 8px 10px 0;
    /* Many questions must scroll here, not push the answer buttons past the modal's clipped edge. */
    flex: 0 1 auto;
    min-height: 0;
    max-height: 60%;
    overflow-y: auto;
  }
  .chat-wrap {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .chat-wrap.hidden,
  .term-wrap.hidden {
    display: none;
  }
  .auto {
    font-size: 10.5px;
    color: var(--text-3);
    border: 1px solid var(--border-strong);
    border-radius: 4px;
    padding: 0 4px;
    white-space: nowrap;
  }
  .ghost {
    box-shadow: none;
    background: transparent;
    color: var(--text-2);
    white-space: nowrap;
  }
  .setup-failed {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: 8px;
    font-size: 12px;
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
  }
  .setup-failed > :global(svg) {
    color: var(--danger);
    flex: none;
  }
  .setup-failed span {
    flex: 1;
  }
  .reason {
    font-size: 12px;
    color: var(--s-needs);
    white-space: nowrap;
  }
  .term-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
    background: var(--bg-term);
    display: flex;
  }
  /* No padding here: the terminal is fitted to this box's size, and padding on it
     would be counted as room for text, pushing the last row and columns out of view. */
  .term {
    flex: 1;
    min-width: 0;
    min-height: 0;
    margin: 8px 4px 4px 10px;
    overflow: hidden;
  }
  .term.hidden {
    display: none;
  }
  .term :global(.xterm) {
    height: 100%;
  }
  .term :global(.xterm-viewport) {
    background: transparent !important;
  }
  .noterm {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 24px;
    color: var(--text-2);
    text-align: center;
  }
  .noterm p {
    margin: 0;
  }
  .prompt {
    max-width: 560px;
    max-height: 40%;
    overflow: auto;
    text-align: left;
    white-space: pre-wrap;
    font: 12px/1.5 var(--mono);
    background: var(--bg-hover);
    padding: 10px 12px;
    border-radius: 8px;
    margin: 8px 0 0;
    -webkit-user-select: text;
    user-select: text;
  }
  .exit {
    position: absolute;
    left: 50%;
    bottom: 14px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 6px 12px;
    border-radius: 8px;
    background: var(--bg-elev);
    box-shadow: var(--shadow-pop);
    white-space: nowrap;
  }
  .exit.err {
    bottom: 56px;
    color: var(--danger);
    padding-right: 12px;
  }
  .side {
    width: 272px;
    flex: none;
    border-left: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .sidetabs {
    flex: none;
    display: flex;
    gap: 2px;
    padding: 8px 10px 0;
    border-bottom: 1px solid var(--border);
  }
  .sidetabs button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 10px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-2);
    font-size: 12px;
    font-weight: 500;
  }
  .sidetabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent, #0a84ff);
  }
  .sidetabs .count {
    min-width: 16px;
    height: 15px;
    padding: 0 4px;
    border-radius: 8px;
    background: var(--bg-active);
    font-size: 10.5px;
    font-weight: 600;
    display: inline-grid;
    place-items: center;
  }
  .sidepane {
    flex: 1;
    min-height: 0;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .sidepane.details {
    overflow-y: auto;
  }
  .sidepane.hidden {
    display: none;
  }
  .edited {
    font-size: 11px;
  }
  /* Hidden, not unmounted, so unsaved notes and drafts survive a toggle. */
  .side.collapsed {
    display: none;
  }
  .side section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .side .grow {
    flex: 1;
    min-height: 120px;
  }
  .notes {
    flex: 1;
    resize: none;
  }
  .row {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .seg {
    display: flex;
    border: 1px solid var(--border-strong);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg button {
    flex: 1;
    border: 0;
    background: transparent;
    height: 24px;
    font-size: 11.5px;
    border-left: 1px solid var(--border);
    padding: 0;
  }
  .seg button:first-child {
    border-left: 0;
  }
  .seg button:hover {
    background: var(--bg-hover);
  }
  .seg button.on {
    background: var(--accent);
    color: var(--accent-text);
  }
  .seg .p-urgent.on {
    background: var(--p-urgent);
  }
  .seg .p-high.on {
    background: var(--p-high);
    color: #1d1d1f;
  }
  .row-label {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .muted {
    font-size: 12px;
    color: var(--text-3);
  }
  .linkrow,
  .msgrow {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: 12px;
  }
  .linkname {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    border: 0;
    background: transparent;
    padding: 2px 0;
    text-align: left;
  }
  .linkname span,
  .mt {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .linkname:hover span {
    color: var(--accent);
  }
  .mt {
    flex: 1;
  }
  .mfrom {
    color: var(--text-3);
    font-size: 11px;
    white-space: nowrap;
  }
  .msgrow > :global(svg) {
    color: var(--accent);
    flex: none;
  }
  .info {
    font-size: 12px;
    gap: 4px !important;
  }
  .info div {
    display: flex;
    gap: 8px;
    min-width: 0;
  }
  .info span {
    width: 52px;
    flex: none;
    color: var(--text-3);
  }
  .info code {
    font: 11px var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    -webkit-user-select: text;
    user-select: text;
  }
  .actions {
    flex-direction: row !important;
    flex-wrap: wrap;
  }
  .confirm {
    padding: 18px 20px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: 14px;
  }
  .confirm p {
    margin: 0;
    color: var(--text-2);
  }
  .cactions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
  @media (max-width: 720px) {
    .scrim {
      padding: 0;
    }
    .modal {
      border-radius: 0;
      height: 100%;
    }
    .main {
      flex-direction: column;
    }
    .side {
      width: auto;
      max-height: 45%;
      border-left: 0;
      border-top: 1px solid var(--border);
    }
  }
</style>
