<script lang="ts">
  import type { AppState } from "./lib/state/app.svelte";
  import { provideApp } from "./lib/state/context";
  import Sidebar from "./lib/ui/Sidebar.svelte";
  import Board from "./lib/ui/Board.svelte";
  import Tracker from "./lib/ui/Tracker.svelte";
  import AgentPicker from "./lib/ui/AgentPicker.svelte";
  import ProjectSettings from "./lib/ui/ProjectSettings.svelte";
  import SettingsDialog from "./lib/ui/SettingsDialog.svelte";
  import ShortcutSheet from "./lib/ui/ShortcutSheet.svelte";
  import CardMenu from "./lib/ui/CardMenu.svelte";
  import LinkPicker from "./lib/ui/LinkPicker.svelte";
  import CardModal from "./lib/ui/CardModal.svelte";
  import NewCardDialog from "./lib/ui/NewCardDialog.svelte";
  import Icon from "./lib/ui/Icon.svelte";

  let { app }: { app: AppState } = $props();
  // svelte-ignore state_referenced_locally
  provideApp(app);

  const conn = $derived(app.conn);
  const counts = $derived.by(() => {
    let running = 0;
    let needs = 0;
    for (const c of Object.values(app.allCards)) {
      if (c.status === "running") running++;
      else if (c.status === "needs_you") needs++;
    }
    return { running, needs };
  });

  // Narrow windows (phones): the sidebar becomes a drawer.
  let narrow = $state(false);
  let drawer = $state(false);
  $effect(() => {
    const mq = window.matchMedia("(max-width: 720px)");
    const set = () => {
      narrow = mq.matches;
      if (!narrow) drawer = false;
    };
    set();
    mq.addEventListener("change", set);
    return () => mq.removeEventListener("change", set);
  });

  function onKey(e: KeyboardEvent) {
    const typing = (e.target as HTMLElement)?.closest?.("input, textarea, select, [contenteditable], .xterm");
    if (e.key === "?" && !typing && !e.metaKey && !e.ctrlKey) {
      e.preventDefault();
      app.shortcutsOpen = !app.shortcutsOpen;
      return;
    }
    // Cmd+T (Ctrl+T elsewhere) toggles board and tracker.
    if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key === ",") {
      e.preventDefault();
      app.settingsOpen = true;
    }
    if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key === "t") {
      e.preventDefault();
      app.setView(app.view === "board" ? "tracker" : "board");
    }
  }

  let retryIn = $state(0);
  let starting = $state(false);
  $effect(() => {
    if (conn.kind !== "waiting") return;
    const until = Date.now() + conn.retryInMs;
    retryIn = Math.ceil(conn.retryInMs / 1000);
    const t = setInterval(() => (retryIn = Math.max(0, Math.ceil((until - Date.now()) / 1000))), 250);
    return () => clearInterval(t);
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="shell" class:narrow>
  {#if !narrow || drawer}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div
      class="side-wrap"
      class:drawer={narrow}
      onclick={(e) => {
        if (narrow && (e.target as HTMLElement).closest(".item")) drawer = false;
      }}
    >
      <Sidebar />
    </div>
    {#if narrow}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="drawer-scrim" onclick={() => (drawer = false)}></div>
    {/if}
  {/if}
  <main>
    <div class="toolbar" data-tauri-drag-region>
      {#if narrow}
        <button class="icon-btn" title="Projects" onclick={() => (drawer = !drawer)}><Icon name="sidebar" /></button>
      {/if}
      <div class="seg" role="tablist" aria-label="View">
        <button role="tab" aria-selected={app.view === "board"} class:on={app.view === "board"} onclick={() => app.setView("board")} title="Board (⌘T)">Board</button>
        <button role="tab" aria-selected={app.view === "tracker"} class:on={app.view === "tracker"} onclick={() => app.setView("tracker")} title="Tracker (⌘T)">
          Tracker{#if counts.needs}<span class="badge-n">{counts.needs}</span>{/if}
        </button>
      </div>
      {#if app.view === "board" && app.currentProject}
        <h1 data-tauri-drag-region>{app.currentProject.name}</h1>
        <span class="path" data-tauri-drag-region title={app.currentProject.path}>{app.currentProject.path}</span>
      {:else if app.view === "tracker"}
        <h1 data-tauri-drag-region>All projects</h1>
      {/if}
      <span class="spacer" data-tauri-drag-region></span>
      {#if counts.needs && !narrow}<span class="pill needs" title="Cards waiting for you">{counts.needs} need you</span>{/if}
      {#if counts.running && !narrow}<span class="pill running">{counts.running} running</span>{/if}
      <span class="conn k-{conn.kind}" title={conn.kind === "ready" ? `Daemon ${conn.daemonVersion}` : conn.kind}>
        <span class="cdot"></span>
        {#if conn.kind === "ready"}Connected{:else if conn.kind === "waiting"}Reconnecting{:else if conn.kind === "connecting"}Connecting{:else}Offline{/if}
      </span>
      <button
        class="btn primary small"
        disabled={!app.currentProjectId || conn.kind !== "ready" || !app.columns.length}
        class:hide-narrow={app.view === "tracker"}
        onclick={(e) => (app.quick = { anchor: e.currentTarget as HTMLElement, columnId: null })}
        title="New session (N) · more options (⇧N)"><Icon name="plus" size={12} />New</button
      >
    </div>

    {#if conn.kind === "ready"}
      {#each (app.daemonStatus?.notices ?? []).filter((n) => !app.dismissed.has(n.id)) as n (n.id)}
        <div class="notice l-{n.level}" role="status">
          <Icon name="warning" size={14} />
          <span>{n.message}</span>
          <button class="icon-btn" title="Hide until next launch" onclick={() => (app.dismissed = new Set(app.dismissed).add(n.id))}><Icon name="close" size={12} /></button>
        </div>
      {/each}
    {/if}

    {#if conn.kind !== "ready" && !app.board.loaded}
      <div class="center">
        <div class="waiting">
          <div class="spinner"></div>
          <h2>{conn.kind === "waiting" ? "Waiting for the Muster daemon" : "Connecting to the Muster daemon"}</h2>
          {#if conn.kind === "waiting"}
            <p>The daemon runs in the background and keeps your sessions alive while this window is closed.{app.platform.startDaemon ? " Start it here; if it stops again, its log is musterd.log in the Muster data folder." : ""}</p>
            <p class="err">{conn.error}</p>
            <p>Retrying in {retryIn}s</p>
            <div class="row-btns">
              {#if app.platform.startDaemon}
                <button
                  class="btn primary"
                  disabled={starting}
                  onclick={async () => {
                    starting = true;
                    try {
                      await app.platform.startDaemon!();
                      setTimeout(() => app.client.reconnectNow(), 800);
                    } catch (e) {
                      app.toast(`Could not start the daemon: ${e instanceof Error ? e.message : e}`);
                    } finally {
                      setTimeout(() => (starting = false), 1500);
                    }
                  }}>Start daemon</button
                >
              {/if}
              <button class="btn" onclick={() => app.client.reconnectNow()}>Retry now</button>
            </div>
          {/if}
        </div>
      </div>
    {:else if app.view === "tracker"}
      <Tracker />
    {:else if !app.currentProjectId}
      <div class="center">
        <div class="waiting">
          {#if app.projects.length}
            <h2>Pick a project</h2>
            <p>Choose a project in the sidebar to see its board, or open the tracker for everything at once.</p>
          {:else}
            <h2>Add your first project</h2>
            <p>A project is a folder, usually a git repository. Each one gets a board where every card is an agent session.</p>
            <div class="row-btns">
              <button class="btn primary" onclick={() => app.requestAddProject()}><Icon name="folder" size={13} />Add project folder</button>
            </div>
          {/if}
        </div>
      </div>
    {:else}
      <Board />
    {/if}

    {#if conn.kind !== "ready" && app.board.loaded}
      <div class="banner">
        <Icon name="warning" size={14} />
        Connection to the daemon lost{conn.kind === "waiting" ? ` — retrying in ${retryIn}s` : "…"}
        <button class="btn small" onclick={() => app.client.reconnectNow()}>Retry now</button>
      </div>
    {/if}
  </main>
</div>

{#if app.openCardId}
  {#key app.openCardId}
    <CardModal cardId={app.openCardId} />
  {/key}
{/if}

{#if app.quick}
  <AgentPicker />
{/if}

{#if app.cardMenu}
  <CardMenu />
{/if}

{#if app.linking}
  <LinkPicker />
{/if}

{#if app.shortcutsOpen}
  <ShortcutSheet />
{/if}

{#if app.settingsOpen}
  <SettingsDialog />
{/if}

{#if app.projectSettings}
  <ProjectSettings projectId={app.projectSettings} />
{/if}

{#if app.newCard}
  <NewCardDialog columnId={app.newCard.columnId} />
{/if}

<div class="toasts" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div class="toast {t.kind}">{t.text}</div>
  {/each}
</div>

<style>
  .shell {
    display: flex;
    height: 100%;
  }
  .side-wrap {
    display: flex;
  }
  .side-wrap.drawer {
    position: fixed;
    inset: 0 auto 0 0;
    z-index: 35;
    box-shadow: var(--shadow-pop);
  }
  .drawer-scrim {
    position: fixed;
    inset: 0;
    z-index: 34;
    background: var(--scrim);
  }
  .seg {
    display: inline-flex;
    background: var(--bg-hover);
    border-radius: 7px;
    padding: 2px;
    gap: 2px;
    flex: none;
  }
  .seg button {
    border: 0;
    background: transparent;
    height: 22px;
    padding: 0 10px;
    border-radius: 5px;
    font-size: 12px;
    color: var(--text-2);
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .seg button.on {
    background: var(--bg-elev);
    color: var(--text);
    box-shadow: var(--shadow);
  }
  .badge-n {
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 8px;
    background: var(--s-needs);
    color: #1d1d1f;
    font-size: 10.5px;
    font-weight: 700;
    display: inline-grid;
    place-items: center;
  }
  .narrow .path {
    display: none;
  }
  .narrow .hide-narrow {
    display: none;
  }
  main {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .toolbar {
    height: var(--titlebar);
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 14px 0 16px;
  }
  h1 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    white-space: nowrap;
  }
  .path {
    color: var(--text-3);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .spacer {
    flex: 1;
  }
  .pill {
    font-size: 11px;
    font-weight: 500;
    padding: 2px 8px;
    border-radius: 10px;
    white-space: nowrap;
  }
  .pill.needs {
    color: var(--s-needs);
    background: color-mix(in srgb, var(--s-needs) 15%, transparent);
  }
  .pill.running {
    color: var(--s-running);
    background: color-mix(in srgb, var(--s-running) 13%, transparent);
  }
  .conn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-3);
    white-space: nowrap;
  }
  .cdot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--s-exited);
  }
  .k-ready .cdot {
    background: var(--s-done);
  }
  .k-waiting .cdot,
  .k-connecting .cdot {
    background: var(--s-needs);
  }
  .center {
    flex: 1;
    display: grid;
    place-items: center;
  }
  .waiting {
    text-align: center;
    color: var(--text-2);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    max-width: 440px;
  }
  .waiting h2 {
    margin: 8px 0 0;
    font-size: 15px;
    color: var(--text);
  }
  .waiting p {
    margin: 0;
  }
  .waiting .err {
    font: 12px var(--mono);
    color: var(--text-3);
    word-break: break-word;
  }
  .row-btns {
    display: flex;
    gap: 8px;
    margin-top: 8px;
  }
  .spinner {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 14px 6px;
    padding: 6px 6px 6px 10px;
    border-radius: 8px;
    font-size: 12px;
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .notice.l-warning {
    background: color-mix(in srgb, var(--s-needs) 12%, transparent);
    border-color: color-mix(in srgb, var(--s-needs) 35%, transparent);
  }
  .notice > :global(svg) {
    flex: none;
    color: var(--s-needs);
  }
  .notice span {
    flex: 1;
  }
  .banner {
    position: fixed;
    left: 50%;
    bottom: 16px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 6px 12px;
    border-radius: 8px;
    background: var(--bg-elev);
    box-shadow: var(--shadow-pop);
    color: var(--s-needs);
    z-index: 45;
    white-space: nowrap;
  }
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 16px;
    z-index: 300;
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-end;
    pointer-events: none;
  }
  .toast {
    max-width: 420px;
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--bg-elev);
    box-shadow: var(--shadow-pop);
    animation: fade-in 0.15s ease-out;
  }
  .toast.error {
    border-left: 3px solid var(--danger);
  }
  @keyframes fade-in {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
</style>
