<script lang="ts">
  import type { ContextInfo } from "../protocol/types";
  import { contextPercent, level, tokens } from "../agent/agent";

  let { context, wide = false }: { context: ContextInfo; wide?: boolean } = $props();
  const pct = $derived(contextPercent(context));
  const lvl = $derived(pct == null ? "ok" : level(pct, 75, 90));
  const title = $derived(
    `Context: ${tokens(context.usedTokens)}${context.windowTokens ? ` of ${tokens(context.windowTokens)}` : ""} tokens` +
      (pct != null ? ` (${Math.round(pct)}%)` : ""),
  );
</script>

<span class="ctx l-{lvl}" class:wide {title}>
  {#if pct != null}
    <span class="bar"><span class="fill" style="width:{pct}%"></span></span>
    <span class="num">{Math.round(pct)}%</span>
  {:else}
    <span class="num">{tokens(context.usedTokens)}</span>
  {/if}
</span>

<style>
  .ctx {
    --c: var(--text-3);
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10.5px;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }
  .bar {
    width: 28px;
    height: 4px;
    border-radius: 2px;
    background: var(--bg-active);
    overflow: hidden;
  }
  .wide .bar {
    width: 80px;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--c);
    border-radius: 2px;
  }
  .l-warn {
    --c: var(--s-needs);
    color: var(--s-needs);
  }
  .l-crit {
    --c: var(--danger);
    color: var(--danger);
  }
</style>
