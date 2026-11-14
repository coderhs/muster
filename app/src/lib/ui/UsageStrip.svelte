<script lang="ts">
  import type { UsageWindow } from "../protocol/types";
  import { useApp } from "../state/context";
  import { clock } from "../state/clock.svelte";
  import { level, resetLabel } from "../agent/agent";
  import { shortDuration } from "./time";

  const app = useApp();
  const u = $derived(app.usage);
  const agentLabel = $derived(u?.agentId ? app.agent(u.agentId).label : null);
  // Named 5-hour / weekly windows when the sizes match; otherwise every window the agent reported.
  const rows = $derived.by(() => {
    const named = [
      { label: "5-hour", w: u?.fiveHour },
      { label: "Weekly", w: u?.sevenDay },
    ].filter((r): r is { label: string; w: UsageWindow } => !!r.w);
    if (named.length) return named;
    return (u?.windows ?? []).map((w) => ({
      label: w.windowMinutes ? (w.windowMinutes % 1440 === 0 ? `${w.windowMinutes / 1440}-day` : w.windowMinutes % 60 === 0 ? `${w.windowMinutes / 60}-hour` : `${w.windowMinutes}-min`) : w.name,
      w,
    }));
  });
</script>

{#if u && rows.length}
  <div class="usage" title="Account usage, observed {shortDuration(clock.now - u.updatedAt)} ago">
    {#each rows as r (r.label)}
      {@const pct = Math.max(0, Math.min(100, r.w.usedPercent))}
      <div class="win l-{level(pct, 75, 90)}">
        <div class="top">
          <span class="lbl">{agentLabel ? `${agentLabel} · ` : ""}{r.label}</span>
          <span class="pct">{Math.round(pct)}% used</span>
          {#if resetLabel(r.w.resetsAt, clock.now)}<span class="reset">{resetLabel(r.w.resetsAt, clock.now)}</span>{/if}
        </div>
        <div class="bar"><span class="fill" style="width:{pct}%"></span></div>
      </div>
    {/each}
  </div>
{/if}

<style>
  .usage {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    margin: 4px 0 10px;
  }
  .win {
    --c: var(--s-done);
    background: var(--bg-column);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 7px 12px 9px;
  }
  .top {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 12px;
  }
  .lbl {
    color: var(--text-2);
  }
  .pct {
    font-weight: 600;
    color: var(--c);
    font-variant-numeric: tabular-nums;
  }
  .reset {
    margin-left: auto;
    color: var(--text-3);
    font-size: 11px;
  }
  .bar {
    margin-top: 5px;
    height: 5px;
    border-radius: 3px;
    background: var(--bg-active);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--c);
    border-radius: 3px;
    transition: width 0.3s;
  }
  .l-warn {
    --c: var(--s-needs);
  }
  .l-crit {
    --c: var(--danger);
  }
  @media (max-width: 560px) {
    .usage {
      grid-template-columns: 1fr;
    }
  }
</style>
