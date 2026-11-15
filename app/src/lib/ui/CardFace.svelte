<script lang="ts">
  import { isSleeping } from "./idle";
  import type { Agent, Card } from "../protocol/types";
  import AgentMark from "./AgentMark.svelte";
  import StatusBadge from "./StatusBadge.svelte";
  import Icon from "./Icon.svelte";
  import { clock } from "../state/clock.svelte";
  import { dueLabel } from "./time";
  import ApprovalBar from "./ApprovalBar.svelte";
  import ContextMeter from "./ContextMeter.svelte";
  import StatsChip from "./StatsChip.svelte";
  import { useApp } from "../state/context";
  import { caps, reasonLabel } from "../agent/agent";

  let {
    card,
    agent,
    selected = false,
    dragging = false,
    onopen,
    onstart,
    onselect,
  }: {
    card: Card;
    agent: Agent;
    selected?: boolean;
    dragging?: boolean;
    onopen: () => void;
    onstart: () => void;
    onselect: () => void;
  } = $props();

  const app = useApp();
  const stats = $derived(card.hasSession ? app.stats.get(card.id) : undefined);
  const links = $derived(app.linkedTo(card.id).length);
  const queued = $derived(card.queuedMessages?.length ?? 0);
  const overdue = $derived(card.dueAt != null && card.dueAt < clock.now && card.status !== "done");
  const PRIORITY_LABEL = { low: "Low", medium: "Medium", high: "High", urgent: "Urgent" } as const;
</script>

<div
  class="card"
  class:selected
  class:dragging
  class:unread={card.unread}
  data-card-id={card.id}
  role="option"
  aria-selected={selected}
  tabindex={selected ? 0 : -1}
  onclick={onselect}
  ondblclick={onopen}
  oncontextmenu={(e) => {
    e.preventDefault();
    app.cardMenu = { cardId: card.id, anchor: e.currentTarget as HTMLElement };
  }}
  onkeydown={(e) => {
    if (e.key === "Enter" && e.target === e.currentTarget) {
      e.preventDefault();
      onopen();
    }
  }}
>
  <div class="top">
    <AgentMark {agent} />
    <div class="title">{card.title || "Untitled"}</div>
    {#if card.unread}<span class="unread-dot" title="Unread"></span>{/if}
  </div>
  {#if card.status === "needs_you" && card.needsYouReason && !card.pendingApproval}
    <div class="reason">{reasonLabel(card.needsYouReason, card.setup?.exitCode)}</div>
  {/if}
  {#if card.pendingApproval}
    <ApprovalBar {card} compact />
  {/if}
  <div class="meta">
    <StatusBadge status={card.status} since={card.statusSince} settingUp={card.setup?.state === "running"} sleeping={isSleeping(card)} source={caps(agent).statusSource} />
    {#if card.model}<span class="chip model" title="Model: {card.model}">{card.model}</span>{/if}
    {#if card.context && caps(agent).context}<ContextMeter context={card.context} />{/if}
    {#if card.priority}
      <span class="prio p-{card.priority}" title="{PRIORITY_LABEL[card.priority]} priority">
        <span class="bars"><i></i><i></i><i></i></span>
        {#if card.priority === "urgent" || card.priority === "high"}{PRIORITY_LABEL[card.priority]}{/if}
      </span>
    {/if}
    {#if card.dueAt}
      <span class="chip" class:overdue title="Due {new Date(card.dueAt).toLocaleString()}">
        <Icon name="calendar" size={11} />{dueLabel(card.dueAt, clock.now)}
      </span>
    {/if}
  </div>
  {#if card.branch || card.status === "draft" || card.restorable || links || queued || stats}
    <div class="foot">
      {#if card.branch}
        <span class="chip branch" title={card.cwd}><Icon name="branch" size={11} />{card.branch}</span>
      {/if}
      {#if links}<span class="chip" title="Linked to {links} card{links === 1 ? '' : 's'}"><Icon name="link" size={11} />{links}</span>{/if}
      {#if queued}<span class="chip queued" title="{queued} queued message{queued === 1 ? '' : 's'}"><Icon name="mail" size={11} />{queued}</span>{/if}
      {#if stats}<StatsChip s={stats} />{/if}
      {#if card.restorable}
        <button
          class="start"
          onclick={(e) => {
            e.stopPropagation();
            onstart();
          }}
          onpointerdown={(e) => e.stopPropagation()}
          title="Restore the session that was interrupted (scrollback and agent conversation)"
        >
          <Icon name="restore" size={10} />Restore
        </button>
      {:else if card.status === "draft"}
        <button
          class="start"
          onclick={(e) => {
            e.stopPropagation();
            onstart();
          }}
          onpointerdown={(e) => e.stopPropagation()}
          title="Start agent"
        >
          <Icon name="play" size={10} />Start
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .card {
    position: relative;
    background: var(--bg-card);
    border-radius: var(--radius);
    padding: 8px 10px 8px;
    box-shadow: var(--shadow);
    border: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 6px;
    outline: none;
    touch-action: manipulation;
  }
  .card:hover {
    background: var(--bg-card-hover);
  }
  .card.selected {
    border-color: var(--accent);
    box-shadow:
      0 0 0 1px var(--accent),
      var(--shadow);
  }
  .card.dragging {
    opacity: 0.35;
  }
  .top {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  .title {
    flex: 1;
    min-width: 0;
    font-weight: 500;
    line-height: 18px;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    word-break: break-word;
  }
  .reason {
    font-size: 11.5px;
    color: var(--s-needs);
    margin: -2px 0 0 26px;
  }
  .unread-dot {
    flex: none;
    width: 8px;
    height: 8px;
    margin-top: 5px;
    border-radius: 50%;
    background: var(--accent);
  }
  .meta,
  .foot {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .chip.model {
    max-width: 110px;
    white-space: nowrap;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 18px;
    padding: 0 6px;
    border-radius: 4px;
    font-size: 11px;
    color: var(--text-2);
    background: var(--bg-hover);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .branch {
    font-family: var(--mono);
    font-size: 10.5px;
  }
  .chip.overdue {
    color: var(--danger);
  }
  .prio {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    font-weight: 500;
    color: var(--c);
  }
  .bars {
    display: inline-flex;
    align-items: flex-end;
    gap: 1.5px;
    height: 10px;
  }
  .bars i {
    width: 2.5px;
    border-radius: 1px;
    background: var(--c);
    opacity: 0.3;
  }
  .bars i:nth-child(1) {
    height: 4px;
  }
  .bars i:nth-child(2) {
    height: 7px;
  }
  .bars i:nth-child(3) {
    height: 10px;
  }
  .p-low {
    --c: var(--p-low);
  }
  .p-low i:nth-child(1),
  .p-medium i:nth-child(-n + 2),
  .p-high i,
  .p-urgent i {
    opacity: 1;
  }
  .p-medium {
    --c: var(--p-medium);
  }
  .p-high {
    --c: var(--p-high);
  }
  .p-urgent {
    --c: var(--p-urgent);
  }
  .queued {
    color: var(--accent);
  }
  .start {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 8px;
    border-radius: 5px;
    border: 0;
    font-size: 11px;
    font-weight: 600;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .start:hover {
    background: var(--accent);
    color: var(--accent-text);
  }
</style>
