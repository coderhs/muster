<script lang="ts">
  import type { Agent } from "../protocol/types";
  let { agent, size = 18 }: { agent: Agent; size?: number } = $props();

  const mono = $derived(
    agent.id === "shell" ? ">_" : (agent.label || agent.id).replace(/[^A-Za-z0-9]/g, "").slice(0, 1).toUpperCase() || "?",
  );
</script>

<span
  class="mark"
  style="--c:{agent.color || '#8e8e93'}; width:{size}px; height:{size}px; font-size:{Math.round(size * 0.56)}px"
  title={agent.label}
  aria-label={agent.label}
  class:shell={agent.id === "shell"}>{mono}</span
>

<style>
  .mark {
    flex: none;
    display: inline-grid;
    place-items: center;
    border-radius: 5px;
    background: color-mix(in srgb, var(--c) 22%, transparent);
    color: var(--c);
    font-weight: 700;
    letter-spacing: -0.02em;
    box-shadow: inset 0 0 0 0.5px color-mix(in srgb, var(--c) 50%, transparent);
  }
  .shell {
    font-family: var(--mono);
    font-weight: 600;
    letter-spacing: -0.08em;
  }
</style>
