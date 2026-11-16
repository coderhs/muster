<script lang="ts">
  import { onMount, tick } from "svelte";
  import type { CardStatus, Id } from "../protocol/types";
  import { useApp } from "../state/context";
  import { searchCards } from "./search";
  import { STATUS_LABEL } from "./StatusBadge.svelte";
  import AgentMark from "./AgentMark.svelte";
  import Icon from "./Icon.svelte";
  import { colorOr } from "./colors";

  const app = useApp();

  let query = $state("");
  let active = $state<Id | null>(null);
  let input: HTMLInputElement;
  let list: HTMLDivElement;
  let restore: HTMLElement | null = null;

  const groups = $derived(searchCards(Object.values(app.allCards), app.projects, query, app.currentProjectId));
  const flat = $derived(groups.flatMap((g) => g.hits.map((h) => h.card.id)));

  // Keep a valid row selected as the results change.
  $effect(() => {
    if (!active || !flat.includes(active)) active = flat[0] ?? null;
  });

  onMount(() => {
    restore = document.activeElement as HTMLElement | null;
    input.focus();
    return () => restore?.focus?.();
  });

  function close() {
    app.searchOpen = false;
  }

  function open(id: Id) {
    // The card window takes the focus; do not hand it back to what had it before.
    restore = null;
    close();
    void app.openCard(id);
  }

  async function move(by: number) {
    if (!flat.length) return;
    const i = active ? flat.indexOf(active) : -1;
    active = flat[Math.min(flat.length - 1, Math.max(0, i + by))];
    await tick();
    list?.querySelector(`[data-id="${CSS.escape(active)}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) {
      e.preventDefault();
      void move(1);
    } else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) {
      e.preventDefault();
      void move(-1);
    } else if (e.key === "Enter" && active) {
      e.preventDefault();
      open(active);
    } else if (e.key === "Tab") {
      e.preventDefault();
    }
  }

  const STATUS_VAR: Record<CardStatus, string> = {
    running: "--s-running",
    needs_you: "--s-needs",
    done: "--s-done",
    idle: "--s-idle",
    exited: "--s-exited",
    draft: "--s-draft",
  };
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={close}></div>
<div class="search" role="dialog" aria-modal="true" aria-label="Search cards" tabindex="-1" onkeydown={onKey}>
  <div class="field">
    <Icon name="search" size={15} />
    <input
      bind:this={input}
      bind:value={query}
      placeholder="Search cards in all projects"
      spellcheck="false"
      autocomplete="off"
      role="combobox"
      aria-expanded="true"
      aria-controls="search-results"
      aria-activedescendant={active ? `sr-${active}` : undefined}
    />
    <kbd>esc</kbd>
  </div>
  <div class="results" id="search-results" role="listbox" bind:this={list}>
    {#each groups as g (g.project.id)}
      <div class="group" role="group" aria-label={g.project.name}>
        <div class="ghead">
          <span class="pdot" style="background:{colorOr(g.project.color, '#8e8e93')}"></span>
          {g.project.name}
          {#if g.project.id === app.currentProjectId}<span class="here">current</span>{/if}
          {#if !query.trim()}<span class="recent">recent</span>{/if}
        </div>
        {#each g.hits as h (h.card.id)}
          {@const c = h.card}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="row"
            id="sr-{c.id}"
            data-id={c.id}
            role="option"
            tabindex="-1"
            aria-selected={active === c.id}
            class:on={active === c.id}
            onmousemove={() => (active = c.id)}
            onclick={() => open(c.id)}
          >
            <AgentMark agent={app.agent(c.agentId)} size={18} />
            <div class="text">
              <div class="title">{c.title || "Untitled"}</div>
              {#if h.excerpt}<div class="excerpt">{h.excerpt}</div>{/if}
            </div>
            {#if c.projectId === app.currentProjectId && app.board.columns[c.columnId]}
              <span class="col">{app.board.columns[c.columnId].title}</span>
            {/if}
            <span class="status" style="--c:var({STATUS_VAR[c.status]})"><span class="sdot"></span>{STATUS_LABEL[c.status]}</span>
          </div>
        {/each}
      </div>
    {:else}
      <div class="empty">{query.trim() ? "No cards match." : "No cards yet."}</div>
    {/each}
  </div>
  <div class="foot">
    <span><kbd>↑</kbd><kbd>↓</kbd> select</span>
    <span><kbd>↵</kbd> open</span>
    <span><kbd>esc</kbd> close</span>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 200;
    background: var(--scrim);
  }
  .search {
    position: fixed;
    top: 12vh;
    left: 50%;
    transform: translateX(-50%);
    width: min(640px, calc(100vw - 32px));
    max-height: 70vh;
    z-index: 201;
    display: flex;
    flex-direction: column;
    background: var(--bg-elev);
    border-radius: 12px;
    box-shadow: var(--shadow-pop);
    overflow: hidden;
    outline: none;
  }
  .field {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 14px;
    height: 48px;
    flex: none;
    border-bottom: 1px solid var(--border);
    color: var(--text-3);
  }
  .field input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: 16px;
  }
  .results {
    overflow-y: auto;
    padding: 4px 6px 6px;
  }
  .ghead {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 10px 8px 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-2);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .pdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .here,
  .recent {
    font-weight: 500;
    text-transform: none;
    letter-spacing: 0;
    color: var(--text-3);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px;
    border-radius: 7px;
    cursor: default;
  }
  .row.on {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .title,
  .excerpt {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title {
    font-size: 13px;
  }
  .excerpt {
    font-size: 11.5px;
    color: var(--text-3);
    margin-top: 1px;
  }
  .col {
    flex: none;
    font-size: 11px;
    color: var(--text-3);
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-2);
    width: 78px;
  }
  .sdot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--c);
  }
  .empty {
    padding: 24px;
    text-align: center;
    color: var(--text-3);
  }
  .foot {
    flex: none;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 6px 12px;
    border-top: 1px solid var(--border);
    font-size: 11px;
    color: var(--text-3);
  }
  .foot span {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  kbd {
    font: 10.5px var(--mono);
    padding: 1px 5px;
    border-radius: 4px;
    border: 1px solid var(--border-strong);
    color: var(--text-3);
  }
</style>
