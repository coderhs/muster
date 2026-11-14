<script lang="ts">
  import { tick } from "svelte";
  import type { Card, Id } from "../protocol/types";
  import { useApp } from "../state/context";
  import { clock } from "../state/clock.svelte";
  import { groupCards, timerLabel, type GroupKey } from "../tracker/tracker";
  import AgentMark from "./AgentMark.svelte";
  import Icon from "./Icon.svelte";
  import { colorOr } from "./colors";
  import ApprovalBar from "./ApprovalBar.svelte";
  import ContextMeter from "./ContextMeter.svelte";
  import AgentUsage from "./AgentUsage.svelte";
  import StatsChip from "./StatsChip.svelte";
  import { bytes, cpu } from "./format";
  import { reasonLabel } from "../agent/agent";

  const app = useApp();

  const groups = $derived(groupCards(Object.values(app.allCards)));
  const projectsById = $derived(new Map(app.projects.map((p) => [p.id, p])));

  const OPEN_KEY = "muster.tracker.open";
  let opened = $state<Partial<Record<GroupKey, boolean>>>(load());
  function load() {
    try {
      return JSON.parse(localStorage.getItem(OPEN_KEY) ?? "{}");
    } catch {
      return {};
    }
  }
  function toggle(k: GroupKey, dflt: boolean) {
    opened = { ...opened, [k]: !(opened[k] ?? !dflt) };
    try {
      localStorage.setItem(OPEN_KEY, JSON.stringify(opened));
    } catch {
      /* ignore */
    }
  }
  const isOpen = (k: GroupKey, collapsedByDefault: boolean) => opened[k] ?? !collapsedByDefault;

  /** Rows in display order, for keyboard navigation. */
  const visible = $derived(groups.flatMap((g) => (isOpen(g.key, g.collapsedByDefault) ? g.cards : [])));

  let listEl: HTMLDivElement;

  async function select(id: Id | null) {
    app.selectedCardId = id;
    await tick();
    if (!id) return;
    const el = listEl?.querySelector<HTMLElement>(`[data-row-id="${CSS.escape(id)}"]`);
    el?.scrollIntoView({ block: "nearest" });
    el?.focus({ preventScroll: true });
  }

  function onKey(e: KeyboardEvent) {
    if (e.defaultPrevented || app.openCardId || app.newCard || app.shortcutsOpen || app.settingsOpen || app.projectSettings) return;
    const t = e.target as HTMLElement;
    if (t.closest("input, textarea, select, [contenteditable], [role=dialog], [role=menu]")) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const i = visible.findIndex((c) => c.id === app.selectedCardId);
    if (e.key === "ArrowDown" || e.key === "j") {
      e.preventDefault();
      void select(visible[Math.min(visible.length - 1, i + 1)]?.id ?? null);
    } else if (e.key === "ArrowUp" || e.key === "k") {
      e.preventDefault();
      void select(visible[Math.max(0, i - 1)]?.id ?? visible[0]?.id ?? null);
    } else if (e.key === "Enter" && i >= 0) {
      e.preventDefault();
      void app.openCard(visible[i].id);
    }
  }

  const total = $derived(Object.keys(app.allCards).length);
  const count = (k: GroupKey) => groups.find((g) => g.key === k)?.cards.length ?? 0;
</script>

<svelte:window onkeydown={onKey} />

<div class="tracker" bind:this={listEl}>
  <AgentUsage />
  <div class="summary">
    <div class="stat needs" class:zero={!count("needs_you")}><b>{count("needs_you")}</b> need you</div>
    <div class="stat fin" class:zero={!count("finished")}><b>{count("finished")}</b> finished</div>
    <div class="stat run" class:zero={!count("running")}><b>{count("running")}</b> running</div>
    <div class="stat idle" class:zero={!count("idle")}><b>{count("idle")}</b> idle</div>
  </div>

  {#if app.statsTotal}
    {@const t = app.statsTotal}
    <div class="res" title="All card sessions together; the daemon itself separately">
      <Icon name="cpu" size={13} />
      Sessions use <b>{bytes(t.memBytes)}</b> and <b>{cpu(t.cpuPercent)}</b> CPU in {t.processes} process{t.processes === 1 ? "" : "es"}
      {#if app.statsDaemon}<span class="d">· daemon {bytes(app.statsDaemon.memBytes)}</span>{/if}
    </div>
  {/if}

  {#if !total}
    <div class="empty">No cards yet. Create one from a project board.</div>
  {/if}

  {#each groups as g (g.key)}
    {#if g.cards.length}
      {@const open = isOpen(g.key, g.collapsedByDefault)}
      <section class="group g-{g.key}">
        <button class="ghead" onclick={() => toggle(g.key, g.collapsedByDefault)} aria-expanded={open}>
          <span class="chev" class:open><Icon name="chevronRight" size={12} /></span>
          <span class="gdot"></span>
          {g.label}
          <span class="gcount">{g.cards.length}</span>
        </button>
        {#if open}
          <div class="rows" role="listbox" aria-label={g.label}>
            {#each g.cards as c (c.id)}
              {@render row(c)}
            {/each}
          </div>
        {/if}
      </section>
    {/if}
  {/each}
</div>

{#snippet row(c: Card)}
  {@const p = projectsById.get(c.projectId)}
  <div
    class="row"
    class:selected={app.selectedCardId === c.id}
    data-row-id={c.id}
    role="option"
    aria-selected={app.selectedCardId === c.id}
    tabindex={app.selectedCardId === c.id ? 0 : -1}
    onclick={() => void app.openCard(c.id)}
    oncontextmenu={(e) => {
      e.preventDefault();
      app.cardMenu = { cardId: c.id, anchor: e.currentTarget as HTMLElement };
    }}
    onkeydown={(e) => {
      if (e.key === "Enter" && e.target === e.currentTarget) {
        e.preventDefault();
        void app.openCard(c.id);
      }
    }}
  >
    <AgentMark agent={app.agent(c.agentId)} size={20} />
    <div class="main">
      {#if c.pendingApproval}<div class="appr"><ApprovalBar card={c} compact /></div>{/if}
      <div class="title">
        {#if c.unread}<span class="unread"></span>{/if}
        {c.title || "Untitled"}
      </div>
      <div class="meta">
        {#if p}<span class="proj"><span class="pdot" style="--c:{colorOr(p.color, '#0a84ff')}"></span>{p.name}</span>{/if}
        {#if c.branch}<span class="branch"><Icon name="branch" size={11} />{c.branch}</span>{/if}
        {#if c.status === "needs_you" && c.needsYouReason}<span class="why">{reasonLabel(c.needsYouReason, c.setup?.exitCode)}</span>{/if}
        {#if c.model}<span class="model" title="Model">{c.model}</span>{/if}
        {#if c.context}<ContextMeter context={c.context} />{/if}
        {#if c.hasSession && app.stats.get(c.id)}<StatsChip s={app.stats.get(c.id)!} />{/if}
        {#if c.restorable}<span class="why">restorable</span>{/if}
      </div>
    </div>
    <div class="timer s-{c.status}">{timerLabel(c, clock.now)}</div>
  </div>
{/snippet}

<style>
  .tracker {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 16px 24px;
    max-width: 920px;
    width: 100%;
    margin: 0 auto;
  }
  .summary {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 8px;
    margin: 4px 0 14px;
  }
  .stat {
    background: var(--bg-column);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 8px 12px;
    color: var(--text-2);
    font-size: 12px;
  }
  .stat b {
    display: block;
    font-size: 20px;
    font-weight: 600;
    color: var(--c);
    font-variant-numeric: tabular-nums;
  }
  .stat.zero b {
    color: var(--text-3);
  }
  .needs {
    --c: var(--s-needs);
  }
  .fin {
    --c: var(--s-done);
  }
  .run {
    --c: var(--s-running);
  }
  .idle {
    --c: var(--s-idle);
  }
  .res {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-2);
    margin: -4px 2px 12px;
    font-variant-numeric: tabular-nums;
  }
  .res b {
    color: var(--text);
    font-weight: 600;
  }
  .res .d {
    color: var(--text-3);
  }
  .empty {
    color: var(--text-3);
    text-align: center;
    padding: 40px 0;
  }
  .group {
    margin-bottom: 12px;
  }
  .ghead {
    display: flex;
    align-items: center;
    gap: 7px;
    border: 0;
    background: transparent;
    padding: 4px 4px;
    font-weight: 600;
    font-size: 12px;
    color: var(--text-2);
    width: 100%;
    text-align: left;
  }
  .chev {
    display: inline-grid;
    transition: transform 0.12s;
    color: var(--text-3);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .gdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--gc);
  }
  .gcount {
    color: var(--text-3);
    font-weight: 400;
  }
  .g-needs_you {
    --gc: var(--s-needs);
  }
  .g-finished {
    --gc: var(--s-done);
  }
  .g-running {
    --gc: var(--s-running);
  }
  .g-idle {
    --gc: var(--s-idle);
  }
  .g-exited {
    --gc: var(--s-exited);
  }
  .g-draft {
    --gc: transparent;
  }
  .g-draft .gdot {
    box-shadow: inset 0 0 0 1.5px var(--s-draft);
  }
  .rows {
    display: flex;
    flex-direction: column;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    min-height: 48px;
    border-top: 1px solid var(--border);
    outline: none;
  }
  .row:first-child {
    border-top: 0;
  }
  .row:hover {
    background: var(--bg-card-hover);
  }
  .row.selected {
    background: color-mix(in srgb, var(--accent) 12%, var(--bg-card));
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .main {
    flex: 1;
    min-width: 0;
  }
  .title {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .unread {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
  }
  .meta {
    display: flex;
    gap: 10px;
    font-size: 11.5px;
    color: var(--text-3);
    margin-top: 1px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
  .proj,
  .branch {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .branch {
    font-family: var(--mono);
    font-size: 10.5px;
  }
  .pdot {
    width: 7px;
    height: 7px;
    border-radius: 2px;
    background: var(--c);
    flex: none;
  }
  .why {
    color: var(--s-needs);
  }
  .model {
    font-size: 10.5px;
    padding: 0 5px;
    border-radius: 4px;
    background: var(--bg-hover);
    color: var(--text-2);
    white-space: nowrap;
  }
  .appr {
    order: 3;
    margin-top: 6px;
  }
  .main {
    display: flex;
    flex-direction: column;
  }
  .timer {
    flex: none;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
    white-space: nowrap;
  }
  .timer.s-needs_you {
    color: var(--s-needs);
    font-weight: 500;
  }
  @media (max-width: 560px) {
    .tracker {
      padding: 4px 10px 24px;
    }
    .summary {
      grid-template-columns: repeat(2, 1fr);
    }
    .row {
      padding: 8px 10px;
    }
  }
</style>
